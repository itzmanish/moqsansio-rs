use crate::codec::{write_prefixed_bytes, write_varint, Cursor, Decode, Encode};
use crate::error::Result;
use crate::params::Parameters;
use crate::types::{
    decode_extensions_from_remaining, encode_extensions, KeyValuePair, ReasonPhrase, TrackNamespace,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publish {
    pub request_id: u64,
    pub track_namespace: TrackNamespace,
    pub track_name: Vec<u8>,
    pub track_alias: u64,
    pub parameters: Parameters,
    pub track_extensions: Vec<KeyValuePair>,
}

impl Encode for Publish {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        self.track_namespace.encode(buf)?;
        write_prefixed_bytes(buf, &self.track_name)?;
        write_varint(buf, self.track_alias)?;
        self.parameters.encode(buf)?;
        encode_extensions(buf, &self.track_extensions)?;
        Ok(())
    }
}

impl Decode for Publish {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        let track_namespace = TrackNamespace::decode(cursor)?;
        let track_name = cursor.read_prefixed_bytes()?.to_vec();
        let track_alias = cursor.read_varint()?;
        let parameters = Parameters::decode(cursor)?;
        let track_extensions = decode_extensions_from_remaining(cursor)?;
        Ok(Self {
            request_id,
            track_namespace,
            track_name,
            track_alias,
            parameters,
            track_extensions,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishOk {
    pub request_id: u64,
    pub parameters: Parameters,
}

impl Encode for PublishOk {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        self.parameters.encode(buf)?;
        Ok(())
    }
}

impl Decode for PublishOk {
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
pub struct PublishDone {
    pub request_id: u64,
    pub status_code: u64,
    pub stream_count: u64,
    pub error_reason: ReasonPhrase,
}

impl Encode for PublishDone {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        write_varint(buf, self.status_code)?;
        write_varint(buf, self.stream_count)?;
        self.error_reason.encode(buf)?;
        Ok(())
    }
}

impl Decode for PublishDone {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        let status_code = cursor.read_varint()?;
        let stream_count = cursor.read_varint()?;
        let error_reason = ReasonPhrase::decode(cursor)?;
        Ok(Self {
            request_id,
            status_code,
            stream_count,
            error_reason,
        })
    }
}
