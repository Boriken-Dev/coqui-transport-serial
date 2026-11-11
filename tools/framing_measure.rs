//! Overhead of COBS, SLIP and HDLC byte stuffing, and what each checksum lets through a damaged COBS frame.
use crc::{CRC_16_IBM_SDLC, CRC_16_XMODEM, CRC_32_ISCSI, CRC_32_ISO_HDLC, Crc};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

fn slip(m: &[u8]) -> usize {
    1 + m.iter().map(|b| if *b == 0xC0 || *b == 0xDB { 2 } else { 1 }).sum::<usize>()
}
fn hdlc(m: &[u8]) -> usize {
    1 + m.iter().map(|b| if *b == 0x7E || *b == 0x7D { 2 } else { 1 }).sum::<usize>()
}

type Sum = (&'static str, usize, fn(&[u8]) -> Vec<u8>);

fn c16a(d: &[u8]) -> Vec<u8> { Crc::<u16>::new(&CRC_16_IBM_SDLC).checksum(d).to_le_bytes().to_vec() }
fn c16b(d: &[u8]) -> Vec<u8> { Crc::<u16>::new(&CRC_16_XMODEM).checksum(d).to_le_bytes().to_vec() }
fn c32a(d: &[u8]) -> Vec<u8> { Crc::<u32>::new(&CRC_32_ISO_HDLC).checksum(d).to_le_bytes().to_vec() }
fn c32c(d: &[u8]) -> Vec<u8> { Crc::<u32>::new(&CRC_32_ISCSI).checksum(d).to_le_bytes().to_vec() }

/// Decodes one wire frame (no delimiter) and checks the sum; the message when it passes.
fn open(wire: &[u8], sum: &Sum) -> Option<Vec<u8>> {
    if wire.is_empty() || wire.contains(&0) {
        return None;
    }
    let body = cobs::decode_vec(wire).ok()?;
    if body.len() < sum.1 {
        return None;
    }
    let (msg, tail) = body.split_at(body.len() - sum.1);
    (sum.2(msg) == tail).then(|| msg.to_vec())
}

fn main() {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    println!("== overhead in bytes added to a message (delimiter included, checksum excluded)");
    println!("{:>7} {:>10} {:>10} {:>10} {:>10} {:>10}", "size", "cobs rand", "cobs worst", "slip rand", "slip worst", "hdlc worst");
    for n in [1usize, 16, 64, 254, 255, 1024, 4096, 65536] {
        let trials = 200;
        let mut cobs_rand = 0.0;
        let mut slip_rand = 0.0;
        for _ in 0..trials {
            let m = rng.bytes(n);
            cobs_rand += (cobs::encode_vec(&m).len() + 1 - n) as f64;
            slip_rand += (slip(&m) - n) as f64;
        }
        let nz = vec![1u8; n];
        let c = vec![0xC0u8; n];
        let h = vec![0x7Eu8; n];
        println!("{:>7} {:>10.2} {:>10} {:>10.2} {:>10} {:>10}", n, cobs_rand / trials as f64, cobs::encode_vec(&nz).len() + 1 - n, slip_rand / trials as f64, slip(&c) - n, hdlc(&h) - n);
    }

    println!("\n== damaged frames: undetected / trials (a damaged frame that opens as a different message)");
    let sums: [Sum; 4] = [("crc16-ibm-sdlc", 2, c16a), ("crc16-xmodem", 2, c16b), ("crc32-iso-hdlc", 4, c32a), ("crc32c", 4, c32c)];
    let kinds = ["1 bit flip", "2 bit flips", "burst 17..48 bits", "byte dropped", "byte inserted", "random bytes"];
    for size in [16usize, 256, 1500] {
        println!("-- message size {size}");
        for sum in &sums {
            let mut line = format!("{:<15}", sum.0);
            for kind in 0..kinds.len() {
                let trials = 1_000_000u32;
                let mut bad = 0u32;
                for _ in 0..trials {
                    let msg = rng.bytes(size);
                    let mut body = msg.clone();
                    body.extend((sum.2)(&msg));
                    let mut wire = cobs::encode_vec(&body);
                    match kind {
                        0 => { let b = rng.below(wire.len() * 8); wire[b / 8] ^= 1 << (b % 8); }
                        1 => { for _ in 0..2 { let b = rng.below(wire.len() * 8); wire[b / 8] ^= 1 << (b % 8); } }
                        2 => {
                            let len = 17 + rng.below(32);
                            let start = rng.below(wire.len() * 8);
                            for k in 0..len { let b = start + k; if b < wire.len() * 8 { if k == 0 || k == len - 1 || rng.below(2) == 0 { wire[b / 8] ^= 1 << (b % 8); } } }
                        }
                        3 => { let i = rng.below(wire.len()); wire.remove(i); }
                        4 => { let i = rng.below(wire.len() + 1); wire.insert(i, rng.next() as u8); }
                        _ => { let n = wire.len(); wire = (0..n).map(|_| 1 + rng.below(255) as u8).collect(); }
                    }
                    if let Some(got) = open(&wire, sum) {
                        if got != msg { bad += 1; }
                    }
                }
                line.push_str(&format!(" {:>5}/{}", bad, "1M"));
            }
            println!("{line}");
        }
    }
    println!("columns: {}", kinds.join(" | "));
}
