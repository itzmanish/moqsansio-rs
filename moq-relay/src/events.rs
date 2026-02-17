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
    /// Object received on a subscription stream (§10.2.1 canonical properties).
    SubscriptionObjectReceived {
        session: S,
        track_alias: TrackAlias,
        /// `None` for Datagram objects (§10.4.2).
        subgroup_id: Option<u64>,
        location: Location,
        forwarding_preference: ForwardingPreference,
        publisher_priority: Option<u8>,
        object_status: ObjectStatus,
        extensions: Vec<KeyValuePair>,
        payload: Vec<u8>,
    },
    FetchObjectReceived {
        session: S,
        fetch_request_id: RequestId,
        subgroup_id: Option<u64>,
        location: Location,
        forwarding_preference: ForwardingPreference,
        publisher_priority: Option<u8>,
        object_status: ObjectStatus,
        extensions: Vec<KeyValuePair>,
        payload: Vec<u8>,
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
    /// §8.7: relay MUST NOT modify object properties except track_alias (translated).
    SendSubscriptionObject {
        session: S,
        track_alias: TrackAlias,
        subgroup_id: Option<u64>,
        location: Location,
        forwarding_preference: ForwardingPreference,
        publisher_priority: Option<u8>,
        object_status: ObjectStatus,
        extensions: Vec<KeyValuePair>,
        payload: Vec<u8>,
    },
    SendFetchObject {
        session: S,
        downstream_fetch_request_id: RequestId,
        subgroup_id: Option<u64>,
        location: Location,
        forwarding_preference: ForwardingPreference,
        publisher_priority: Option<u8>,
        object_status: ObjectStatus,
        extensions: Vec<KeyValuePair>,
        payload: Vec<u8>,
    },
}
