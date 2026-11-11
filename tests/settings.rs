//! The serial settings are values a file fills, each setter reaches them, and a port that is not there
//! is an error on every platform.

use std::time::Duration;

use coqui_builder::Partial;
use coqui_transport_serial::{
    DataBits, FlowControl, Parity, Serial, SerialBuilder, SerialSettings, StopBits, Transceiver,
};

fn from_toml(text: &str) -> Result<SerialSettings, String> {
    toml::from_str::<Partial<SerialSettings>>(text)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

#[test]
fn an_empty_file_is_every_default() {
    let settings = from_toml("").unwrap();
    assert_eq!(settings, SerialSettings::new());
    assert!(settings.non_defaults().is_empty());
    assert_eq!(settings.baud_rate, 115_200);
    assert_eq!(settings.data_bits, DataBits::Eight);
    assert!(settings.discard_input);
    assert_eq!(settings.wake, Duration::from_millis(250));
}

#[test]
fn a_file_sets_each_key_and_unknown_or_bad_keys_are_refused() {
    let settings = from_toml(
        "baud_rate = 9600\ndata_bits = \"7\"\nparity = \"even\"\nstop_bits = \"2\"\n\
         flow_control = \"hardware\"\ntransceiver = \"rs485-full-duplex\"\nwrite_timeout = \"2s\"\n\
         wake = \"100ms\"\ndiscard_input = false\ndtr = true\nrts = false",
    )
    .unwrap();
    assert_eq!(settings.baud_rate, 9600);
    assert_eq!(settings.data_bits, DataBits::Seven);
    assert_eq!(settings.parity, Parity::Even);
    assert_eq!(settings.stop_bits, StopBits::Two);
    assert_eq!(settings.flow_control, FlowControl::Hardware);
    assert_eq!(settings.transceiver, Transceiver::Rs485FullDuplex);
    assert_eq!(settings.write_timeout, Duration::from_secs(2));
    assert_eq!(settings.wake, Duration::from_millis(100));
    assert!(!settings.discard_input);
    assert_eq!((settings.dtr, settings.rts), (Some(true), Some(false)));
    assert_eq!(settings.non_defaults().len(), 11);
    assert!(from_toml("baud = 1").is_err());
    assert!(from_toml("parity = \"mark\"").is_err());
    assert!(from_toml("data_bits = \"9\"").is_err());
}

#[test]
fn the_setters_reach_the_settings_and_a_path_is_required() {
    let builder = SerialBuilder::new()
        .baud_rate(57_600)
        .parity(Parity::Odd)
        .discard_input(false);
    let keys: Vec<String> = builder
        .non_defaults()
        .into_iter()
        .map(|n| n.setting)
        .collect();
    assert_eq!(keys, ["baud_rate", "discard_input", "parity"]);
    let missing = builder.build().unwrap_err().to_string();
    assert!(missing.contains("path"), "{missing}");
}

#[test]
fn a_port_that_is_not_there_is_not_found() {
    let name = if cfg!(windows) {
        "COM250"
    } else {
        "/dev/coqui-transport-no-such-port"
    };
    let error = Serial::open(name).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{error:?}");
}
