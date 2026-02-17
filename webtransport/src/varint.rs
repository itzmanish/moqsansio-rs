//! QUIC variable-length integer encoding (RFC 9000 §16).
//!
//! Varints use 1, 2, 4, or 8 bytes.  The two most-significant bits of
//! the first byte encode the length prefix:
//!
//! | 2MSB | Length | Usable Bits | Max Value              |
//! |------|--------|-------------|------------------------|
//! | 00   | 1      | 6           | 63                     |
//! | 01   | 2      | 14          | 16383                  |
//! | 10   | 4      | 30          | 1073741823             |
//! | 11   | 8      | 62          | 4611686018427387903    |

use crate::error::{Error, Result};

/// Maximum value representable as a QUIC varint.
pub const MAX_VARINT: u64 = (1 << 62) - 1;

/// Returns the number of bytes needed to encode `v` as a QUIC varint.
pub const fn varint_len(v: u64) -> usize {
    if v <= 63 {
        1
    } else if v <= 16383 {
        2
    } else if v <= 1_073_741_823 {
        4
    } else {
        8
    }
}

/// Encode `v` into `buf`, returning the number of bytes written.
///
/// Returns [`Error::BufferTooShort`] if `buf` is too small, or
/// [`Error::VarintOverflow`] if `v` exceeds [`MAX_VARINT`].
pub fn encode(v: u64, buf: &mut [u8]) -> Result<usize> {
    if v > MAX_VARINT {
        return Err(Error::VarintOverflow);
    }
    let len = varint_len(v);
    if buf.len() < len {
        return Err(Error::BufferTooShort);
    }
    match len {
        1 => {
            buf[0] = v as u8;
        }
        2 => {
            buf[0] = 0x40 | ((v >> 8) as u8);
            buf[1] = v as u8;
        }
        4 => {
            buf[0] = 0x80 | ((v >> 24) as u8);
            buf[1] = (v >> 16) as u8;
            buf[2] = (v >> 8) as u8;
            buf[3] = v as u8;
        }
        8 => {
            buf[0] = 0xc0 | ((v >> 56) as u8);
            buf[1] = (v >> 48) as u8;
            buf[2] = (v >> 40) as u8;
            buf[3] = (v >> 32) as u8;
            buf[4] = (v >> 24) as u8;
            buf[5] = (v >> 16) as u8;
            buf[6] = (v >> 8) as u8;
            buf[7] = v as u8;
        }
        _ => unreachable!(),
    }
    Ok(len)
}

/// Encode `v` into a new `Vec<u8>`.
pub fn encode_vec(v: u64) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; varint_len(v)];
    encode(v, &mut buf)?;
    Ok(buf)
}

/// Decode a varint from the start of `buf`.
///
/// Returns `(value, bytes_consumed)`.  Returns [`Error::BufferTooShort`]
/// if `buf` does not contain enough bytes.
pub fn decode(buf: &[u8]) -> Result<(u64, usize)> {
    if buf.is_empty() {
        return Err(Error::BufferTooShort);
    }
    let prefix = buf[0] >> 6;
    let len = 1usize << prefix;
    if buf.len() < len {
        return Err(Error::BufferTooShort);
    }
    let mut v = (buf[0] as u64) & 0x3f;
    for &b in &buf[1..len] {
        v = (v << 8) | (b as u64);
    }
    Ok((v, len))
}

// ---------------------------------------------------------------------------
// Incremental decoder — for parsing stream headers byte-by-byte
// ---------------------------------------------------------------------------

/// State machine for incrementally decoding a single varint.
#[derive(Debug, Clone)]
pub struct VarintDecoder {
    /// Accumulated value so far.
    value: u64,
    /// Total expected length in bytes (0 = not yet known).
    expected_len: usize,
    /// Bytes consumed so far.
    consumed: usize,
}

impl VarintDecoder {
    pub const fn new() -> Self {
        Self {
            value: 0,
            expected_len: 0,
            consumed: 0,
        }
    }

