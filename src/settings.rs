use core::time::Duration;

use coqui_builder::{Describe, Leaf, NonDefault};

use crate::SerialBuilder;

/// Data bits per character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Leaf)]
#[non_exhaustive]
pub enum DataBits {
    /// Five bits.
    #[map(rename = "5")]
    Five,
    /// Six bits.
    #[map(rename = "6")]
    Six,
    /// Seven bits.
    #[map(rename = "7")]
    Seven,
    /// Eight bits: every byte value, which a binary framing needs.
    #[map(rename = "8")]
    Eight,
}

/// The parity bit of each character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Leaf)]
#[map(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Parity {
    /// No parity bit.
    None,
    /// An odd count of one bits.
    Odd,
    /// An even count of one bits.
    Even,
}

/// Stop bits after each character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Leaf)]
#[non_exhaustive]
pub enum StopBits {
    /// One stop bit.
    #[map(rename = "1")]
    One,
    /// Two stop bits.
    #[map(rename = "2")]
    Two,
}

/// How the receiver holds the sender back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Leaf)]
#[map(rename_all = "lowercase")]
#[non_exhaustive]
pub enum FlowControl {
    /// None: a sender that outruns its receiver loses bytes, which the framing then detects.
    None,
    /// XON and XOFF characters in the data stream. They are bytes the framing would carry, so
    /// this is only for a link whose framing leaves `0x11` and `0x13` out: not this crate's.
    Software,
    /// The RTS and CTS lines.
    Hardware,
}

/// What the port's transceiver does, where the driver and the operating system let software say.
///
/// Anything but `default` needs the `rs485` feature and Linux, and is refused with
/// [`std::io::ErrorKind::Unsupported`] elsewhere. A driver may also ignore it without saying so.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Leaf)]
#[map(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Transceiver {
    /// The port as it is, which is RS-232 unless its hardware is wired otherwise.
    Default,
    /// RS-422: four wires, a receiver that is always on.
    Rs422,
    /// Two-wire RS-485: the kernel raises RTS around each transmission to turn the driver on, and
    /// the receiver is off while it transmits.
    Rs485,
    /// Four-wire RS-485: as `rs485`, with the receiver left on while transmitting.
    Rs485FullDuplex,
}

impl crate::Serial {
    /// The default baud rate: 115200.
    pub const DEFAULT_BAUD_RATE: u32 = 115_200;
    /// The default write timeout: 1 s.
    pub const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
    /// The default wake: 250 ms.
    pub const DEFAULT_WAKE: Duration = Duration::from_millis(250);
}

/// What a [`SerialBuilder`](crate::SerialBuilder) takes whole through `fill`, in the form a file, a
/// form or a peer fills.
///
/// Every key is optional (an absent one is its default) and an unknown key is refused; a duration
/// is a number with an optional unit (`1`, `250ms`). The type is `coqui_builder::Describe`: a
/// `Partial` fills it, and its setters are methods of [`SerialBuilder`](crate::SerialBuilder). It is
/// `#[non_exhaustive]`: start from [`Default`] and assign the fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Describe)]
#[map(deny_unknown_fields)]
#[builder(setters_on = SerialBuilder)]
#[non_exhaustive]
pub struct SerialSettings {
    /// Bits per second. Any rate the driver accepts, standard or not. Default
    /// [`Serial::DEFAULT_BAUD_RATE`](crate::Serial::DEFAULT_BAUD_RATE).
    #[map(default = SerialSettings::new().baud_rate)]
    pub baud_rate: u32,
    /// Data bits per character. Default `8`.
    #[map(default = SerialSettings::new().data_bits)]
    pub data_bits: DataBits,
    /// Parity. Default `none`.
    #[map(default = SerialSettings::new().parity)]
    pub parity: Parity,
    /// Stop bits. Default `1`.
    #[map(default = SerialSettings::new().stop_bits)]
    pub stop_bits: StopBits,
    /// Flow control. Default `none`.
    #[map(default = SerialSettings::new().flow_control)]
    pub flow_control: FlowControl,
    /// The transceiver mode; see [`Transceiver`]. Default `default`.
    #[map(default = SerialSettings::new().transceiver)]
    pub transceiver: Transceiver,
    /// How long one write call may go with the port taking none of it before it returns a timeout,
    /// which a channel retries until its own send limit. Default
    /// [`Serial::DEFAULT_WRITE_TIMEOUT`](crate::Serial::DEFAULT_WRITE_TIMEOUT).
    #[map(default = SerialSettings::new().write_timeout)]
    pub write_timeout: Duration,
    /// How often a blocked read returns, so that a channel being closed is seen: a close takes up
    /// to this long to wake the reading thread. Default
    /// [`Serial::DEFAULT_WAKE`](crate::Serial::DEFAULT_WAKE).
    #[map(default = SerialSettings::new().wake)]
    pub wake: Duration,
    /// Throw away what the driver received before the port was opened, so line noise or the end of
    /// an earlier session does not meet the first frame. Default on.
    #[map(default = SerialSettings::new().discard_input)]
    pub discard_input: bool,
    /// Set DTR at open: `true` raises it, `false` lowers it, unset leaves it as the driver has it.
    /// Many devices reset or enter a bootloader when DTR changes; opening a port may raise it
    /// whatever this says. Default unset.
    #[map(default = None, none = "unchanged")]
    pub dtr: Option<bool>,
    /// Set RTS at open, as `dtr`. With `hardware` flow control or an RS-485 transceiver the
    /// driver owns RTS. Default unset.
    #[map(default = None, none = "unchanged")]
    pub rts: Option<bool>,
}

impl SerialSettings {
    /// Every setting at its default.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            baud_rate: crate::Serial::DEFAULT_BAUD_RATE,
            data_bits: DataBits::Eight,
            parity: Parity::None,
            stop_bits: StopBits::One,
            flow_control: FlowControl::None,
            transceiver: Transceiver::Default,
            write_timeout: crate::Serial::DEFAULT_WRITE_TIMEOUT,
            wake: crate::Serial::DEFAULT_WAKE,
            discard_input: true,
            dtr: None,
            rts: None,
        }
    }

    /// Every setting that differs from its default, named by key path.
    #[must_use]
    pub fn non_defaults(&self) -> alloc::vec::Vec<NonDefault> {
        coqui_builder::non_defaults(self)
    }
}

impl Default for SerialSettings {
    fn default() -> Self {
        Self::new()
    }
}
