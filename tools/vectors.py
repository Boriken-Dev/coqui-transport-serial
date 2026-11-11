"""Writes tests/vectors/cobs_crc32c.json: the frame bytes of fixed messages, made here in Python with a COBS
and a CRC-32C written from their definitions, so the Rust codec is checked against a second implementation.

    python tools/vectors.py > tests/vectors/cobs_crc32c.json

CRC-32C: polynomial 0x1EDC6F41 reflected (0x82F63B78), initial value and final xor 0xFFFFFFFF; its check value for
b"123456789" is 0xE3069283.
"""

import json
import sys


def crc32c(data: bytes) -> int:
    crc = 0xFFFFFFFF
    for byte in data:
        crc ^= byte
        for _ in range(8):
            crc = (crc >> 1) ^ (0x82F63B78 if crc & 1 else 0)
    return crc ^ 0xFFFFFFFF


def cobs(data: bytes, trailing_after_full: bool = False) -> bytes:
    """COBS: each run of up to 254 non-zero bytes is led by a code byte one above its length, 0xFF for a full run
    (no zero follows it); a zero ends a shorter run. Data that ends on a full run gets a trailing 0x01 block only when
    `trailing_after_full`: both forms decode alike, and the minimal one is what the Rust crate writes."""
    out = bytearray()
    run = bytearray()
    full = False
    for byte in data:
        full = False
        if byte == 0:
            out.append(len(run) + 1)
            out += run
            run = bytearray()
        else:
            run.append(byte)
            if len(run) == 254:
                out.append(0xFF)
                out += run
                run = bytearray()
                full = True
    if not full or trailing_after_full:
        out.append(len(run) + 1)
        out += run
    return bytes(out)


def frame(message: bytes) -> bytes:
    body = message + crc32c(message).to_bytes(4, "little")
    return b"\x00" + cobs(body) + b"\x00"


def ends_on_a_full_run() -> bytes:
    """A message of 250 non-zero bytes whose CRC has no zero byte either: 254 non-zero bytes, one full COBS run."""
    for seed in range(1, 100000):
        message = bytes((seed * 7 + n) % 255 + 1 for n in range(250))
        if 0 not in crc32c(message).to_bytes(4, "little"):
            return message
    raise SystemExit("no such message found")


MESSAGES = {
    "check value": b"123456789",
    "empty": b"",
    "one byte": b"a",
    "a zero": b"\x00",
    "two zeros": b"\x00\x00",
    "zeros between": b"\x01\x00\x02\x00\x03",
    "every byte": bytes(range(256)),
    "253 bytes": b"a" * 253,
    "254 bytes": b"b" * 254,
    "255 bytes": b"c" * 255,
    "500 bytes": bytes((n % 255) + 1 for n in range(500)),
    "body ends on a full run": ends_on_a_full_run(),
}

if __name__ == "__main__":
    assert crc32c(b"123456789") == 0xE3069283
    vectors = [
        {"name": name, "message": message.hex(), "crc32c": f"{crc32c(message):08x}", "frame": frame(message).hex()}
        for name, message in MESSAGES.items()
    ]
    sys.stdout.buffer.write((json.dumps(vectors, indent=2) + "\n").encode())
