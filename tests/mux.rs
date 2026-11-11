//! The versioned frame of `coqui_transport::mux` over a serial line's `Cobs` framing, through a wire
//! that damages what crosses it. No port is needed: an in-memory wire stands in for the line, and on
//! Unix a pseudo-terminal pair carries the same frames through the kernel's terminal driver.

use std::io::{Read, Write};
use std::thread;
use std::time::Duration;

use coqui_transport::mux::Mux;
use coqui_transport::{Channel, Delivery, Failure, StreamChannel, memory};
use coqui_transport_serial::Cobs;

const WAIT: Option<Duration> = Some(Duration::from_secs(20));
const SHORT: Option<Duration> = Some(Duration::from_millis(300));

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

/// Multiplexers on both ends of a damaging wire, and the receiving side's framing counters.
fn muxes(damage: Option<usize>) -> (Mux, Mux, coqui_transport_serial::Counters) {
    let (a, b) = wire(damage);
    let received = Cobs::new();
    let counters = received.counters();
    let a = Mux::start(StreamChannel::start(a, Cobs::new()).unwrap()).unwrap();
    let b = Mux::start(StreamChannel::start(b, received).unwrap()).unwrap();
    (a, b, counters)
}

#[test]
fn channels_and_datagrams_cross_a_serial_framing_intact() {
    let (a, b, counters) = muxes(None);
    let (a0, a1) = (a.channel(0).unwrap(), a.channel(1).unwrap());
    let (b0, b1) = (b.channel(0).unwrap(), b.channel(1).unwrap());
    a0.send(b"zero").unwrap();
    a1.send(&vec![0xA5; 5000]).unwrap();
    a0.datagrams().unwrap().send(b"gram").unwrap();
    assert_eq!(b0.receive(WAIT).unwrap().unwrap(), b"zero");
    assert_eq!(b1.receive(WAIT).unwrap().unwrap(), vec![0xA5; 5000]);
    let datagrams = b0.datagrams().unwrap();
    assert_eq!(datagrams.delivery(), Delivery::OverStream);
    assert_eq!(datagrams.receive(WAIT).unwrap().unwrap(), b"gram");
    assert_eq!(counters.corrupt(), 0);
    assert_eq!(b.counters().corrupt, 0);
}

#[test]
fn a_damaged_message_frame_is_found_and_ends_only_its_channel() {
    let (a, b, counters) = muxes(Some(2));
    let (a0, a1) = (a.channel(0).unwrap(), a.channel(1).unwrap());
    let (b0, b1) = (b.channel(0).unwrap(), b.channel(1).unwrap());
    for message in [b"one", b"two", b"six"] {
        a0.send(message).unwrap();
    }
    // The line dropped the damaged second frame; the channel does not hand over a stream with a hole.
    assert_eq!(b0.receive(WAIT).unwrap().unwrap(), b"one");
    assert_eq!(b0.receive(WAIT), Err(Failure::Protocol));
    assert!(b0.is_closed());
    assert_eq!(counters.corrupt(), 1);
    // The other channel is untouched, and the damaged one told its peer.
    a1.send(b"still here").unwrap();
    assert_eq!(b1.receive(WAIT).unwrap().unwrap(), b"still here");
    assert_eq!(a0.receive(WAIT), Ok(None));
}

#[test]
fn a_damaged_datagram_is_lost_and_counted_and_nothing_else_is() {
    let (a, b, counters) = muxes(Some(2));
    let a0 = a.channel(0).unwrap();
    let b0 = b.channel(0).unwrap();
    let sent = a0.datagrams().unwrap();
    for n in 0..4_u8 {
        sent.send(&[n; 20]).unwrap();
        // Written one at a time, so the second written is the second on the wire.
        thread::sleep(Duration::from_millis(50));
    }
    let received = b0.datagrams().unwrap();
    for n in [0_u8, 2, 3] {
        assert_eq!(received.receive(WAIT).unwrap().unwrap(), [n; 20]);
    }
    assert_eq!(received.receive(SHORT), Err(Failure::Timeout));
    assert_eq!(counters.corrupt(), 1);
    // The datagram count shows the hole when a later datagram arrives, here the third.
    assert_eq!(b.counters().lost_datagrams, 1);
    // The message channel was never involved.
    a0.send(b"fine").unwrap();
    assert_eq!(b0.receive(WAIT).unwrap().unwrap(), b"fine");
}

#[cfg(unix)]
#[test]
fn the_frame_crosses_a_pseudo_terminal_pair() {
    use coqui_transport_serial::Serial;

    let (x, y) = Serial::pair().unwrap();
    let a = Mux::start(StreamChannel::start(x, Cobs::new()).unwrap()).unwrap();
    let b = Mux::start(StreamChannel::start(y, Cobs::new()).unwrap()).unwrap();
    let (a0, a1) = (a.channel(0).unwrap(), a.channel(1).unwrap());
    let (b0, b1) = (b.channel(0).unwrap(), b.channel(1).unwrap());
    let large: Vec<u8> = (0..50_000).map(|n| (n % 253) as u8).collect();
    a0.send(&large).unwrap();
    a1.send(b"small").unwrap();
    a1.datagrams().unwrap().send(b"gram").unwrap();
    b0.send(b"back").unwrap();
    assert_eq!(b0.receive(WAIT).unwrap().unwrap(), large);
    assert_eq!(b1.receive(WAIT).unwrap().unwrap(), b"small");
    assert_eq!(
        b1.datagrams().unwrap().receive(WAIT).unwrap().unwrap(),
        b"gram"
    );
    assert_eq!(a0.receive(WAIT).unwrap().unwrap(), b"back");
}
