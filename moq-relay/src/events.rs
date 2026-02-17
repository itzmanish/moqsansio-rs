use moq_protocol::error::SessionError;
use moq_protocol::message::ControlMessage;
use moq_protocol::types::{KeyValuePair, Location};

use crate::types::*;

#[derive(Debug)]
pub enum InputEvent<S: SessionKey> {
    SessionOpened {
        session: S,
        role: EndpointRole,
    },
    SessionClosed {
        session: S,
    },
    ControlMessageReceived {
        session: S,
        channel: ControlChannel,
        message: ControlMessage,
    },
    SubscriptionObjectReceived {
        session: S,
        track_alias: TrackAlias,
        location: Location,
        publisher_priority: Option<u8>,
        extensions: Vec<KeyValuePair>,
        payload: Vec<u8>,
        end_of_group: bool,
        end_of_track: bool,
    },
    FetchObjectReceived {
        session: S,
        fetch_request_id: RequestId,
        location: Location,
        publisher_priority: Option<u8>,
        extensions: Vec<KeyValuePair>,
        payload: Vec<u8>,
        end_of_group: bool,
        end_of_track: bool,
    },
}

#[derive(Debug)]
pub enum OutputEvent<S: SessionKey> {
    SendControlMessage {
        session: S,
        channel: ControlChannel,
        message: ControlMessage,
    },
    CloseSession {
        session: S,
        error: SessionError,
        reason: String,
    },
    SendSubscriptionObject {
        session: S,
        track_alias: TrackAlias,
        location: Location,
        publisher_priority: Option<u8>,
        extensions: Vec<KeyValuePair>,
        payload: Vec<u8>,
        end_of_group: bool,
        end_of_track: bool,
    },
    SendFetchObject {
        session: S,
        downstream_fetch_request_id: RequestId,
        location: Location,
        publisher_priority: Option<u8>,
        extensions: Vec<KeyValuePair>,
        payload: Vec<u8>,
        end_of_group: bool,
        end_of_track: bool,
    },
}
