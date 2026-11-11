use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use coqui_builder::Builder;
use coqui_transport::{Connector as ConnectorTrait, Halves, Interrupt, Split};

use crate::settings::{DataBits, FlowControl, Parity, SerialSettings, StopBits, Transceiver};

/// An open serial port: a byte stream that never ends.
///
/// [`Split`] it into a reading half and a writing half for a
/// [`StreamChannel`](coqui_transport::StreamChannel) with the [`Cobs`](crate::Cobs) codec, which
/// detects the damage a serial line does.
///
/// **A serial line has no end of stream.** Dropping the writing half sends nothing, so the peer's
/// channel does not see a close: a protocol that needs one says so in a message. Closing the
/// channel here ends this side after the channel's `linger` of silence.
///
/// **It is a wire.** Bytes can be lost, repeated or damaged (the framing detects it and drops the
/// frame), the peer may be absent, and nothing says so until a read times out. Flow control `none`
/// lets a sender that outruns its receiver lose bytes. A two-wire RS-485 bus is half-duplex: see
/// the crate's documentation before putting a channel on one.
///
/// Handles on the same port share it: [`Serial::lines`] outlives the split.
#[derive(Debug)]
pub struct Serial {
    port: Arc<serial2::SerialPort>,
    stop: Arc<AtomicBool>,
}

/// What a [`SerialBuilder`] holds: the settings and the port's name. Build with
/// `SerialBuilder::new()`, set the settings through its setters (or `fill`), give `path`, then
/// `build`, which opens the port. The builder's `non_defaults()` lists the settings that differ
/// from their defaults.
///
/// `path` is a device path on Unix (`/dev/ttyUSB0`) and a COM port on Windows (`COM3`, or a
/// `\\.\COM10` path, which ports above 9 need and which is added to a plain `COM10`).
#[derive(Builder, Debug)]
#[builder(build = Serial, fallible, entry = "new")]
pub struct SerialParts {
    settings: SerialSettings,
    #[builder(input)]
    path: PathBuf,
}

impl TryFrom<SerialParts> for Serial {
    type Error = io::Error;

    fn try_from(parts: SerialParts) -> io::Result<Self> {
        Self::open_with(&parts.path, parts.settings)
    }
}

const fn char_size(bits: DataBits) -> serial2::CharSize {
    match bits {
        DataBits::Five => serial2::CharSize::Bits5,
        DataBits::Six => serial2::CharSize::Bits6,
        DataBits::Seven => serial2::CharSize::Bits7,
        DataBits::Eight => serial2::CharSize::Bits8,
    }
}

