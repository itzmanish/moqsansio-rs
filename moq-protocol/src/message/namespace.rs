use crate::codec::{write_varint, Cursor, Decode, Encode};
use crate::error::Result;
use crate::params::Parameters;
use crate::types::{ReasonPhrase, SubscribeOption, TrackNamespace, TrackNamespacePrefix};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishNamespace {
    pub request_id: u64,
    pub track_namespace: TrackNamespace,
    pub parameters: Parameters,
}

impl Encode for PublishNamespace {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        self.track_namespace.encode(buf)?;
        self.parameters.encode(buf)?;
        Ok(())
    }
}

impl Decode for PublishNamespace {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        let track_namespace = TrackNamespace::decode(cursor)?;
        let parameters = Parameters::decode(cursor)?;
        Ok(Self {
            request_id,
            track_namespace,
            parameters,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Namespace {
    pub track_namespace_suffix: TrackNamespacePrefix,
}

impl Encode for Namespace {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        self.track_namespace_suffix.encode(buf)
    }
}

impl Decode for Namespace {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let track_namespace_suffix = TrackNamespacePrefix::decode(cursor)?;
        Ok(Self {
            track_namespace_suffix,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishNamespaceDone {
    pub request_id: u64,
}

impl Encode for PublishNamespaceDone {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)
    }
}

impl Decode for PublishNamespaceDone {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        Ok(Self { request_id })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceDone {
    pub track_namespace_suffix: TrackNamespacePrefix,
}

impl Encode for NamespaceDone {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        self.track_namespace_suffix.encode(buf)
    }
}

impl Decode for NamespaceDone {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let track_namespace_suffix = TrackNamespacePrefix::decode(cursor)?;
        Ok(Self {
            track_namespace_suffix,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishNamespaceCancel {
    pub request_id: u64,
    pub error_code: u64,
    pub error_reason: ReasonPhrase,
}

impl Encode for PublishNamespaceCancel {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        write_varint(buf, self.error_code)?;
        self.error_reason.encode(buf)?;
        Ok(())
    }
}

impl Decode for PublishNamespaceCancel {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        let error_code = cursor.read_varint()?;
        let error_reason = ReasonPhrase::decode(cursor)?;
        Ok(Self {
            request_id,
            error_code,
            error_reason,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribeNamespace {
    pub request_id: u64,
    pub track_namespace_prefix: TrackNamespacePrefix,
    pub subscribe_options: SubscribeOption,
    pub parameters: Parameters,
}

impl Encode for SubscribeNamespace {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        self.track_namespace_prefix.encode(buf)?;
        write_varint(buf, self.subscribe_options.as_u64())?;
        self.parameters.encode(buf)?;
        Ok(())
    }
}

impl Decode for SubscribeNamespace {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        let track_namespace_prefix = TrackNamespacePrefix::decode(cursor)?;
        let subscribe_options = SubscribeOption::from_u64(cursor.read_varint()?)?;
        let parameters = Parameters::decode(cursor)?;
        Ok(Self {
            request_id,
            track_namespace_prefix,
            subscribe_options,
            parameters,
        })
    }
}
