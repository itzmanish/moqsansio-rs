//! QUIC variable-length integer encoding (RFC 9000 §16).
//!
//! Varints use 1, 2, 4, or 8 bytes. The two most-significant bits of
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

/// Encode a varint and append it to `buf`.
pub fn encode_varint(buf: &mut Vec<u8>, v: u64) -> Result<()> {
    if v > MAX_VARINT {
        return Err(Error::VarintOverflow);
    }
    match varint_len(v) {
        1 => {
            buf.push(v as u8);
        }
        2 => {
            buf.push(0x40 | ((v >> 8) as u8));
            buf.push(v as u8);
        }
        4 => {
            buf.push(0x80 | ((v >> 24) as u8));
            buf.push((v >> 16) as u8);
            buf.push((v >> 8) as u8);
            buf.push(v as u8);
        }
        8 => {
            buf.push(0xc0 | ((v >> 56) as u8));
            buf.push((v >> 48) as u8);
            buf.push((v >> 40) as u8);
            buf.push((v >> 32) as u8);
            buf.push((v >> 24) as u8);
            buf.push((v >> 16) as u8);
            buf.push((v >> 8) as u8);
            buf.push(v as u8);
        }
        _ => unreachable!(),
    }
    Ok(())
}

/// Decode a varint from the start of `buf`.
///
/// Returns `(value, bytes_consumed)`.
pub fn decode_varint(buf: &[u8]) -> Result<(u64, usize)> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_sizes() {
        let test_values: &[u64] = &[
            0,
            1,
            63, // 1-byte
            64,
            100,
            16383, // 2-byte
            16384,
            1_073_741_823, // 4-byte
            1_073_741_824,
            MAX_VARINT, // 8-byte
        ];
        for &v in test_values {
            let mut buf = Vec::new();
            encode_varint(&mut buf, v).unwrap();
            let expected_len = varint_len(v);
            assert_eq!(buf.len(), expected_len, "encoding length for {v}");
            let (decoded, consumed) = decode_varint(&buf).unwrap();
            assert_eq!(decoded, v, "roundtrip for {v}");
            assert_eq!(consumed, expected_len);
        }
    }

    #[test]
    fn overflow_rejected() {
        let mut buf = Vec::new();
        assert!(matches!(
            encode_varint(&mut buf, MAX_VARINT + 1),
            Err(Error::VarintOverflow)
        ));
    }

    #[test]
    fn short_buffer() {
        assert!(matches!(decode_varint(&[]), Err(Error::BufferTooShort)));
        assert!(matches!(decode_varint(&[0x40]), Err(Error::BufferTooShort)));
    }

    #[test]
    fn rfc_test_vectors() {
        let (v, n) = decode_varint(&[0xc2, 0x19, 0x7c, 0x5e, 0xff, 0x14, 0xe8, 0x8c]).unwrap();
        assert_eq!(v, 151_288_809_941_952_652);
        assert_eq!(n, 8);

        let (v, n) = decode_varint(&[0x9d, 0x7f, 0x3e, 0x7d]).unwrap();
        assert_eq!(v, 494_878_333);
        assert_eq!(n, 4);

        let (v, n) = decode_varint(&[0x7b, 0xbd]).unwrap();
        assert_eq!(v, 15293);
        assert_eq!(n, 2);

        let (v, n) = decode_varint(&[0x25]).unwrap();
        assert_eq!(v, 37);
        assert_eq!(n, 1);
    }
}
