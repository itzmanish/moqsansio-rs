pub mod fetch;
pub mod namespace;
pub mod publish;
pub mod request;
pub mod session_control;
pub mod setup;
pub mod subscribe;

use crate::codec::{write_u16, write_varint, Cursor, Decode, Encode};
use crate::error::{Error, Result};

pub const MSG_CLIENT_SETUP: u64 = 0x20;
pub const MSG_SERVER_SETUP: u64 = 0x21;
pub const MSG_GOAWAY: u64 = 0x10;
pub const MSG_MAX_REQUEST_ID: u64 = 0x15;
pub const MSG_REQUESTS_BLOCKED: u64 = 0x1A;
pub const MSG_REQUEST_OK: u64 = 0x07;
pub const MSG_REQUEST_ERROR: u64 = 0x05;
pub const MSG_SUBSCRIBE: u64 = 0x03;
pub const MSG_SUBSCRIBE_OK: u64 = 0x04;
pub const MSG_REQUEST_UPDATE: u64 = 0x02;
pub const MSG_UNSUBSCRIBE: u64 = 0x0A;
pub const MSG_PUBLISH: u64 = 0x1D;
pub const MSG_PUBLISH_OK: u64 = 0x1E;
pub const MSG_PUBLISH_DONE: u64 = 0x0B;
pub const MSG_FETCH: u64 = 0x16;
pub const MSG_FETCH_OK: u64 = 0x18;
pub const MSG_FETCH_CANCEL: u64 = 0x17;
pub const MSG_TRACK_STATUS: u64 = 0x0D;
pub const MSG_PUBLISH_NAMESPACE: u64 = 0x06;
pub const MSG_NAMESPACE: u64 = 0x08;
pub const MSG_PUBLISH_NAMESPACE_DONE: u64 = 0x09;
pub const MSG_NAMESPACE_DONE: u64 = 0x0E;
pub const MSG_PUBLISH_NAMESPACE_CANCEL: u64 = 0x0C;
pub const MSG_SUBSCRIBE_NAMESPACE: u64 = 0x11;

use fetch::*;
use namespace::*;
use publish::*;
use request::*;
use session_control::*;
use setup::*;
use subscribe::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlMessage {
    ClientSetup(ClientSetup),
    ServerSetup(ServerSetup),
    Goaway(Goaway),
    MaxRequestId(MaxRequestId),
    RequestsBlocked(RequestsBlocked),
    RequestOk(RequestOk),
    RequestError(RequestErrorMsg),
    Subscribe(Subscribe),
    SubscribeOk(SubscribeOk),
    RequestUpdate(RequestUpdate),
    Unsubscribe(Unsubscribe),
    Publish(Publish),
    PublishOk(PublishOk),
    PublishDone(PublishDone),
    Fetch(Fetch),
    FetchOk(FetchOk),
    FetchCancel(FetchCancel),
    TrackStatus(TrackStatus),
    PublishNamespace(PublishNamespace),
    Namespace(Namespace),
    PublishNamespaceDone(PublishNamespaceDone),
    NamespaceDone(NamespaceDone),
    PublishNamespaceCancel(PublishNamespaceCancel),
    SubscribeNamespace(SubscribeNamespace),
}

impl ControlMessage {
    pub fn message_type(&self) -> u64 {
        match self {
            Self::ClientSetup(_) => MSG_CLIENT_SETUP,
            Self::ServerSetup(_) => MSG_SERVER_SETUP,
            Self::Goaway(_) => MSG_GOAWAY,
            Self::MaxRequestId(_) => MSG_MAX_REQUEST_ID,
            Self::RequestsBlocked(_) => MSG_REQUESTS_BLOCKED,
            Self::RequestOk(_) => MSG_REQUEST_OK,
            Self::RequestError(_) => MSG_REQUEST_ERROR,
            Self::Subscribe(_) => MSG_SUBSCRIBE,
            Self::SubscribeOk(_) => MSG_SUBSCRIBE_OK,
            Self::RequestUpdate(_) => MSG_REQUEST_UPDATE,
            Self::Unsubscribe(_) => MSG_UNSUBSCRIBE,
            Self::Publish(_) => MSG_PUBLISH,
            Self::PublishOk(_) => MSG_PUBLISH_OK,
            Self::PublishDone(_) => MSG_PUBLISH_DONE,
            Self::Fetch(_) => MSG_FETCH,
            Self::FetchOk(_) => MSG_FETCH_OK,
            Self::FetchCancel(_) => MSG_FETCH_CANCEL,
            Self::TrackStatus(_) => MSG_TRACK_STATUS,
            Self::PublishNamespace(_) => MSG_PUBLISH_NAMESPACE,
            Self::Namespace(_) => MSG_NAMESPACE,
            Self::PublishNamespaceDone(_) => MSG_PUBLISH_NAMESPACE_DONE,
            Self::NamespaceDone(_) => MSG_NAMESPACE_DONE,
            Self::PublishNamespaceCancel(_) => MSG_PUBLISH_NAMESPACE_CANCEL,
            Self::SubscribeNamespace(_) => MSG_SUBSCRIBE_NAMESPACE,
        }
    }

