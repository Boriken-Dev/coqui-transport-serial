//! A pseudo-terminal is a serial port the kernel provides: these tests open it as one, by pair and
//! by the slave's path, and run channels over it. They run where pseudo-terminals exist (Unix).
#![cfg(unix)]

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use coqui_transport::{Channel, Connector as _, Duplex, Split, StreamChannel};
use coqui_transport_serial::{
    Cobs, Connector, FlowControl, Serial, SerialBuilder, SerialSettings, StopBits,
};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

const WAIT: Option<Duration> = Some(Duration::from_secs(20));

/// A pseudo-terminal master and the path of its slave.
fn terminal() -> (OwnedFd, PathBuf) {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).expect("a master");
    grantpt(&master).expect("grantpt");
    unlockpt(&master).expect("unlockpt");
    let slave = ptsname(&master, Vec::new()).expect("the slave's name");
    (
        master,
        PathBuf::from(slave.to_str().expect("a path").to_owned()),
    )
}

#[test]
fn the_two_ends_of_a_pair_talk_at_once_in_both_directions() {
    let (a, b) = Serial::pair().unwrap();
    let a = StreamChannel::start(a, Cobs::new()).unwrap();
    let b = StreamChannel::start(b, Cobs::new()).unwrap();
    let large: Vec<u8> = (0..300 * 1024).map(|n| (n % 253) as u8).collect();
    let messages: Vec<Vec<u8>> = vec![
        b"hello".to_vec(),
        Vec::new(),
        (0..=255).collect(),
        vec![0; 1000],
        large,
    ];
    thread::scope(|scope| {
        scope.spawn(|| {
            for message in &messages {
                a.send(message).unwrap();
            }
            for message in &messages {
                assert_eq!(b.receive(WAIT).unwrap().as_ref(), Some(message));
            }
        });
        for message in &messages {
            b.send(message).unwrap();
        }
        for message in &messages {
            assert_eq!(a.receive(WAIT).unwrap().as_ref(), Some(message));
        }
    });
}