impl Serial {
    /// Opens the port at `path` with the default settings: 115200 baud, 8 data bits, no parity, one
    /// stop bit, no flow control.
    ///
    /// # Errors
    ///
    /// The port could not be opened or configured.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_with(path.as_ref(), SerialSettings::new())
    }

    fn open_with(path: &Path, settings: SerialSettings) -> io::Result<Self> {
        let mut port = serial2::SerialPort::open(path, |mut port: serial2::Settings| {
            port.set_raw();
            port.set_baud_rate(settings.baud_rate)?;
            port.set_char_size(char_size(settings.data_bits));
            port.set_parity(match settings.parity {
                Parity::Odd => serial2::Parity::Odd,
                Parity::Even => serial2::Parity::Even,
                _ => serial2::Parity::None,
            });
            port.set_stop_bits(match settings.stop_bits {
                StopBits::Two => serial2::StopBits::Two,
                _ => serial2::StopBits::One,
            });
            port.set_flow_control(match settings.flow_control {
                FlowControl::Software => serial2::FlowControl::XonXoff,
                FlowControl::Hardware => serial2::FlowControl::RtsCts,
                _ => serial2::FlowControl::None,
            });
            Ok(port)
        })?;
        Self::prepare(&mut port, settings)?;
        Ok(Self::around(port))
    }

    fn around(port: serial2::SerialPort) -> Self {
        Self {
            port: Arc::new(port),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Timeouts, the modem lines and the transceiver, on a port just opened.
    fn prepare(port: &mut serial2::SerialPort, settings: SerialSettings) -> io::Result<()> {
        port.set_read_timeout(settings.wake.max(Duration::from_millis(1)))?;
        port.set_write_timeout(settings.write_timeout.max(Duration::from_millis(1)))?;
        transceiver(port, settings.transceiver)?;
        if let Some(dtr) = settings.dtr {
            port.set_dtr(dtr)?;
        }
        if let Some(rts) = settings.rts {
            port.set_rts(rts)?;
        }
        if settings.discard_input {
            port.discard_input_buffer()?;
        }
        Ok(())
    }

    /// Two pseudo-terminals joined as the two ends of a null-modem cable, for tests and for two
    /// parts of one program. Each end is an ordinary [`Serial`] with the default settings, except
    /// that the baud rate and the framing of characters mean nothing to a pseudo-terminal.
    ///
    /// # Errors
    ///
    /// The pseudo-terminals could not be opened.
    #[cfg(unix)]
    pub fn pair() -> io::Result<(Self, Self)> {
        let (mut a, mut b) = serial2::SerialPort::pair()?;
        let settings = SerialSettings::new();
        Self::prepare(&mut a, settings)?;
        Self::prepare(&mut b, settings)?;
        Ok((Self::around(a), Self::around(b)))
    }

    /// The modem control lines of this port, which stay usable after it is split.
    #[must_use]
    pub fn lines(&self) -> Lines {
        Lines {
            port: Arc::clone(&self.port),
        }
    }
}

/// Applies the transceiver mode, where the platform has one.
#[cfg_attr(
    not(all(feature = "rs485", target_os = "linux")),
    allow(clippy::unnecessary_wraps)
)]
fn transceiver(port: &mut serial2::SerialPort, mode: Transceiver) -> io::Result<()> {
    if mode == Transceiver::Default {
        return Ok(());
    }
    #[cfg(all(feature = "rs485", target_os = "linux"))]
    {
        use serial2::rs4xx::{Rs485Config, TransceiverMode};
        let mode = match mode {
            Transceiver::Rs422 => TransceiverMode::Rs422,
            Transceiver::Rs485 => TransceiverMode::Rs485(Rs485Config::new()),
            _ => {
                let mut config = Rs485Config::new();
                config.set_full_duplex(true);
                TransceiverMode::Rs485(config)
            }
        };
        port.set_rs4xx_mode(mode)
    }
    #[cfg(not(all(feature = "rs485", target_os = "linux")))]
    {
        let _ = port;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a transceiver mode needs the `rs485` feature and Linux",
        ))
    }
}

/// The modem control lines of a [`Serial`]: outputs to set, inputs to read. Clones share the port.
#[derive(Clone, Debug)]
pub struct Lines {
    port: Arc<serial2::SerialPort>,
}

impl Lines {
    /// Raises or lowers DTR (data terminal ready).
    ///
    /// # Errors
    ///
    /// The driver refused, or the port has no such line.
    pub fn set_dtr(&self, on: bool) -> io::Result<()> {
        self.port.set_dtr(on)
    }

    /// Raises or lowers RTS (request to send).
    ///
    /// # Errors
    ///
    /// The driver refused, or the port has no such line.
    pub fn set_rts(&self, on: bool) -> io::Result<()> {
        self.port.set_rts(on)
    }

    /// CTS (clear to send), driven by the peer.
    ///
    /// # Errors
    ///
    /// The driver could not say.
    pub fn read_cts(&self) -> io::Result<bool> {
        self.port.read_cts()
    }

    /// DSR (data set ready), driven by the peer.
    ///
    /// # Errors
    ///
    /// The driver could not say.
    pub fn read_dsr(&self) -> io::Result<bool> {
        self.port.read_dsr()
    }

