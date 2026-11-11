//! A channel framed with `Cobs` over a byte stream that damages what crosses it: the damaged frame is
//! lost and counted, the channel goes on. No port is needed: an in-memory wire stands in for the line.

use std::io::{Read, Write};
use std::thread;
use std::time::Duration;

use coqui_transport::{Channel, Failure, StreamChannel, memory};
use coqui_transport_serial::Cobs;

const WAIT: Option<Duration> = Some(Duration::from_secs(20));

/// Two ends joined by a wire: what end A writes crosses a relay that flips one bit inside the
/// `damage`th frame (counting from 1), what end B writes crosses it unchanged.
fn wire(damage: Option<usize>) -> (memory::Stream, memory::Stream) {
    let (a, a_far) = memory::pipe(4096);
    let (b_far, b) = memory::pipe(4096);
    let (mut a_in, a_out) = a_far.into_halves();
    let (b_in, mut b_out) = b_far.into_halves();
    let (mut a_out, mut b_in) = (a_out, b_in);
    // A to B, damaging.
    thread::spawn(move || {
        let mut delimiters = 0;
        let mut flipped = false;
        let mut buffer = [0_u8; 512];
        while let Ok(read) = a_in.read(&mut buffer) {
            if read == 0 {
                break;
            }
            for byte in &mut buffer[..read] {
                if *byte == 0 {
                    delimiters += 1;
                } else if !flipped && damage.is_some_and(|n| delimiters == 2 * n - 1) {
                    // The first byte of the frame after its leading delimiter.
                    *byte ^= 0x10;
                    flipped = true;
                }
            }
            if b_out.write_all(&buffer[..read]).is_err() {
                break;
            }
        }
    });
    // B to A, as it was.
    thread::spawn(move || {
        let mut buffer = [0_u8; 512];
        while let Ok(read) = b_in.read(&mut buffer) {
            if read == 0 || a_out.write_all(&buffer[..read]).is_err() {
                break;
            }
        }
    });
    (a, b)
}

#[test]
fn messages_cross_both_ways_whatever_their_size() {
    let (a, b) = wire(None);
    let (ca, cb) = (Cobs::new(), Cobs::new());
    let a = StreamChannel::start(a, ca).unwrap();
    let b = StreamChannel::start(b, cb).unwrap();
    let big: Vec<u8> = (0..200_000).map(|n| (n % 251) as u8).collect();
    let messages = [b"hello".to_vec(), Vec::new(), vec![0; 300], big];
    for message in &messages {
        a.send(message).unwrap();
        b.send(message).unwrap();
    }
    for message in &messages {
        assert_eq!(b.receive(WAIT).unwrap().as_ref(), Some(message));
        assert_eq!(a.receive(WAIT).unwrap().as_ref(), Some(message));
    }
}

#[test]
fn a_damaged_frame_is_lost_and_counted_and_the_channel_goes_on() {
    let (a, b) = wire(Some(2));
    let sent = Cobs::new();
    let received = Cobs::new();
    let counters = received.counters();
    let a = StreamChannel::start(a, sent).unwrap();
    let b = StreamChannel::start(b, received).unwrap();
    for n in 1..=5_u8 {
        a.send(&[n; 40]).unwrap();
    }
    for n in [1_u8, 3, 4, 5] {
        assert_eq!(b.receive(WAIT).unwrap(), Some(vec![n; 40]), "message {n}");
    }
    assert_eq!(
        b.receive(Some(Duration::from_millis(300))),
        Err(Failure::Timeout)
    );
    assert_eq!(counters.delivered(), 4);
    assert_eq!(counters.corrupt(), 1);
    // Still working, both ways.
    a.send(b"later").unwrap();
    assert_eq!(
        b.receive(WAIT).unwrap().as_deref(),
        Some(b"later".as_slice())
    );
    b.send(b"back").unwrap();
    assert_eq!(
        a.receive(WAIT).unwrap().as_deref(),
        Some(b"back".as_slice())
    );
}

#[test]
fn a_message_over_the_channels_maximum_is_refused_at_send() {
    let (a, _b) = wire(None);
    let a = StreamChannel::start(a, Cobs::new()).unwrap();
    a.set_max_send(100);
    assert_eq!(a.send(&[0; 101]), Err(Failure::Protocol));
    assert_eq!(a.send(&[0; 100]), Ok(()));
}
