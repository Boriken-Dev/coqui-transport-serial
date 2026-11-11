# `coqui-transport-serial` — API header

Header-file-style reference for the `coqui-transport-serial` crate: every public item with its signature, contract and
gotchas, so the crate can be used without reading its source. It ships inside the package and is self-contained.
Development documentation lives with the source at <https://github.com/Boriken-Dev/coqui-transport-serial>.

| Feature | Adds | Needed for | Default |
| --- | --- | --- | --- |
| `rs485` | `serial2/rs4xx` | `Transceiver::Rs422`, `Rs485`, `Rs485FullDuplex` (Linux only; refused with `Unsupported` otherwise) | off |

MSRV Rust 1.89. A default build depends on `coqui-builder` (by git revision, `derive`), `coqui-transport` (by git revision,
`2601da5`, no features), `serial2` 0.2 (itself `libc` and `windows-sys`), `cobs` 0.5 and `crc` 3. Version `0.0.0-alpha.0`,
unreleased: depend on a git revision, and name the same `coqui-builder` and `coqui-transport` revisions if you use them directly.

Imports: the crate root holds `Serial`, `SerialBuilder`, `SerialParts`, `SerialSettings`, `Connector`, `Lines`, `ReadHalf`,
`WriteHalf`, `Cobs`, `CobsDecoder`, `Counters`, `DataBits`, `Parity`, `StopBits`, `FlowControl`, `Transceiver`, and re-exports
`BuildError`, `Missing` and `NonDefault` of `coqui-builder`. One public module: `prelude` (`SerialSettingsSetters`, for a
builder in another crate; `SerialBuilder` has the setters as methods).

No `unsafe` (`#![forbid(unsafe_code)]`). No process-wide state. A library prints nothing.

## Ports (crate root)

```rust
pub struct Serial;                              // Debug; Split
impl Serial {
    pub const DEFAULT_BAUD_RATE: u32;           // 115200
    pub const DEFAULT_WRITE_TIMEOUT: Duration;  // 1 s
    pub const DEFAULT_WAKE: Duration;           // 250 ms
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self>;       // default settings
    pub fn pair() -> io::Result<(Self, Self)>;  // Unix: two pseudo-terminals as a null-modem cable
    pub fn lines(&self) -> Lines;
}
// Derived by coqui_builder::Builder from SerialParts (settings + input `path`). Setters are inherent methods.
impl SerialBuilder {
    pub fn new() -> Self;  pub fn path(self, path: impl Into<PathBuf>) -> Self;                    // required
    pub fn baud_rate(self, u32) / data_bits(impl Into<DataBits>) / parity(Parity) / stop_bits(StopBits) / flow_control(FlowControl)
        / transceiver(Transceiver) / write_timeout(impl Into<Duration>) / wake(impl Into<Duration>) / discard_input(bool)
        / dtr(impl Into<Option<bool>>) / rts(impl Into<Option<bool>>) -> Self;
    pub fn fill(self, settings: Partial<SerialSettings>) -> Self;  pub fn non_defaults(&self) -> Vec<NonDefault>;
    pub fn build(self) -> Result<Serial, BuildError<io::Error>>;   // opens the port
}
#[non_exhaustive] pub struct SerialSettings { pub baud_rate: u32, pub data_bits: DataBits, pub parity: Parity,
    pub stop_bits: StopBits, pub flow_control: FlowControl, pub transceiver: Transceiver, pub write_timeout: Duration,
    pub wake: Duration, pub discard_input: bool, pub dtr: Option<bool>, pub rts: Option<bool> }   // coqui_builder::Describe; Default
// Defaults: 115200, 8, none, 1, none, default, 1s, 250ms, true, unset, unset. Text forms: data_bits "5".."8", parity
// none|odd|even, stop_bits "1"|"2", flow_control none|software|hardware, transceiver default|rs422|rs485|rs485-full-duplex,
// dtr and rts true|false|unchanged.
pub struct Lines;                                // Clone, Debug; shares the port, usable after Serial is split
impl Lines { pub fn set_dtr(&self, bool) / set_rts(&self, bool) -> io::Result<()>;
             pub fn read_cts / read_dsr / read_ri / read_cd(&self) -> io::Result<bool>; }
pub struct Connector;                            // Clone, Debug, must_use
impl Connector { pub fn new(path: impl Into<PathBuf>) -> Self; pub const fn settings(self, SerialSettings) -> Self; pub fn path(&self) -> &Path; }
impl coqui_transport::Connector for Connector { type Connection = Serial; type Error = io::Error; }
impl Split for Serial { type Reader = ReadHalf; type Writer = WriteHalf; }   // with an Interrupt
```

