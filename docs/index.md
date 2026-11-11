# coqui-transport-serial

Serial ports as a stream of [`coqui-transport`](https://github.com/Boriken-Dev/coqui-transport), so a channel of whole
messages runs over a COM port, a `/dev/tty*` device or a pseudo-terminal, with a framing that detects the damage a serial
line does.

!!! note "Alpha"
    `0.0.0-alpha.0` is unreleased: depend on a git revision. The API may change in any commit.

- **The framing** is COBS with a CRC-32C between zero delimiters. A damaged frame is dropped and counted, never
  delivered, and the decoder resynchronizes at the next delimiter.
- **The line is a wire**: it has no end of stream, flow control defaults to none, and two-wire RS-485 is half-duplex. The
  README says what each means for a channel.
- **Why these choices** (the three serial crates measured, and the framing against SLIP, HDLC and CRC-16) is on the
  [measurements page](measurements.md).

The API reference is under
[Rust API](https://boriken-dev.github.io/coqui-transport-serial/rust/index.html) for `main`. The source,
the API header (`AGENTS.md`) and the changelog are in the
[repository](https://github.com/Boriken-Dev/coqui-transport-serial).
