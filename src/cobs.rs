//! The serial framing: COBS with a CRC-32C, delimited by zero bytes.
//!
//! One frame on the wire is
//!
//! ```text
//! 00  COBS( message ‖ CRC-32C(message) as 4 little-endian bytes )  00
//! ```
//!
//! COBS (consistent overhead byte stuffing) rewrites the body so it holds no zero byte, which
//! leaves zero free as the delimiter: a reader that has lost its place finds the next frame by
//! looking for one. The zero that ends a frame may also start the next, so a frame is delimited
//! from both sides and a receiver that joins mid-frame or sees line noise before a frame loses at
//! most what the noise touched. Empty stretches between delimiters are ignored.
//!
//! The CRC is CRC-32C (Castagnoli: polynomial `0x1EDC6F41`, reflected, initial value and final xor
//! `0xFFFFFFFF`, the "iSCSI" CRC). A frame whose COBS structure is invalid, or whose CRC does not
//! match, is never delivered: it is dropped, counted ([`Counters`]) and the decoder carries on at
//! the next delimiter.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use coqui_transport::frame::{Codec, Decode, FrameError};
use crc::{CRC_32_ISCSI, Crc};

const CRC: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);
const CRC_LEN: usize = 4;

/// The most bytes COBS adds to `n` bytes: one code byte per 254, and one.
const fn cobs_len(n: usize) -> usize {
    n + n / 254 + 1
}

/// COBS with a CRC-32C as a [`Codec`], for a byte stream that can lose, repeat or damage bytes.
///
/// Give a clone of one to the channel and keep another to read its [`Counters`]. A message of at
/// most `maximum` bytes becomes a frame of at most `maximum + 7 + (maximum + 4) / 254` bytes: the
/// overhead is two delimiters, the 4-byte CRC, one COBS code byte and one more per 254 bytes.
///
/// A damaged frame is dropped rather than ending the channel, so a message can be lost on a
/// damaged link: this codec detects damage, it does not repair it. A protocol that cannot lose a
/// message acknowledges and repeats above the channel.
#[derive(Clone, Debug, Default)]
pub struct Cobs {
    counters: Counters,
}

impl Cobs {
    /// A codec with counters of its own at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What every decoder made by this codec and its clones has found so far.
    #[must_use]
    pub fn counters(&self) -> Counters {
        self.counters.clone()
    }
}

/// What the decoders of a [`Cobs`] codec have found, shared by it and its clones.
#[derive(Clone, Debug, Default)]
pub struct Counters {
    inner: Arc<Totals>,
}

#[derive(Debug, Default)]
struct Totals {
    delivered: AtomicU64,
    corrupt: AtomicU64,
    oversized: AtomicU64,
}

impl Counters {
    /// Frames whose structure and CRC were intact, and which were delivered.
    #[must_use]
    pub fn delivered(&self) -> u64 {
        self.inner.delivered.load(Ordering::SeqCst)
    }

    /// Frames dropped for an invalid COBS structure, a body too short to hold a CRC or a CRC that
    /// did not match.
    #[must_use]
    pub fn corrupt(&self) -> u64 {
        self.inner.corrupt.load(Ordering::SeqCst)
    }

    /// Stretches between delimiters longer than a frame of the largest message can be, dropped
    /// unread.
    #[must_use]
    pub fn oversized(&self) -> u64 {
        self.inner.oversized.load(Ordering::SeqCst)
    }
}

impl Codec for Cobs {
    type Decoder = CobsDecoder;

    fn encode(&self, message: &[u8], maximum: usize) -> Result<Vec<u8>, FrameError> {
        if message.len() > maximum {
            return Err(FrameError::Size);
        }
        let mut body = Vec::with_capacity(message.len() + CRC_LEN);
        body.extend_from_slice(message);
        body.extend_from_slice(&CRC.checksum(message).to_le_bytes());
        let encoded = cobs::encode_vec(&body);
        let mut frame = Vec::with_capacity(encoded.len() + 2);
        frame.push(0);
        frame.extend_from_slice(&encoded);
        frame.push(0);
        Ok(frame)
    }

    fn decoder(&self, maximum: usize) -> CobsDecoder {
        CobsDecoder {
            maximum,
            raw: Vec::new(),
            skipping: false,
            counters: self.counters.clone(),
        }
    }
}

/// The decoder of [`Cobs`]: finds frames between zero bytes, checks each and returns the messages
/// of the intact ones. It never fails: damage is counted and passed over.
#[derive(Debug)]
pub struct CobsDecoder {
    maximum: usize,
    /// Bytes pushed and not yet taken apart.
    raw: Vec<u8>,
    /// The stretch being read is too long to be a frame: drop it up to its delimiter.
    skipping: bool,
    counters: Counters,
}

impl CobsDecoder {
    /// The longest stretch between delimiters that can hold a frame of the largest message.
    const fn bound(&self) -> usize {
        cobs_len(self.maximum.saturating_add(CRC_LEN))
    }

    /// The message in one stretch between delimiters, if it is a whole intact frame.
    fn open(&self, stretch: &[u8]) -> Option<Vec<u8>> {
        let mut body = cobs::decode_vec(stretch).ok()?;
        let message_len = body.len().checked_sub(CRC_LEN)?;
        let (message, crc) = body.split_at(message_len);
        (CRC.checksum(message).to_le_bytes() == crc).then(|| {
            body.truncate(message_len);
            body
        })
    }
}

impl Decode for CobsDecoder {
    fn set_maximum(&mut self, maximum: usize) {
        self.maximum = maximum;
    }

    fn push(&mut self, part: &[u8]) {
        self.raw.extend_from_slice(part);
    }

    fn next_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        loop {
            let Some(end) = self.raw.iter().position(|byte| *byte == 0) else {
                // No delimiter yet: what is held is the start of a frame, or of a stretch too long
                // to be one, which is dropped as it grows so that line noise cannot fill memory.
                if self.skipping {
                    self.raw.clear();
                } else if self.raw.len() > self.bound() {
                    self.counters.inner.oversized.fetch_add(1, Ordering::SeqCst);
                    self.skipping = true;
                    self.raw.clear();
                }
                return Ok(None);
            };
            let stretch: Vec<u8> = self.raw.drain(..=end).take(end).collect();
            if core::mem::take(&mut self.skipping) {
                continue;
            }
            if stretch.is_empty() {
                continue;
            }
            if stretch.len() > self.bound() {
                self.counters.inner.oversized.fetch_add(1, Ordering::SeqCst);
                continue;
            }
            match self.open(&stretch) {
                Some(message) => {
                    self.counters.inner.delivered.fetch_add(1, Ordering::SeqCst);
                    return Ok(Some(message));
                }
                None => {
                    self.counters.inner.corrupt.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    }

    fn buffered(&self) -> usize {
        self.raw.len()
    }
}