    fn encode_payload(&self, buf: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::ClientSetup(m) => m.encode(buf),
            Self::ServerSetup(m) => m.encode(buf),
            Self::Goaway(m) => m.encode(buf),
            Self::MaxRequestId(m) => m.encode(buf),
            Self::RequestsBlocked(m) => m.encode(buf),
            Self::RequestOk(m) => m.encode(buf),
            Self::RequestError(m) => m.encode(buf),
            Self::Subscribe(m) => m.encode(buf),
            Self::SubscribeOk(m) => m.encode(buf),
            Self::RequestUpdate(m) => m.encode(buf),
            Self::Unsubscribe(m) => m.encode(buf),
            Self::Publish(m) => m.encode(buf),
            Self::PublishOk(m) => m.encode(buf),
            Self::PublishDone(m) => m.encode(buf),
            Self::Fetch(m) => m.encode(buf),
            Self::FetchOk(m) => m.encode(buf),
            Self::FetchCancel(m) => m.encode(buf),
            Self::TrackStatus(m) => m.encode(buf),
            Self::PublishNamespace(m) => m.encode(buf),
            Self::Namespace(m) => m.encode(buf),
            Self::PublishNamespaceDone(m) => m.encode(buf),
            Self::NamespaceDone(m) => m.encode(buf),
            Self::PublishNamespaceCancel(m) => m.encode(buf),
            Self::SubscribeNamespace(m) => m.encode(buf),
        }
    }
}

pub fn encode_control_message(msg: &ControlMessage, buf: &mut Vec<u8>) -> Result<()> {
    write_varint(buf, msg.message_type())?;

    let mut payload = Vec::new();
    msg.encode_payload(&mut payload)?;

    if payload.len() > u16::MAX as usize {
        return Err(Error::MessageTooLong);
    }
    write_u16(buf, payload.len() as u16);
    buf.extend_from_slice(&payload);
    Ok(())
}

pub fn decode_control_message(buf: &[u8]) -> Result<(ControlMessage, usize)> {
    let mut outer = Cursor::new(buf);

    let msg_type = outer.read_varint()?;
    let msg_len = outer.read_u16()? as usize;

    if outer.remaining() < msg_len {
        return Err(Error::BufferTooShort);
    }

    let mut payload = outer.split(msg_len)?;
    let total_consumed = outer.position();

    let msg = match msg_type {
        MSG_CLIENT_SETUP => ControlMessage::ClientSetup(ClientSetup::decode(&mut payload)?),
        MSG_SERVER_SETUP => ControlMessage::ServerSetup(ServerSetup::decode(&mut payload)?),
        MSG_GOAWAY => ControlMessage::Goaway(Goaway::decode(&mut payload)?),
        MSG_MAX_REQUEST_ID => ControlMessage::MaxRequestId(MaxRequestId::decode(&mut payload)?),
        MSG_REQUESTS_BLOCKED => {
            ControlMessage::RequestsBlocked(RequestsBlocked::decode(&mut payload)?)
        }
        MSG_REQUEST_OK => ControlMessage::RequestOk(RequestOk::decode(&mut payload)?),
        MSG_REQUEST_ERROR => ControlMessage::RequestError(RequestErrorMsg::decode(&mut payload)?),
        MSG_SUBSCRIBE => ControlMessage::Subscribe(Subscribe::decode(&mut payload)?),
        MSG_SUBSCRIBE_OK => ControlMessage::SubscribeOk(SubscribeOk::decode(&mut payload)?),
        MSG_REQUEST_UPDATE => ControlMessage::RequestUpdate(RequestUpdate::decode(&mut payload)?),
        MSG_UNSUBSCRIBE => ControlMessage::Unsubscribe(Unsubscribe::decode(&mut payload)?),
        MSG_PUBLISH => ControlMessage::Publish(Publish::decode(&mut payload)?),
        MSG_PUBLISH_OK => ControlMessage::PublishOk(PublishOk::decode(&mut payload)?),
        MSG_PUBLISH_DONE => ControlMessage::PublishDone(PublishDone::decode(&mut payload)?),
        MSG_FETCH => ControlMessage::Fetch(Fetch::decode(&mut payload)?),
        MSG_FETCH_OK => ControlMessage::FetchOk(FetchOk::decode(&mut payload)?),
        MSG_FETCH_CANCEL => ControlMessage::FetchCancel(FetchCancel::decode(&mut payload)?),
        MSG_TRACK_STATUS => ControlMessage::TrackStatus(TrackStatus::decode(&mut payload)?),
        MSG_PUBLISH_NAMESPACE => {
            ControlMessage::PublishNamespace(PublishNamespace::decode(&mut payload)?)
        }
        MSG_NAMESPACE => ControlMessage::Namespace(Namespace::decode(&mut payload)?),
        MSG_PUBLISH_NAMESPACE_DONE => {
            ControlMessage::PublishNamespaceDone(PublishNamespaceDone::decode(&mut payload)?)
        }
        MSG_NAMESPACE_DONE => ControlMessage::NamespaceDone(NamespaceDone::decode(&mut payload)?),
        MSG_PUBLISH_NAMESPACE_CANCEL => {
            ControlMessage::PublishNamespaceCancel(PublishNamespaceCancel::decode(&mut payload)?)
        }
        MSG_SUBSCRIBE_NAMESPACE => {
            ControlMessage::SubscribeNamespace(SubscribeNamespace::decode(&mut payload)?)
        }
        _ => return Err(Error::UnknownMessageType(msg_type)),
    };

    Ok((msg, total_consumed))
}
