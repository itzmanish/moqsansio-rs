use crate::codec::{write_prefixed_bytes, write_varint, Cursor, Decode, Encode};
use crate::error::{Error, Result};

const MAX_GOAWAY_URI_BYTES: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Goaway {
    pub new_session_uri: Vec<u8>,
}

impl Encode for Goaway {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_prefixed_bytes(buf, &self.new_session_uri)
    }
}

impl Decode for Goaway {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let len = cursor.read_varint()? as usize;
        if len > MAX_GOAWAY_URI_BYTES {
            return Err(Error::ProtocolViolation(format!(
                "GOAWAY URI length {len} exceeds max {MAX_GOAWAY_URI_BYTES}"
            )));
        }
        let new_session_uri = cursor.read_bytes(len)?.to_vec();
        Ok(Self { new_session_uri })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaxRequestId {
    pub max_request_id: u64,
}

impl Encode for MaxRequestId {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.max_request_id)
    }
}

impl Decode for MaxRequestId {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let max_request_id = cursor.read_varint()?;
        Ok(Self { max_request_id })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestsBlocked {
    pub maximum_request_id: u64,
}

impl Encode for RequestsBlocked {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.maximum_request_id)
    }
}

impl Decode for RequestsBlocked {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let maximum_request_id = cursor.read_varint()?;
        Ok(Self { maximum_request_id })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsubscribe {
    pub request_id: u64,
}

impl Encode for Unsubscribe {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)
    }
}

impl Decode for Unsubscribe {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        Ok(Self { request_id })
    }
}
