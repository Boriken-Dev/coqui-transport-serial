//! The COBS and CRC-32C framing: its bytes against vectors made by another implementation, and what
//! it does with damage: a damaged frame is never delivered, and the frames after it are.

use coqui_transport::frame::{Codec, Decode, FrameError};
use coqui_transport_serial::Cobs;
use proptest::prelude::*;

const MAX: usize = 1024;

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex"))
        .collect()
}

/// Every message the decoder yields from `wire`, pushed in pieces of `piece` bytes.
fn decode(codec: &Cobs, wire: &[u8], piece: usize) -> Vec<Vec<u8>> {
    let mut decoder = codec.decoder(MAX);
    let mut found = Vec::new();
    for part in wire.chunks(piece.max(1)) {
        decoder.push(part);
        while let Some(message) = decoder.next_frame().expect("this decoder never fails") {
            found.push(message);
        }
    }
    found
}

fn encode(codec: &Cobs, message: &[u8]) -> Vec<u8> {
    codec.encode(message, MAX).expect("within the maximum")
}

fn vectors() -> Vec<serde_json::Value> {
    let text = include_str!("vectors/cobs_crc32c.json");
    serde_json::from_str::<serde_json::Value>(text)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn the_vectors_made_in_python_are_the_bytes_this_writes_and_reads() {
    let vectors = vectors();
    assert!(vectors.len() >= 12);
    let codec = Cobs::new();
    for vector in &vectors {
        let name = vector["name"].as_str().unwrap();
        let message = unhex(vector["message"].as_str().unwrap());
        let frame = unhex(vector["frame"].as_str().unwrap());
        assert_eq!(
            codec.encode(&message, 4096).unwrap(),
            frame,
            "{name}: encode"
        );
        assert_eq!(
            decode(&codec, &frame, 1_000_000),
            std::slice::from_ref(&message),
            "{name}: decode"
        );
        assert_eq!(
            decode(&codec, &frame, 1),
            [message],
            "{name}: decode a byte at a time"
        );
    }
}

#[test]
fn the_other_form_of_a_full_run_decodes_too() {
    // COBS may end a full run with a trailing 0x01 block or not; both decode to the same bytes.
    let codec = Cobs::new();
    let vectors = vectors();
    let vector = vectors
        .iter()
        .find(|v| v["name"] == "body ends on a full run")
        .unwrap();
    let message = unhex(vector["message"].as_str().unwrap());
    let mut frame = unhex(vector["frame"].as_str().unwrap());
    let last = frame.len() - 1;
    frame.splice(last..last, [0x01]);
    assert_eq!(decode(&codec, &frame, 7), [message]);
}

#[test]
fn a_message_over_the_maximum_is_refused_and_the_maximum_is_the_largest_allowed() {
    let codec = Cobs::new();
    assert_eq!(codec.encode(&[1; 11], 10), Err(FrameError::Size));
    assert!(codec.encode(&[1; 10], 10).is_ok());
    assert!(codec.encode(&[], 0).is_ok());
}

#[test]
fn a_frame_is_never_longer_than_the_bound_the_docs_give() {
    let codec = Cobs::new();
    for len in [0, 1, 249, 250, 251, 253, 254, 500, 1000, 1024] {
        for fill in [0_u8, 1, 0xFF] {
            let frame = encode(&codec, &vec![fill; len]);
            assert!(
                frame.len() <= len + 7 + (len + 4) / 254,
                "{len} of {fill}: {}",
                frame.len()
            );
        }
    }
}

#[test]
fn every_single_bit_flip_in_a_stream_of_frames_delivers_no_wrong_message() {
    let codec = Cobs::new();
    let sent: Vec<Vec<u8>> = vec![
        b"first".to_vec(),
        vec![0, 0, 1, 0],
        Vec::new(),
        (0..=255).collect(),
    ];
    let wire: Vec<u8> = sent.iter().flat_map(|m| encode(&codec, m)).collect();
    for bit in 0..wire.len() * 8 {
        let mut damaged = wire.clone();
        damaged[bit / 8] ^= 1 << (bit % 8);
        let got = decode(&Cobs::new(), &damaged, 64);
        // What arrives is some of what was sent, in order, and nothing else.
        let mut rest = sent.iter();
        for message in &got {
            assert!(
                rest.any(|sent| sent == message),
                "bit {bit} delivered a message that was not sent"
            );
        }
        // One flipped bit costs at most the frame it is in and a neighbour joined to it.
        assert!(
            got.len() + 2 >= sent.len(),
            "bit {bit}: only {} of {} survived",
            got.len(),
            sent.len()
        );
    }
}

#[test]
fn a_decoder_finds_its_place_after_noise_a_cut_frame_and_a_flood() {
    let codec = Cobs::new();
    let counters = codec.counters();
    let good = encode(&codec, b"a good message");
    let mut wire = Vec::new();
    // Noise with no delimiter, ending mid-air; then a frame; a frame cut short; a frame; noise
    // with delimiters; a frame.
    wire.extend_from_slice(&[0x55; 100]);
    wire.extend_from_slice(&good);
    wire.extend_from_slice(&good[..good.len() / 2]);
    wire.extend_from_slice(&good);
    wire.extend_from_slice(&[0x07, 0x00, 0x33, 0x44, 0x00, 0x09]);
    wire.extend_from_slice(&good);
    let got = decode(&codec, &wire, 5);
    assert_eq!(got.len(), 3, "{got:?}");
    assert!(got.iter().all(|m| m == b"a good message"));
    assert_eq!(counters.delivered(), 3);
    assert!(counters.corrupt() >= 3, "{}", counters.corrupt());
}

#[test]
fn a_flood_with_no_delimiter_is_dropped_as_it_grows_and_counted_once() {
    let codec = Cobs::new();
    let counters = codec.counters();
    let mut decoder = codec.decoder(100);
    for _ in 0..1000 {
        decoder.push(&[0x42; 64]);
        assert!(decoder.next_frame().unwrap().is_none());
        assert!(
            decoder.buffered() <= 120,
            "{} bytes held",
            decoder.buffered()
        );
    }
    assert_eq!(counters.oversized(), 1);
    // The flood ends at the next delimiter, and the frame after it arrives.
    let good = codec.encode(b"after", 100).unwrap();
    decoder.push(&[0x42, 0x42]);
    decoder.push(&good);
    assert_eq!(
        decoder.next_frame().unwrap().as_deref(),
        Some(b"after".as_slice())
    );
    assert_eq!(counters.delivered(), 1);
}

#[test]
fn a_whole_frame_longer_than_the_maximum_is_dropped_and_counted() {
    let codec = Cobs::new();
    let counters = codec.counters();
    let big = codec.encode(&[7; 500], 500).unwrap();
    let small = codec.encode(b"ok", 500).unwrap();
    let mut decoder = codec.decoder(100);
    decoder.push(&big);
    decoder.push(&small);
    assert_eq!(
        decoder.next_frame().unwrap().as_deref(),
        Some(b"ok".as_slice())
    );
    assert_eq!(decoder.next_frame().unwrap(), None);
    assert_eq!(counters.oversized(), 1);
    // The maximum moves at the next frame.
    decoder.set_maximum(1000);
    decoder.push(&big);
    assert_eq!(decoder.next_frame().unwrap().map(|m| m.len()), Some(500));
}

proptest! {
    #[test]
    fn what_is_encoded_is_decoded_however_the_bytes_arrive(
        messages in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..600), 0..8),
        piece in 1_usize..700,
    ) {
        let codec = Cobs::new();
        let wire: Vec<u8> = messages.iter().flat_map(|m| codec.encode(m, MAX).unwrap()).collect();
        prop_assert_eq!(decode(&codec, &wire, piece), messages);
    }

    #[test]
    fn damage_never_delivers_a_message_that_was_not_sent(
        messages in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..300), 1..6),
        edits in proptest::collection::vec((any::<proptest::sample::Index>(), 0_u8..4, any::<u8>()), 1..6),
        piece in 1_usize..300,
    ) {
        let codec = Cobs::new();
        let mut wire: Vec<u8> = messages.iter().flat_map(|m| codec.encode(m, MAX).unwrap()).collect();
        for (at, kind, value) in edits {
            if wire.is_empty() { break; }
            let at = at.index(wire.len());
            match kind {
                0 => wire[at] ^= value | 1,
                1 => { wire.remove(at); }
                2 => wire.insert(at, value),
                _ => { let end = (at + usize::from(value % 24) + 1).min(wire.len()); wire.drain(at..end); }
            }
        }
        let got = decode(&Cobs::new(), &wire, piece);
        let mut rest = messages.iter();
        for message in &got {
            prop_assert!(rest.any(|sent| sent == message), "delivered a message that was not sent");
        }
    }
}