`path` is a device path on Unix (`/dev/ttyUSB0`) and a COM port on Windows (`COM3`; ports above 9 take `\\.\COM10`, which `serial2` adds
to a plain `COM10`). Opening configures the characters, sets the read timeout to `wake` and the write timeout to `write_timeout`,
applies the transceiver mode, sets DTR and RTS if given and, if `discard_input`, throws away what the driver received already. A port that is not
there is `ErrorKind::NotFound`. The driver may refuse a combination it cannot do (a pseudo-terminal forces 8 data bits and no
parity): the open then fails with an `Other` error.

`ReadHalf` reads and returns `TimedOut` after `wake` with nothing received, which a `StreamChannel` retries; `WriteHalf` writes and
returns `TimedOut` when the port took none for `write_timeout`, also retried. After the interrupt (a channel closing) the next read
returns `ConnectionAborted` and the next write `BrokenPipe`, so a close takes up to `wake` (`write_timeout` for a stalled write). The
port closes when its last half and every `Lines` are dropped. `Connector::connect` opens on a thread and gives up waiting after `timeout`
(`TimedOut`; a port that opens late is closed again); a zero timeout is `InvalidInput`.

## Framing (crate root)

```rust
pub struct Cobs;                                 // Clone, Debug, Default; coqui_transport::frame::Codec
impl Cobs { pub fn new() -> Self; pub fn counters(&self) -> Counters; }
pub struct CobsDecoder;                          // Debug; coqui_transport::frame::Decode; never fails
pub struct Counters;                             // Clone, Debug, Default; shared by a codec, its clones and their decoders
impl Counters { pub fn delivered(&self) -> u64; pub fn corrupt(&self) -> u64; pub fn oversized(&self) -> u64; }
```

Wire format of one frame: `00 ‖ COBS(message ‖ CRC-32C(message) as 4 little-endian bytes) ‖ 00`. CRC-32C is polynomial `0x1EDC6F41`
reflected, initial value and final xor `0xFFFFFFFF` (check value of `123456789`: `0xE3069283`). A frame for a message of `n` bytes is at
most `n + 7 + (n + 4) / 254` bytes. `encode` refuses a message over `maximum` with `FrameError::Size`; an empty message is allowed. The
decoder returns the messages of intact frames in order; a stretch between zero bytes whose COBS structure or CRC is wrong,
or that is too short to hold a CRC, is dropped and counted in `corrupt`; one longer than a frame of the largest message can be is
dropped as it grows (so noise with no zero cannot fill memory) and counted once in `oversized`; empty stretches are ignored. Both forms
of COBS for data ending on a full 254-byte run decode alike. Damage is therefore never delivered and never an error: a message can be
lost, and the channel goes on. `Cobs` is the channel's only framing: do not put `LengthPrefix` under it.

## Gotchas

- **The versioned frame over `Cobs`.** `coqui_transport::mux::Mux::start(StreamChannel::start(port, Cobs::new())?)` runs logical channels
  and over-stream datagrams on a port (`coqui-transport`'s `docs/frame.md`). The frame has its own CRC-32C, redundant here (4 bytes);
  `Cobs` drops a damaged frame first, so the multiplexer sees a gap: a message channel ends with `Failure::Protocol`, a datagram is lost and
  counted (`Mux::counters().lost_datagrams`, beside `Counters::corrupt`). `tests/mux.rs` runs it over a damaging in-memory wire and, on Unix, a
  pseudo-terminal pair.

- **A serial line has no end of stream.** Closing a channel sends nothing; the peer's `receive` does not return `Ok(None)`. A protocol
  that needs a close sends one as a message. (A pseudo-terminal pair does hang up when one end closes; a cable does not.)
- **Lost messages are not an error.** Count `Counters::corrupt` and `oversized`, and acknowledge and repeat above the channel where a
  message must not be lost.
- **Two-wire RS-485 is half-duplex**: this crate neither arbitrates the bus nor turns the driver around (adapters usually do it in hardware;
  `Transceiver::Rs485` asks a Linux kernel driver to, and a driver may ignore the request). A transceiver that keeps its receiver on while
  transmitting returns the channel its own frames. Two nodes sending at once corrupt each other's frames: the CRC detects it.
- **Flow control defaults to none**: a sender faster than its receiver loses bytes, which the framing detects and drops. `software`
  flow control (XON/XOFF) consumes bytes `0x11` and `0x13` that a binary framing carries: do not use it with this one.
- **Baud rate and a channel's limits**: 115200 baud is about 11 KB/s; a channel's `send_limit` and a message's size should agree with it.
- **DTR**: many devices reset when DTR changes, which opening a port can cause on some systems.
- **Port enumeration is not offered**; open by name.
- A `Serial` is not an authenticated peer.
- Tested: Linux (pseudo-terminals, x86-64 and aarch64); Windows builds and its tests run with no COM port to open; macOS and the BSDs share the
  Unix code and have not run. Tests with real cabled ports exist (`tests/hardware.rs`, ignored) and have not run.
