use std::collections::HashMap;

use moq_protocol::params::Parameters;
use moq_protocol::types::KeyValuePair;

use crate::types::*;

// ---------------------------------------------------------------------------
// Subscription state enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownstreamSubState {
    /// Waiting for at least one upstream SUBSCRIBE_OK.
    Pending,
    /// At least one upstream leg confirmed; SUBSCRIBE_OK sent downstream.
    Established,
    /// Terminal – either UNSUBSCRIBE received, REQUEST_ERROR sent, or PUBLISH_DONE forwarded.
    Terminated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamSubState {
    /// SUBSCRIBE sent upstream, awaiting response.
    Pending,
    /// SUBSCRIBE_OK received from publisher.
    Established,
    /// Terminal – REQUEST_ERROR received, UNSUBSCRIBE sent, or PUBLISH_DONE received.
    Terminated,
}

// ---------------------------------------------------------------------------
// SubscriptionId — uniquely identifies a downstream subscription
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubscriptionId<S: SessionKey> {
    pub session: S,
    pub request_id: RequestId,
}

// ---------------------------------------------------------------------------
// DownstreamSubscription — subscriber-initiated subscription
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct DownstreamSubscription<S: SessionKey> {
    pub id: SubscriptionId<S>,
    pub state: DownstreamSubState,
    pub requested_parameters: Parameters,
    /// Whether to pass upstream's SUBSCRIBE_OK downstream (true until sent).
    pub forward_desired: bool,
    /// Track alias allocated for sending objects to this subscriber.
    pub outbound_alias: Option<TrackAlias>,
}

// ---------------------------------------------------------------------------
// UpstreamSubscriptionLeg — one per publisher session per track
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct UpstreamSubscriptionLeg<S: SessionKey> {
    pub publisher_session: S,
    /// Request ID allocated by the relay for the upstream SUBSCRIBE.
    pub request_id: RequestId,
    pub state: UpstreamSubState,
    /// Track alias assigned by the publisher (from SUBSCRIBE_OK).
    pub inbound_alias: Option<TrackAlias>,
    /// Whether this upstream leg is currently forwarding data.
    pub forward_current: bool,
}

// ---------------------------------------------------------------------------
// TrackRoute — per-track subscription aggregation
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct TrackRoute<S: SessionKey> {
    pub track: TrackKey,
    pub downstream_subs: HashMap<SubscriptionId<S>, DownstreamSubscription<S>>,
    pub upstream_subs: HashMap<S, UpstreamSubscriptionLeg<S>>,
    pub track_extensions: Option<Vec<KeyValuePair>>,
}

impl<S: SessionKey> TrackRoute<S> {
    pub fn new(track: TrackKey) -> Self {
        Self {
            track,
            downstream_subs: HashMap::new(),
            upstream_subs: HashMap::new(),
            track_extensions: None,
        }
    }

    /// Returns true if all upstream legs are in Terminated state.
    pub fn all_upstream_terminated(&self) -> bool {
        !self.upstream_subs.is_empty()
            && self
                .upstream_subs
                .values()
                .all(|leg| leg.state == UpstreamSubState::Terminated)
    }

    /// Returns true if there are no remaining downstream subscriptions
    /// that are Pending or Established.
    pub fn no_active_downstream(&self) -> bool {
        !self.downstream_subs.values().any(|ds| {
            ds.state == DownstreamSubState::Pending || ds.state == DownstreamSubState::Established
        })
    }

    /// Returns true if there is at least one Established upstream leg.
    pub fn has_established_upstream(&self) -> bool {
        self.upstream_subs
            .values()
            .any(|leg| leg.state == UpstreamSubState::Established)
    }
}
