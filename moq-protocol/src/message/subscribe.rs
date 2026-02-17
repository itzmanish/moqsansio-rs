use crate::codec::{write_prefixed_bytes, write_varint, Cursor, Decode, Encode};
use crate::error::Result;
use crate::params::Parameters;
use crate::types::{
    decode_extensions_from_remaining, encode_extensions, KeyValuePair, TrackNamespace,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscribe {
    pub request_id: u64,
    pub track_namespace: TrackNamespace,
    pub track_name: Vec<u8>,
    pub parameters: Parameters,
}

impl Encode for Subscribe {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        self.track_namespace.encode(buf)?;
        write_prefixed_bytes(buf, &self.track_name)?;
        self.parameters.encode(buf)?;
        Ok(())
    }
}

impl Decode for Subscribe {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        let track_namespace = TrackNamespace::decode(cursor)?;
        let track_name = cursor.read_prefixed_bytes()?.to_vec();
        let parameters = Parameters::decode(cursor)?;
        Ok(Self {
            request_id,
            track_namespace,
            track_name,
            parameters,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribeOk {
    pub request_id: u64,
    pub track_alias: u64,
    pub parameters: Parameters,
    pub track_extensions: Vec<KeyValuePair>,
}

impl Encode for SubscribeOk {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        write_varint(buf, self.track_alias)?;
        self.parameters.encode(buf)?;
        encode_extensions(buf, &self.track_extensions)?;
        Ok(())
    }
}

impl Decode for SubscribeOk {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        let track_alias = cursor.read_varint()?;
        let parameters = Parameters::decode(cursor)?;
        let track_extensions = decode_extensions_from_remaining(cursor)?;
        Ok(Self {
            request_id,
            track_alias,
            parameters,
            track_extensions,
        })
    }
}
