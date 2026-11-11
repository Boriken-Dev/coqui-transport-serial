//! Two real serial ports cabled to each other (a null-modem cable, or two USB adapters wired
//! together), named by `COQUI_TEST_SERIAL_A` and `COQUI_TEST_SERIAL_B`. Ignored, so a run without
//! the hardware reports them as ignored and never as passed; run with `--ignored` where it exists.

use std::time::Duration;

use coqui_transport::{Channel, StreamChannel};
use coqui_transport_serial::{Cobs, Serial, SerialBuilder, SerialSettings};

const WAIT: Option<Duration> = Some(Duration::from_secs(30));

fn port(variable: &str, baud: u32) -> Serial {
    let name = std::env::var(variable).unwrap_or_else(|_| panic!("{variable} names a port"));
    let settings = {
        let mut settings = SerialSettings::new();
        settings.baud_rate = baud;
        settings
    };
    SerialBuilder::new()
        .path(name)
        .fill(coqui_builder::Partial::from(settings))
        .build()
        .unwrap()
}

#[test]
#[ignore = "needs two cabled serial ports: COQUI_TEST_SERIAL_A and COQUI_TEST_SERIAL_B"]
fn frames_cross_a_real_cable_both_ways() {
    for baud in [9_600, 115_200] {
        let a = StreamChannel::start(port("COQUI_TEST_SERIAL_A", baud), Cobs::new()).unwrap();
        let b = StreamChannel::start(port("COQUI_TEST_SERIAL_B", baud), Cobs::new()).unwrap();
        let large: Vec<u8> = (0..20_000).map(|n| (n % 251) as u8).collect();
        for message in [b"ping".to_vec(), Vec::new(), (0..=255).collect(), large] {
            a.send(&message).unwrap();
            assert_eq!(b.receive(WAIT).unwrap().as_ref(), Some(&message));
            b.send(&message).unwrap();
            assert_eq!(a.receive(WAIT).unwrap().as_ref(), Some(&message));
        }
    }
}