    /// RI (ring indicator).
    ///
    /// # Errors
    ///
    /// The driver could not say.
    pub fn read_ri(&self) -> io::Result<bool> {
        self.port.read_ri()
    }

    /// CD (carrier detect).
    ///
    /// # Errors
    ///
    /// The driver could not say.
    pub fn read_cd(&self) -> io::Result<bool> {
        self.port.read_cd()
    }
}

/// The reading half of a split [`Serial`]. A read that finds nothing for the `wake` setting returns
/// [`io::ErrorKind::TimedOut`], which a channel retries; once the channel is closed it returns
/// [`io::ErrorKind::ConnectionAborted`] instead.
#[derive(Debug)]
pub struct ReadHalf {
    port: Arc<serial2::SerialPort>,
    stop: Arc<AtomicBool>,
}

/// The writing half of a split [`Serial`]. A write the port takes none of for the `write_timeout`
/// setting returns [`io::ErrorKind::TimedOut`], which a channel retries; once the channel is closed
/// it returns [`io::ErrorKind::BrokenPipe`]. Dropping it ends nothing: a serial line has no end.
#[derive(Debug)]
pub struct WriteHalf {
    port: Arc<serial2::SerialPort>,
    stop: Arc<AtomicBool>,
}

impl Read for ReadHalf {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.stop.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::ConnectionAborted.into());
        }
        match self.port.read(buf) {
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                if self.stop.load(Ordering::SeqCst) {
                    Err(io::ErrorKind::ConnectionAborted.into())
                } else {
                    Err(error)
                }
            }
            other => other,
        }
    }
}

impl Write for WriteHalf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.stop.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        match self.port.write(buf) {
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                if self.stop.load(Ordering::SeqCst) {
                    Err(io::ErrorKind::BrokenPipe.into())
                } else {
                    Err(error)
                }
            }
            other => other,
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.port.flush()
    }
}

impl Split for Serial {
    type Reader = ReadHalf;
    type Writer = WriteHalf;

    /// A reading half and a writing half on the same port. The interrupt makes the next read or
    /// write that returns a timeout fail, so a channel's threads end within the `wake` or
    /// `write_timeout` setting of a close; the port closes when the last half is gone.
    fn split(self) -> io::Result<Halves<ReadHalf, WriteHalf>> {
        let stop = self.stop;
        let ended = Arc::clone(&stop);
        Ok(Halves::new(
            ReadHalf {
                port: Arc::clone(&self.port),
                stop: Arc::clone(&stop),
            },
            WriteHalf {
                port: self.port,
                stop,
            },
        )
        .interrupt(Interrupt::new(move || ended.store(true, Ordering::SeqCst))))
    }
}

/// Opens a port each time it connects.
///
/// Opening can wait on the driver (a USB adapter that is going away, a port another program holds
/// on some systems), so `connect` opens on a thread and gives up waiting after `timeout`; a port
/// that opens late is closed again.
#[derive(Clone, Debug)]
#[must_use]
pub struct Connector {
    path: PathBuf,
    settings: SerialSettings,
}

impl Connector {
    /// A connector for the port at `path`, with the default settings.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            settings: SerialSettings::new(),
        }
    }

    /// Sets the settings every connection is opened with.
    pub const fn settings(mut self, settings: SerialSettings) -> Self {
        self.settings = settings;
        self
    }

    /// The port it opens.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ConnectorTrait for Connector {
    type Connection = Serial;
    type Error = io::Error;

    fn connect(&self, timeout: Duration) -> io::Result<Serial> {
        if timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a connect needs a timeout above zero",
            ));
        }
        let (sender, opened) = mpsc::channel();
        let path = self.path.clone();
        let settings = self.settings;
        thread::Builder::new()
            .name("coqui-transport-serial open".into())
            .spawn(move || {
                // A port nobody waits for any more is dropped here, which closes it.
                let _ = sender.send(Serial::open_with(&path, settings));
            })?;
        opened
            .recv_timeout(timeout)
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the port did not open in time"))?
    }
}
