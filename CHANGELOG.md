# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `Serial`: a serial port (a Windows COM port, a Unix tty device, a pseudo-terminal) as a `coqui_transport::Split`
  stream, so `StreamChannel::start(port, codec)` is a channel of whole messages on it. `Serial::open(path)`, `SerialBuilder`
  with the settings by name (`baud_rate`, `data_bits`, `parity`, `stop_bits`, `flow_control`, `transceiver`,
  `write_timeout`, `wake`, `discard_input`, `dtr`, `rts`) and the port's `path` as the required input, `Serial::pair()`
  (Unix) for two pseudo-terminals joined as a null-modem cable, `Serial::lines()` for RTS and DTR out and CTS, DSR, RI and CD in.
  `SerialSettings` is a `coqui-builder` setting a file fills (`Partial<SerialSettings>`); `non_defaults()` lists what differs
  from the defaults.
- `Connector`: opens a port per `connect`, with a timeout that holds.
- `Cobs`: the serial framing as a `coqui_transport::frame::Codec`: a zero byte, the message and its CRC-32C (4 bytes,
  little-endian) COBS-encoded, a zero byte. A frame whose structure or CRC is wrong, or that is longer than the largest
  message allows, is dropped and counted (`Counters::corrupt`, `oversized`) and the decoder carries on at the next
  zero byte; nothing damaged is delivered. Test vectors come from an implementation in Python that shares no code with it.
- `Transceiver::Rs422`, `Rs485` and `Rs485FullDuplex` with feature `rs485` (Linux): the kernel's RS-485 mode, where the
  driver supports it.