    /// Feed the next byte.  Returns `Some((value, total_bytes))` when the
    /// varint is complete, or `None` if more bytes are needed.
    pub fn feed(&mut self, byte: u8) -> Option<(u64, usize)> {
        if self.consumed == 0 {
            let prefix = byte >> 6;
            self.expected_len = 1usize << prefix;
            self.value = (byte as u64) & 0x3f;
        } else {
            self.value = (self.value << 8) | (byte as u64);
        }
        self.consumed += 1;
        if self.consumed == self.expected_len {
            Some((self.value, self.consumed))
        } else {
            None
        }
    }

    /// Reset the decoder for reuse.
    pub fn reset(&mut self) {
        self.value = 0;
        self.expected_len = 0;
        self.consumed = 0;
    }

    /// Whether the decoder has not yet received any bytes.
    pub const fn is_fresh(&self) -> bool {
        self.consumed == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_small() {
        let mut buf = [0u8; 8];
        for v in 0..=63u64 {
            let n = encode(v, &mut buf).unwrap();
            assert_eq!(n, 1);
            let (decoded, consumed) = decode(&buf[..n]).unwrap();
            assert_eq!(decoded, v);
            assert_eq!(consumed, 1);
        }
    }

    #[test]
    fn roundtrip_two_byte() {
        let mut buf = [0u8; 8];
        for &v in &[64, 100, 16383] {
            let n = encode(v, &mut buf).unwrap();
            assert_eq!(n, 2);
            let (decoded, consumed) = decode(&buf[..n]).unwrap();
            assert_eq!(decoded, v);
            assert_eq!(consumed, 2);
        }
    }

    #[test]
    fn roundtrip_four_byte() {
        let mut buf = [0u8; 8];
        let v = 1_073_741_823u64;
        let n = encode(v, &mut buf).unwrap();
        assert_eq!(n, 4);
        let (decoded, consumed) = decode(&buf[..n]).unwrap();
        assert_eq!(decoded, v);
        assert_eq!(consumed, 4);
    }

    #[test]
    fn roundtrip_eight_byte() {
        let mut buf = [0u8; 8];
        let v = MAX_VARINT;
        let n = encode(v, &mut buf).unwrap();
        assert_eq!(n, 8);
        let (decoded, consumed) = decode(&buf[..n]).unwrap();
        assert_eq!(decoded, v);
        assert_eq!(consumed, 8);
    }

    #[test]
    fn incremental_decoder() {
        let mut buf = [0u8; 8];
        let v = 16384u64;
        let n = encode(v, &mut buf).unwrap();
        let mut dec = VarintDecoder::new();
        for &b in &buf[..n - 1] {
            assert!(dec.feed(b).is_none());
        }
        let (val, len) = dec.feed(buf[n - 1]).unwrap();
        assert_eq!(val, v);
        assert_eq!(len, n);
    }

    #[test]
    fn overflow_rejected() {
        assert!(matches!(
            encode(MAX_VARINT + 1, &mut [0u8; 8]),
            Err(Error::VarintOverflow)
        ));
    }

    #[test]
    fn short_buffer() {
        assert!(matches!(decode(&[]), Err(Error::BufferTooShort)));
        assert!(matches!(decode(&[0x40]), Err(Error::BufferTooShort)));
    }

    #[test]
    fn rfc_test_vectors() {
        // RFC 9000 §A.1 examples
        let (v, n) = decode(&[0xc2, 0x19, 0x7c, 0x5e, 0xff, 0x14, 0xe8, 0x8c]).unwrap();
        assert_eq!(v, 151_288_809_941_952_652);
        assert_eq!(n, 8);

        let (v, n) = decode(&[0x9d, 0x7f, 0x3e, 0x7d]).unwrap();
        assert_eq!(v, 494_878_333);
        assert_eq!(n, 4);

        let (v, n) = decode(&[0x7b, 0xbd]).unwrap();
        assert_eq!(v, 15293);
        assert_eq!(n, 2);

        let (v, n) = decode(&[0x25]).unwrap();
        assert_eq!(v, 37);
        assert_eq!(n, 1);
    }
}
