//! Buffer read/write helpers and Encode/Decode traits for sans-IO protocol parsing.

use crate::error::{Error, Result};
use crate::varint::{decode_varint, encode_varint, varint_len};

// ---------------------------------------------------------------------------
// Cursor — zero-copy reader over a byte slice
// ---------------------------------------------------------------------------

/// A cursor for reading structured data from a byte slice.
///
/// Tracks the current read position and ensures bounds checking.
#[derive(Debug)]
pub struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Create a new cursor over `buf`.
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Remaining unread bytes.
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// Current read position (bytes consumed so far).
    pub fn position(&self) -> usize {
        self.pos
    }

    /// The full underlying buffer.
    pub fn buffer(&self) -> &'a [u8] {
        self.buf
    }

    /// The unread portion of the buffer.
    pub fn remaining_bytes(&self) -> &'a [u8] {
        &self.buf[self.pos..]
    }

    /// Read a single byte.
    pub fn read_u8(&mut self) -> Result<u8> {
        if self.remaining() < 1 {
            return Err(Error::BufferTooShort);
        }
        let v = self.buf[self.pos];
        self.pos += 1;
        Ok(v)
    }

    /// Read a big-endian u16 (2 bytes).
    pub fn read_u16(&mut self) -> Result<u16> {
        if self.remaining() < 2 {
            return Err(Error::BufferTooShort);
        }
        let v = u16::from_be_bytes([self.buf[self.pos], self.buf[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    /// Read a QUIC variable-length integer.
    pub fn read_varint(&mut self) -> Result<u64> {
        let (v, n) = decode_varint(&self.buf[self.pos..])?;
        self.pos += n;
        Ok(v)
    }

    /// Read exactly `len` bytes, returning a sub-slice.
    pub fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.remaining() < len {
            return Err(Error::BufferTooShort);
        }
        let slice = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    /// Read a length-prefixed byte string (varint length + bytes).
    pub fn read_prefixed_bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.read_varint()? as usize;
        self.read_bytes(len)
    }

    /// Create a sub-cursor limited to the next `len` bytes.
    /// Advances the parent cursor by `len` bytes.
    pub fn split(&mut self, len: usize) -> Result<Cursor<'a>> {
        if self.remaining() < len {
            return Err(Error::BufferTooShort);
        }
        let sub = Cursor::new(&self.buf[self.pos..self.pos + len]);
        self.pos += len;
        Ok(sub)
    }
}

// ---------------------------------------------------------------------------
// Encode / Decode traits
// ---------------------------------------------------------------------------

/// Trait for types that can be encoded to a byte buffer.
pub trait Encode {
    /// Encode this value, appending bytes to `buf`.
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()>;

    /// Convenience: encode to a new `Vec<u8>`.
    fn encode_to_vec(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.encode(&mut buf)?;
        Ok(buf)
    }
}

/// Trait for types that can be decoded from a byte cursor.
pub trait Decode: Sized {
    /// Decode a value from the cursor, advancing its position.
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self>;
}

// ---------------------------------------------------------------------------
// Buffer write helpers
// ---------------------------------------------------------------------------

/// Write a single byte.
pub fn write_u8(buf: &mut Vec<u8>, v: u8) {
    buf.push(v);
}

/// Write a big-endian u16.
pub fn write_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

/// Write a QUIC variable-length integer.
pub fn write_varint(buf: &mut Vec<u8>, v: u64) -> Result<()> {
    encode_varint(buf, v)
}

/// Write raw bytes.
pub fn write_bytes(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(bytes);
}

/// Write a length-prefixed byte string (varint length + bytes).
pub fn write_prefixed_bytes(buf: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    write_varint(buf, bytes.len() as u64)?;
    write_bytes(buf, bytes);
    Ok(())
}

/// Compute the encoded size of a varint.
pub fn encoded_varint_len(v: u64) -> usize {
    varint_len(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_read_basics() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05];
        let mut c = Cursor::new(&data);

        assert_eq!(c.remaining(), 5);
        assert_eq!(c.read_u8().unwrap(), 0x01);
        assert_eq!(c.remaining(), 4);
        assert_eq!(c.read_u16().unwrap(), 0x0203);
        assert_eq!(c.remaining(), 2);
        assert_eq!(c.read_bytes(2).unwrap(), &[0x04, 0x05]);
        assert_eq!(c.remaining(), 0);
        assert!(c.read_u8().is_err());
    }

    #[test]
    fn cursor_read_varint() {
        let mut buf = Vec::new();
        write_varint(&mut buf, 42).unwrap();
        write_varint(&mut buf, 16384).unwrap();

        let mut c = Cursor::new(&buf);
        assert_eq!(c.read_varint().unwrap(), 42);
        assert_eq!(c.read_varint().unwrap(), 16384);
        assert_eq!(c.remaining(), 0);
    }

    #[test]
    fn prefixed_bytes_roundtrip() {
        let mut buf = Vec::new();
        write_prefixed_bytes(&mut buf, b"hello").unwrap();

        let mut c = Cursor::new(&buf);
        assert_eq!(c.read_prefixed_bytes().unwrap(), b"hello");
        assert_eq!(c.remaining(), 0);
    }

    #[test]
    fn cursor_split() {
        let data = [0x01, 0x02, 0x03, 0x04];
        let mut c = Cursor::new(&data);
        let mut sub = c.split(2).unwrap();
        assert_eq!(sub.read_u8().unwrap(), 0x01);
        assert_eq!(sub.read_u8().unwrap(), 0x02);
        assert_eq!(c.read_u8().unwrap(), 0x03);
    }
}
