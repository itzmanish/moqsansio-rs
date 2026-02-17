use crate::codec::{Cursor, Decode, Encode};
use crate::error::Result;
use crate::params::Parameters;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSetup {
    pub parameters: Parameters,
}

impl Encode for ClientSetup {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        self.parameters.encode(buf)
    }
}

impl Decode for ClientSetup {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let parameters = Parameters::decode(cursor)?;
        Ok(Self { parameters })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSetup {
    pub parameters: Parameters,
}

impl Encode for ServerSetup {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        self.parameters.encode(buf)
    }
}

impl Decode for ServerSetup {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let parameters = Parameters::decode(cursor)?;
        Ok(Self { parameters })
    }
}
