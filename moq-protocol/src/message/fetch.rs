use crate::codec::{write_prefixed_bytes, write_u8, write_varint, Cursor, Decode, Encode};
use crate::error::{Error, Result};
use crate::params::Parameters;
use crate::types::{
    decode_extensions_from_remaining, encode_extensions, KeyValuePair, Location, TrackNamespace,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchType {
    Standalone {
        track_namespace: TrackNamespace,
        track_name: Vec<u8>,
        start: Location,
        end: Location,
    },
    RelativeJoining {
        joining_request_id: u64,
        joining_start: u64,
    },
    AbsoluteJoining {
        joining_request_id: u64,
        joining_start: u64,
    },
}

impl FetchType {
    fn type_code(&self) -> u64 {
        match self {
            Self::Standalone { .. } => 0x1,
            Self::RelativeJoining { .. } => 0x2,
            Self::AbsoluteJoining { .. } => 0x3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetch {
    pub request_id: u64,
    pub fetch_type: FetchType,
    pub parameters: Parameters,
}

impl Encode for Fetch {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        write_varint(buf, self.fetch_type.type_code())?;
        match &self.fetch_type {
            FetchType::Standalone {
                track_namespace,
                track_name,
                start,
                end,
            } => {
                track_namespace.encode(buf)?;
                write_prefixed_bytes(buf, track_name)?;
                start.encode(buf)?;
                end.encode(buf)?;
            }
            FetchType::RelativeJoining {
                joining_request_id,
                joining_start,
            }
            | FetchType::AbsoluteJoining {
                joining_request_id,
                joining_start,
            } => {
                write_varint(buf, *joining_request_id)?;
                write_varint(buf, *joining_start)?;
            }
        }
        self.parameters.encode(buf)?;
        Ok(())
    }
}

impl Decode for Fetch {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        let fetch_type_code = cursor.read_varint()?;
        let fetch_type = match fetch_type_code {
            0x1 => {
                let track_namespace = TrackNamespace::decode(cursor)?;
                let track_name = cursor.read_prefixed_bytes()?.to_vec();
                let start = Location::decode(cursor)?;
                let end = Location::decode(cursor)?;
                FetchType::Standalone {
                    track_namespace,
                    track_name,
                    start,
                    end,
                }
            }
            0x2 => {
                let joining_request_id = cursor.read_varint()?;
                let joining_start = cursor.read_varint()?;
                FetchType::RelativeJoining {
                    joining_request_id,
                    joining_start,
                }
            }
            0x3 => {
                let joining_request_id = cursor.read_varint()?;
                let joining_start = cursor.read_varint()?;
                FetchType::AbsoluteJoining {
                    joining_request_id,
                    joining_start,
                }
            }
            _ => {
                return Err(Error::ProtocolViolation(format!(
                    "invalid fetch type: {fetch_type_code:#x}"
                )));
            }
        };
        let parameters = Parameters::decode(cursor)?;
        Ok(Self {
            request_id,
            fetch_type,
            parameters,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchOk {
    pub request_id: u64,
    pub end_of_track: bool,
    pub end_location: Location,
    pub parameters: Parameters,
    pub track_extensions: Vec<KeyValuePair>,
}

impl Encode for FetchOk {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        write_u8(buf, if self.end_of_track { 1 } else { 0 });
        self.end_location.encode(buf)?;
        self.parameters.encode(buf)?;
        encode_extensions(buf, &self.track_extensions)?;
        Ok(())
    }
}

impl Decode for FetchOk {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        let eot_byte = cursor.read_u8()?;
        let end_of_track = match eot_byte {
            0 => false,
            1 => true,
            _ => {
                return Err(Error::ProtocolViolation(format!(
                    "FETCH_OK End Of Track must be 0 or 1, got {eot_byte}"
                )));
            }
        };
        let end_location = Location::decode(cursor)?;
        let parameters = Parameters::decode(cursor)?;
        let track_extensions = decode_extensions_from_remaining(cursor)?;
        Ok(Self {
            request_id,
            end_of_track,
            end_location,
            parameters,
            track_extensions,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchCancel {
    pub request_id: u64,
}

impl Encode for FetchCancel {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)
    }
}

impl Decode for FetchCancel {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let request_id = cursor.read_varint()?;
        Ok(Self { request_id })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackStatus {
    pub request_id: u64,
    pub track_namespace: TrackNamespace,
    pub track_name: Vec<u8>,
    pub parameters: Parameters,
}

impl Encode for TrackStatus {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.request_id)?;
        self.track_namespace.encode(buf)?;
        write_prefixed_bytes(buf, &self.track_name)?;
        self.parameters.encode(buf)?;
        Ok(())
    }
}

impl Decode for TrackStatus {
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
