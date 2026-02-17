use crate::codec::{write_varint, Cursor, Decode, Encode};
use crate::error::Result;
use crate::params::Parameters;
use crate::types::ReasonPhrase;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestOk {
    pub request_id: u64,
    pub parameters: Parameters,
}

impl Encode for RequestOk {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        self.parameters.encode(buf)?;
        Ok(())
    }
}

impl Decode for RequestOk {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        let parameters = Parameters::decode(cursor)?;
        Ok(Self {
            request_id,
            parameters,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestErrorMsg {
    pub request_id: u64,
    pub error_code: u64,
    pub retry_interval: u64,
    pub error_reason: ReasonPhrase,
}

impl Encode for RequestErrorMsg {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        write_varint(buf, self.error_code)?;
        write_varint(buf, self.retry_interval)?;
        self.error_reason.encode(buf)?;
        Ok(())
    }
}

impl Decode for RequestErrorMsg {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        let error_code = cursor.read_varint()?;
        let retry_interval = cursor.read_varint()?;
        let error_reason = ReasonPhrase::decode(cursor)?;
        Ok(Self {
            request_id,
            error_code,
            retry_interval,
            error_reason,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestUpdate {
    pub request_id: u64,
    pub existing_request_id: u64,
    pub parameters: Parameters,
}

impl Encode for RequestUpdate {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        write_varint(buf, self.existing_request_id)?;
        self.parameters.encode(buf)?;
        Ok(())
    }
}

impl Decode for RequestUpdate {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        let existing_request_id = cursor.read_varint()?;
        let parameters = Parameters::decode(cursor)?;
        Ok(Self {
            request_id,
            existing_request_id,
            parameters,
        })
    }
}
