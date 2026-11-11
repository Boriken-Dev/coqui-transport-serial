# coqui-transport-serial

[![Version](https://img.shields.io/crates/v/coqui-transport-serial.svg)](https://crates.io/crates/coqui-transport-serial)
[![Rust version](https://img.shields.io/crates/msrv/coqui-transport-serial.svg)](https://crates.io/crates/coqui-transport-serial)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/coqui-transport-serial/blob/main/LICENSE)
[![Docs](https://img.shields.io/docsrs/coqui-transport-serial)](https://docs.rs/coqui-transport-serial)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/coqui-transport-serial/test.yml)](https://github.com/Boriken-Dev/coqui-transport-serial/actions/workflows/test.yml)

**Serial ports as a [`coqui-transport`](https://github.com/Boriken-Dev/coqui-transport) stream, with a framing that
detects the damage a serial line does and carries on after it.** A `Serial` splits into a reading and a writing half, so
`coqui_transport::StreamChannel` makes a channel of whole messages on a COM port, a `/dev/tty*` device or a
pseudo-terminal; the `Cobs` codec frames them with COBS and a CRC-32C. See the
[documentation](https://boriken-dev.github.io/coqui-transport-serial/).

> **Status: alpha.** `0.0.0-alpha.0` is unreleased: depend on a git revision. The API may change at any commit.
> Tested on Linux with pseudo-terminals; the Windows path has run with no COM port to open (see Platforms).

## Features

- **Any port the system opens**: Windows COM ports, Linux `/dev/tty*` (USB adapters, on-board UARTs), macOS and BSD
  devices, and pseudo-terminals. Baud rate (any rate the driver takes), data bits, parity, stop bits, software or hardware
  flow control, DTR and RTS at open, and the modem lines afterwards (`Serial::lines`). Every one is a setting with a default,
  read from a file or set on a builder.
- **A framing made for a damaging wire.** `Cobs` writes each message as COBS bytes ending in a CRC-32C, between zero
  delimiters. A frame cut short, repeated, spliced or flipped is never delivered: it is dropped and counted, and the decoder
  resynchronizes at the next delimiter. No double framing: the codec is the channel's only framing.
- **Threads that can be woken.** A blocked read wakes every `wake` (250 ms) to see that its channel closed, so closing
  never leaves a thread behind and never waits on a silent line.
- **Honest about the line.** A serial line has no end of stream, no flow control by default and, on two-wire RS-485, no
  full duplex. The documentation says what that means for a channel instead of hiding it.
- **Four dependencies of its own** (`serial2`, `cobs`, `crc` and the family's `coqui-builder`), and no `unsafe`.

## Installation

```bash
cargo add coqui-transport-serial --git https://github.com/Boriken-Dev/coqui-transport-serial
```

| Feature | Adds | Needed for | Default |
| --- | --- | --- | --- |
| `rs485` | `serial2/rs4xx` | `Transceiver::Rs422`, `Rs485` and `Rs485FullDuplex` (Linux) | off |

## Quick start

A channel over a serial port, both ends framed with `Cobs`:

```rust,no_run
use std::time::Duration;

use coqui_transport::{Channel, StreamChannel};
use coqui_transport_serial::{Cobs, SerialBuilder};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let port = SerialBuilder::new().path("/dev/ttyUSB0").baud_rate(921_600).build()?;
let channel = StreamChannel::start(port, Cobs::new())?;
channel.send(b"hello")?;
let answer = channel.receive(Some(Duration::from_secs(5)))?;
# let _ = answer;
# Ok(()) }
```

The codec alone, and what damage does to a frame:

```rust
use coqui_transport::frame::{Codec, Decode};
use coqui_transport_serial::Cobs;

let codec = Cobs::new();
let counters = codec.counters();
let mut wire = codec.encode(b"first", 1024).unwrap();
let mut damaged = codec.encode(b"second", 1024).unwrap();
damaged[3] ^= 0x01;
wire.extend_from_slice(&damaged);
wire.extend_from_slice(&codec.encode(b"third", 1024).unwrap());

let mut decoder = codec.decoder(1024);
decoder.push(&wire);
let mut got = Vec::new();
while let Ok(Some(message)) = decoder.next_frame() {
    got.push(message);
}
assert_eq!(got, [b"first".to_vec(), b"third".to_vec()]);
assert_eq!((counters.delivered(), counters.corrupt()), (2, 1));
```

## API overview

| Item | Purpose |
| --- | --- |
| `Serial`, `SerialBuilder`, `SerialSettings` | open a port and set it up; `Serial` is a `coqui_transport::Split` |
| `Connector` | opens a port per `connect`, with a real timeout |
| `Lines` | RTS and DTR out, CTS, DSR, RI and CD in, usable after the split |
| `Cobs`, `CobsDecoder`, `Counters` | the framing, and what it dropped |
| `DataBits`, `Parity`, `StopBits`, `FlowControl`, `Transceiver` | the settings' choices |

## The framing

```text
00  COBS( message ‖ CRC-32C(message), 4 bytes little-endian )  00
```

COBS leaves the body with no zero byte, so zero is the delimiter a reader that lost its place looks for; a frame is
delimited on both sides, and stretches between delimiters that hold nothing are ignored. Overhead for a message of `n`
bytes is 7 bytes plus one per 254, at most. In a million trials per case (bit flips, 17 to 48-bit bursts, a byte dropped or inserted, random replacement) no damaged frame
opened as another message under CRC-32C, where a 16-bit CRC let about one in 100 000 through. The measurement behind the
choice (against SLIP and HDLC byte stuffing, and CRC-16 against CRC-32) is in the repository's `docs/`.

A damaged frame is **dropped, not reported as a failure**: the channel carries on, and `Counters` says how many were
lost. A channel that cannot lose a message acknowledges and repeats above this one.

## Half-duplex, RS-485 and what a serial line is not

- **No end of stream.** Dropping a writing half sends nothing. The peer's `receive` does not return `Ok(None)` when this
  side closes; a protocol that needs a close says so in a message.
- **Full duplex is the default assumption, and it is the wire's.** RS-232 and four-wire RS-422 or RS-485 carry both
  directions at once and need nothing more. **Two-wire RS-485 is half-duplex**: one node drives the bus at a time, and
  this crate does nothing to arbitrate it. Put a request-and-response protocol on top, where one side speaks only when
  spoken to, or a token; two sides sending at once corrupt each other's frames, which the CRC detects and the counters
  show, and nothing repairs.
- **Turning the driver around.** Most USB RS-485 adapters switch direction in hardware. On Linux a port whose driver
  supports it can have the kernel switch it (`Transceiver::Rs485`, feature `rs485`): the kernel raises RTS before each
  transmission and lowers it after, and a driver that does not support it ignores the request or refuses it. This is
  configuration, not arbitration.
- **Echo.** A two-wire RS-485 transceiver that leaves its receiver on while transmitting hears its own frames. The channel
  would then receive what it sent. Turn the receiver off while driving (`Transceiver::Rs485` asks the kernel to) or
  filter above the channel.
- **Flow control** defaults to none: a sender faster than its receiver loses bytes, and the CRC finds the damage. Use
  `hardware` where the cable has RTS and CTS.
- **Timing.** The write timeout and the `wake` are settings; a slow baud rate makes a large message take its time
  (115200 baud is about 11 KB/s), and a channel's `send_limit` should allow for it.

## Platforms

Tested on Linux against pseudo-terminals (both by `Serial::pair` and by opening a pseudo-terminal slave by its path, with
baud rate, stop bits and hardware flow control read back from the kernel; its driver forces 8 data bits and no parity, so those two are unverified). Windows builds and its tests run, but the machine had no COM port and no virtual
pair (com0com) to open, so a Windows COM port has not carried a frame; `tests/hardware.rs` runs against two cabled ports
named by `COQUI_TEST_SERIAL_A` and `COQUI_TEST_SERIAL_B`, on any platform that has them. macOS and the BSDs share the Unix
code and have not run.

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

The framing's vectors are made by `tools/vectors.py`, an implementation of COBS and CRC-32C separate from the crate's
(`python tools/vectors.py > tests/vectors/cobs_crc32c.json`). The measurement program is `tools/framing_measure.rs` (not built by the crate).

### Releasing

Nothing is released until the owner chooses a version and tags it: `v*` tags run the test gate, then publish.

## License

[MPL-2.0](https://github.com/Boriken-Dev/coqui-transport-serial/blob/main/LICENSE).
