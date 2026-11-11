#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::alloc_instead_of_core
)]

extern crate alloc;

mod cobs;
mod serial;
mod settings;

pub use cobs::{Cobs, CobsDecoder, Counters};
pub use coqui_builder::builder::BuildError;
pub use coqui_builder::{Missing, NonDefault};
pub use serial::{Connector, Lines, ReadHalf, Serial, SerialBuilder, SerialParts, WriteHalf};
pub use settings::{DataBits, FlowControl, Parity, SerialSettings, StopBits, Transceiver};

/// The setters trait of the settings type, for a builder in another crate that holds the settings. A
/// [`SerialBuilder`] needs no import.
pub mod prelude {
    pub use crate::settings::SerialSettingsSetters;
}