#[test]
fn closing_a_channel_returns_and_ends_its_threads_within_the_wake() {
    let (a, _b) = Serial::pair().unwrap();
    let a = StreamChannel::start(a, Cobs::new()).unwrap();
    a.send(b"x").unwrap();
    let start = Instant::now();
    a.close();
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_port_opened_by_the_path_of_a_slave_carries_frames_and_takes_its_settings() {
    let (master, path) = terminal();
    let settings = {
        let mut settings = SerialSettings::new();
        settings.baud_rate = 19_200;
        settings.stop_bits = StopBits::Two;
        settings.flow_control = FlowControl::Hardware;
        settings
    };
    let serial = SerialBuilder::new()
        .path(path.clone())
        .fill(coqui_builder::Partial::from(settings))
        .build()
        .unwrap();
    // The slave's settings are the pair's: read them from the master. The kernel's pseudo-terminal
    // driver forces 8 data bits and no parity, so those two cannot be read back here.
    let termios = rustix::termios::tcgetattr(&master).unwrap();
    use rustix::termios::ControlModes;
    let modes = termios.control_modes;
    assert!(modes.contains(ControlModes::CS8));
    assert!(modes.contains(ControlModes::CSTOPB));
    assert!(modes.contains(ControlModes::CRTSCTS));
    assert_eq!(termios.output_speed(), 19_200);

    drop(serial);
    let serial = Serial::open(&path).unwrap();
    let file = File::from(master);
    let master_side = Duplex::new(file.try_clone().unwrap(), file);
    let near = StreamChannel::start(serial, Cobs::new()).unwrap();
    let far = StreamChannel::start(master_side, Cobs::new()).unwrap();
    near.send(&(0..=255).collect::<Vec<u8>>()).unwrap();
    far.send(b"from the master").unwrap();
    assert_eq!(
        far.receive(WAIT).unwrap(),
        Some((0..=255).collect::<Vec<u8>>())
    );
    assert_eq!(
        near.receive(WAIT).unwrap().as_deref(),
        Some(b"from the master".as_slice())
    );
    near.close();
}

#[test]
fn input_received_before_the_open_is_discarded_unless_kept() {
    for (discard, expected) in [(true, 0), (false, 7)] {
        let (master, path) = terminal();
        // Something holds the slave open, so the master's bytes wait in its input queue.
        let _holder = File::options().read(true).write(true).open(&path).unwrap();
        let mut master = File::from(master);
        master.write_all(b"garbage").unwrap();
        thread::sleep(Duration::from_millis(100));
        let settings = {
            let mut settings = SerialSettings::new();
            settings.discard_input = discard;
            settings.wake = Duration::from_millis(50);
            settings
        };
        let serial = SerialBuilder::new()
            .path(path.clone())
            .fill(coqui_builder::Partial::from(settings))
            .build()
            .unwrap();
        let halves = serial.split().unwrap();
        let mut reader = halves.reader;
        let mut got = Vec::new();
        let mut buffer = [0_u8; 32];
        let until = Instant::now() + Duration::from_millis(400);
        while Instant::now() < until && got.len() < 7 {
            match reader.read(&mut buffer) {
                Ok(read) => got.extend_from_slice(&buffer[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => panic!("{error}"),
            }
        }
        assert_eq!(got.len(), expected, "discard {discard}: {got:?}");
    }
}

#[test]
fn the_interrupt_wakes_a_blocked_read_and_stops_a_write() {
    let (master, path) = terminal();
    let _master = File::from(master);
    let settings = {
        let mut settings = SerialSettings::new();
        settings.wake = Duration::from_millis(50);
        settings
    };
    let serial = SerialBuilder::new()
        .path(path.clone())
        .fill(coqui_builder::Partial::from(settings))
        .build()
        .unwrap();
    let halves = serial.split().unwrap();
    let (mut reader, mut writer) = (halves.reader, halves.writer);
    let interrupt = halves.interrupt.expect("a serial port has an interrupt");
    let mut buffer = [0_u8; 8];
    let error = reader.read(&mut buffer).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    let start = Instant::now();
    thread::scope(|scope| {
        let waiting = scope.spawn(|| {
            loop {
                match reader.read(&mut buffer) {
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                    other => break other.map_err(|e| e.kind()),
                }
            }
        });
        thread::sleep(Duration::from_millis(100));
        interrupt.interrupt();
        assert_eq!(
            waiting.join().unwrap(),
            Err(std::io::ErrorKind::ConnectionAborted)
        );
    });
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(
        writer.write(b"x").unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
}

#[test]
fn the_connector_opens_a_port_and_refuses_what_is_not_there() {
    let (_master, path) = terminal();
    let connector = Connector::new(&path).settings(SerialSettings::new());
    assert_eq!(connector.path(), path);
    let serial = connector.connect(Duration::from_secs(10)).unwrap();
    drop(serial);
    assert_eq!(
        connector.connect(Duration::ZERO).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    let missing = Connector::new("/dev/coqui-transport-no-such-port");
    assert_eq!(
        missing.connect(Duration::from_secs(10)).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
}

#[test]
fn a_transceiver_mode_is_refused_where_nothing_applies_it() {
    let (a, _b) = Serial::pair().unwrap();
    drop(a);
    let (_master, path) = terminal();
    let settings = {
        let mut settings = SerialSettings::new();
        settings.transceiver = coqui_transport_serial::Transceiver::Rs485;
        settings
    };
    let error = SerialBuilder::new()
        .path(path.clone())
        .fill(coqui_builder::Partial::from(settings))
        .build()
        .map(|_| ())
        .unwrap_err();
    // Without the feature, or on a pseudo-terminal that has no RS-485 ioctl, the open fails.
    println!("transceiver on a pseudo-terminal: {error:?}");
}
