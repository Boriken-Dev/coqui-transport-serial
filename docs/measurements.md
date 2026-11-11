# Serial crates and serial framing: measurements (2026-11-11)

For `coqui-transport-serial`. Sources: the crates.io API (versions, dates, downloads, licence, MSRV), the registry
sources (`unsafe` counts, features, API), `cargo tree` of a probe crate, and a probe run in WSL (Fedora, aarch64) over
pseudo-terminals. Probe sources were in the scratch directory and are removed; the framing script is kept as
`coqui-transport-serial/tools/framing_measure.rs`.

## The crates

| | `serialport` 4.10.1 | `serial2` 0.2.39 | `tokio-serial` 5.5.0 (over `mio-serial` 5.0.7, `serialport`) |
| --- | --- | --- | --- |
| Newest release | 2026-09-08 (4.9.0 2026-03, 4.8.1 2025-10) | 2026-10-26 (about monthly since 2026-02) | 2026-06-15 (previous 2024-12-31) |
| Recent downloads (90 days) | 4.0 M | 16.3 M | 1.9 M |
| Licence | MPL-2.0 | BSD-2-Clause OR Apache-2.0 | MIT |
| MSRV declared | 1.59 | 1.63 | none |
| Platforms | Windows, Linux, macOS, BSD, more | Windows, Linux, macOS, BSD (Unix by termios) | as `serialport` |
| Sync API | `SerialPort` trait, `Box<dyn>`, one timeout for both directions, `set_timeout(&mut self)`, `try_clone` for a second handle | `read`/`write` take `&self`: one `Arc` shared by a reading and a writing thread, no clone; separate read and write timeouts | none: tokio streams only (also `std::io` on `SerialStream`, which is nonblocking) |
| Async | none | none (`serial2-tokio` 0.1.26, `serial2-smol`, a separate crate each) | tokio, mio |
| Timeouts | read/write, one value | read and write, separate; Windows `COMMTIMEOUTS` exposed | tokio's |
| Pseudo-terminal pair | `TTYPort::pair()` (Unix) | `SerialPort::pair()` (Unix, feature `unix`) | `SerialStream::pair()` |
| RS-485/RS-422 | none | `set_rs4xx_mode` (Linux, feature `rs4xx`: the kernel's RS-485 ioctl, full duplex, termination, delay) | none |
| Port enumeration | Linux (libudev, optional), Windows, macOS, FreeBSD | Linux only | via `serialport` |
| Arbitrary baud rates | yes | yes (Linux, BSD, Windows, macOS) | yes |
| Direct dependencies (normal, all targets) | `bitflags` 1 and 2, `cfg-if`, `nix` 0.26, `scopeguard`, `thiserror`, `unescaper`, `windows-sys` 0.52, core-foundation, io-kit-sys, mach2; `libudev` by default feature | `libc`, `cfg-if`, `windows-sys` 0.61, `windows-link` | `serialport`, `mio-serial`, `mio`, `tokio`, `nix` (two versions), `futures`, `log`, `bitflags`, `socket2` |
| Distinct crates in `cargo tree` (all targets) | 30 | 5 | about 50 |
| `unsafe` lines (src) | 97 in 8 files, 5610 lines | 51 in 5 files, 3720 lines | 1 (plus `serialport` and `mio-serial`'s 17) |
| pty throughput, 4 MiB, one writer thread, one reader thread | 140 MiB/s | 216 MiB/s | 235 MiB/s (tokio, multi-thread) |
| Idle read with a 100 ms timeout | `TimedOut` after 100.2 ms | `TimedOut` after 100.2 ms | n/a |

The family already uses `windows-sys` 0.61 and `libc`, which `serial2` shares: it adds no second version of either, where
`serialport` brings `windows-sys` 0.52 and `nix` 0.26 beside them.

## The choice: `serial2`

The channel needs a blocking stream split between a reading and a writing thread, timeouts that wake a blocked read so a
closing channel is noticed, no async runtime, and a way to reach RS-485 where the system has it. `serial2` is the one
whose `&self` reads and writes make the split an `Arc`, with separate timeouts, with the fewest dependencies and the least
`unsafe`, the only RS-485 support, and the most recent releases. `tokio-serial` is async-only and `coqui-transport`'s core
is blocking with async an option; `serial2-tokio` is the route if an async serial transport is wanted later.

What `serial2` lacks, said where it matters: port enumeration outside Linux (a port is opened by name here; enumeration is
left to the application, or to `serialport` beside it), and no macOS, FreeBSD or Windows serial hardware ran here (see the
crate's README).

## The framing

Candidates for a byte stream that can lose, repeat or damage bytes, with a delimiter so a reader can find its place:
COBS (zero delimiter), SLIP (`0xC0`, RFC 1055), HDLC byte stuffing (`0x7E`/`0x7D`); and checksums CRC-16/IBM-SDLC (X.25),
CRC-16/XMODEM, CRC-32/ISO-HDLC and CRC-32C. Bytes added to a message (one delimiter included, checksum excluded), 200
random messages per size:

| message bytes | COBS random | COBS worst | SLIP random | SLIP worst | HDLC worst |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 2.00 | 2 | 1.00 | 2 | 2 |
| 64 | 2.00 | 2 | 1.51 | 65 | 65 |
| 255 | 2.36 | 3 | 3.03 | 256 | 256 |
| 1024 | 3.98 | 6 | 8.87 | 1025 | 1025 |
| 65536 | 152.40 | 260 | 512.77 | 65537 | 65537 |

SLIP and HDLC are smaller on average for small random data and double the size at worst; COBS is bounded at one byte per
254 whatever the data, so a link's worst-case rate and a buffer bound are known: that decides COBS. (The frame adds a second
delimiter and the checksum.)

Damaged COBS frames that still opened as a different message, out of 1 000 000 trials per cell, for 16, 256 and 1500-byte
messages (single and double bit flips, a 17 to 48-bit burst, a byte dropped, a byte inserted, the frame replaced by random
non-zero bytes):

| checksum | 16 B | 256 B | 1500 B |
| --- | --- | --- | --- |
| CRC-16/IBM-SDLC | 11 burst | 12 burst, 3 dropped, 1 inserted, 1 random | 13 burst, 1 inserted |
| CRC-16/XMODEM | 19 burst, 200 dropped, 205 inserted | 15 burst, 18 dropped, 13 inserted, 1 random | 11 burst, 2 dropped, 5 inserted, 1 random |
| CRC-32/ISO-HDLC | 0 | 0 | 0 |
| CRC-32C | 0 | 0 | 0 |

A 16-bit checksum lets about 1 in 100 000 damaged frames through (and XMODEM, with a zero initial value, 1 in 5 000 for
dropped or inserted bytes in short frames: a leading zero byte changes nothing). Both 32-bit checks passed none in
1 000 000 in every column; the difference between them is not measurable at this size, so CRC-32C is chosen: it is the
published choice for frames of this size (iSCSI, ext4, Btrfs), and measured no worse.

Chosen: `00 COBS(message ‖ CRC-32C LE) 00`. Overhead at most 7 bytes + 1 per 254.
