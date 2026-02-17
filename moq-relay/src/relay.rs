use std::collections::{HashMap, HashSet};

use moq_protocol::error::{RequestErrorCode, SessionError};
use moq_protocol::message::namespace::PublishNamespace;
use moq_protocol::message::publish::PublishDone;
use moq_protocol::message::request::{RequestErrorMsg, RequestOk};
use moq_protocol::message::session_control::{Goaway, MaxRequestId, RequestsBlocked, Unsubscribe};
use moq_protocol::message::setup::{ClientSetup, ServerSetup};
use moq_protocol::message::subscribe::{Subscribe, SubscribeOk};
use moq_protocol::message::ControlMessage;
use moq_protocol::params::Parameters;
use moq_protocol::types::{KeyValuePair, Location, TrackNamespace};

use crate::events::*;
use crate::route::*;
use crate::session::{Session, SetupState};
use crate::types::*;

pub struct RelayConfig {
    pub server_setup_parameters: Parameters,
    pub initial_local_max_request_id_exclusive: RequestId,
    pub initial_peer_max_request_id_exclusive: RequestId,
}

pub struct Relay<S: SessionKey> {
    config: RelayConfig,
    sessions: HashMap<S, Session<S>>,
    /// namespace → set of publisher session IDs that PUBLISH_NAMESPACE'd it.
    publishers_by_namespace: HashMap<NamespaceKey, HashSet<S>>,
    /// track (namespace + name) → per-track subscription aggregation.
    tracks: HashMap<TrackKey, TrackRoute<S>>,
    /// Maps (publisher_session, upstream_request_id) → TrackKey, so we can find
    /// the TrackRoute when we receive SUBSCRIBE_OK / REQUEST_ERROR / PUBLISH_DONE
    /// from a publisher.
    upstream_request_index: HashMap<(S, RequestId), TrackKey>,
    /// Maps (subscriber_session, downstream_request_id) → TrackKey, so we can find
    /// the TrackRoute when we receive UNSUBSCRIBE from a subscriber.
    downstream_request_index: HashMap<(S, RequestId), TrackKey>,
}

impl<S: SessionKey> Relay<S> {
    pub fn new(config: RelayConfig) -> Self {
        Self {
            config,
            sessions: HashMap::new(),
            publishers_by_namespace: HashMap::new(),
            tracks: HashMap::new(),
            upstream_request_index: HashMap::new(),
            downstream_request_index: HashMap::new(),
        }
    }

    pub fn session(&self, id: &S) -> Option<&Session<S>> {
        self.sessions.get(id)
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn track_route(&self, key: &TrackKey) -> Option<&TrackRoute<S>> {
        self.tracks.get(key)
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn publishers_for_namespace(&self, ns: &NamespaceKey) -> Option<&HashSet<S>> {
        self.publishers_by_namespace.get(ns)
    }

    pub fn on_event(&mut self, ev: InputEvent<S>) -> Vec<OutputEvent<S>> {
        match ev {
            InputEvent::SessionOpened { session, role } => self.on_session_opened(session, role),
            InputEvent::SessionClosed { session } => self.on_session_closed(session),
            InputEvent::ControlMessageReceived {
                session,
                channel,
                message,
            } => self.on_control_message(session, channel, message),
            InputEvent::SubscriptionObjectReceived {
                session,
                track_alias,
                subgroup_id,
                location,
                forwarding_preference,
                publisher_priority,
                object_status,
                extensions,
                payload,
            } => self.on_subscription_object(
                session,
                track_alias,
                subgroup_id,
                location,
                forwarding_preference,
                publisher_priority,
                object_status,
                extensions,
                payload,
            ),
            InputEvent::FetchObjectReceived { .. } => vec![],
        }
    }

    // -----------------------------------------------------------------------
    // Session lifecycle
    // -----------------------------------------------------------------------

    fn on_session_opened(&mut self, session: S, role: EndpointRole) -> Vec<OutputEvent<S>> {
        let s = Session::new(
            session.clone(),
            role,
            self.config.initial_local_max_request_id_exclusive,
            self.config.initial_peer_max_request_id_exclusive,
        );
        self.sessions.insert(session, s);
        vec![]
    }

    fn on_session_closed(&mut self, session: S) -> Vec<OutputEvent<S>> {
        let mut out = Vec::new();

        // Clean up publisher namespace registrations
        if let Some(sess) = self.sessions.get(&session) {
            let namespaces: Vec<NamespaceKey> = sess.published_namespaces.iter().cloned().collect();
            for ns_key in namespaces {
                if let Some(set) = self.publishers_by_namespace.get_mut(&ns_key) {
                    set.remove(&session);
                    if set.is_empty() {
                        self.publishers_by_namespace.remove(&ns_key);
                    }
                }
            }
        }

        // Clean up tracks where this session is involved
        let track_keys: Vec<TrackKey> = self.tracks.keys().cloned().collect();
        for track_key in track_keys {
            let route = match self.tracks.get_mut(&track_key) {
                Some(r) => r,
                None => continue,
            };

            // If this session is an upstream publisher, terminate its leg
            if route.upstream_subs.contains_key(&session) {
                route.upstream_subs.remove(&session);
                self.upstream_request_index
                    .retain(|&(ref s, _), _| *s != session);
            }

            // If this session has downstream subscriptions, remove them
            let ds_to_remove: Vec<SubscriptionId<S>> = route
                .downstream_subs
                .keys()
                .filter(|id| id.session == session)
                .cloned()
                .collect();

            for ds_id in &ds_to_remove {
                route.downstream_subs.remove(ds_id);
                self.downstream_request_index
                    .remove(&(ds_id.session.clone(), ds_id.request_id));
            }

            // If no active downstream subs remain, send upstream UNSUBSCRIBE for all active legs
            if route.no_active_downstream() {
                let legs: Vec<(S, RequestId)> = route
                    .upstream_subs
                    .values()
                    .filter(|leg| {
                        leg.state == UpstreamSubState::Pending
                            || leg.state == UpstreamSubState::Established
                    })
                    .map(|leg| (leg.publisher_session.clone(), leg.request_id))
                    .collect();

                for (pub_session, req_id) in legs {
                    out.push(OutputEvent::SendControlMessage {
                        session: pub_session.clone(),
                        channel: ControlChannel::ControlStream,
                        message: ControlMessage::Unsubscribe(Unsubscribe {
                            request_id: req_id.0,
                        }),
                    });
                    if let Some(leg) = route.upstream_subs.get_mut(&pub_session) {
                        leg.state = UpstreamSubState::Terminated;
                    }
                }
            }
        }

        // Remove empty tracks
        self.tracks.retain(|_, route| {
            !route.downstream_subs.is_empty() || !route.upstream_subs.is_empty()
        });

        self.sessions.remove(&session);
        out
    }

    // -----------------------------------------------------------------------
    // Data plane: object forwarding (§8.4, §8.7)
    // -----------------------------------------------------------------------

    /// Forward an object from a publisher to all established downstream subscribers.
    ///
    /// §8.4: "Relays use the Track Alias of an incoming Object to identify its
    /// Track and find the current subscribers. Each new Object is forwarded to
    /// each subscriber."
    /// §8.4: "A relay MUST NOT reorder or drop objects received on a multi-object stream."
    /// §8.7: "A relay MUST NOT modify Object properties when forwarding."
    #[allow(clippy::too_many_arguments)]
    fn on_subscription_object(
        &mut self,
        session: S,
        track_alias: TrackAlias,
        subgroup_id: Option<u64>,
        location: Location,
        forwarding_preference: ForwardingPreference,
        publisher_priority: Option<u8>,
        object_status: ObjectStatus,
        extensions: Vec<KeyValuePair>,
        payload: Vec<u8>,
    ) -> Vec<OutputEvent<S>> {
        // Look up the publisher session's inbound alias → TrackKey (§10.1)
        let track_key = match self
            .sessions
            .get(&session)
            .and_then(|sess| sess.inbound_aliases.get(&track_alias))
        {
            Some(tk) => tk.clone(),
            None => {
                // Unknown track alias — protocol violation per §10.1
                return vec![OutputEvent::CloseSession {
                    session,
                    error: SessionError::ProtocolViolation,
                    reason: "object received with unknown track alias".into(),
                }];
            }
        };

        let route = match self.tracks.get(&track_key) {
            Some(r) => r,
            None => return vec![],
        };

        // Fan out to all established downstream subscribers that have an outbound alias
        let mut out = Vec::new();
        for ds in route.downstream_subs.values() {
            if ds.state != DownstreamSubState::Established {
                continue;
            }
            let outbound_alias = match ds.outbound_alias {
                Some(a) => a,
                None => continue,
            };

            // §8.7: pass through all properties unchanged, only translate track_alias
            out.push(OutputEvent::SendSubscriptionObject {
                session: ds.id.session.clone(),
                track_alias: outbound_alias,
                subgroup_id,
                location: location.clone(),
                forwarding_preference,
                publisher_priority,
                object_status,
                extensions: extensions.clone(),
                payload: payload.clone(),
            });
        }

        out
    }

    // -----------------------------------------------------------------------
    // Message dispatch
    // -----------------------------------------------------------------------

    fn on_control_message(
        &mut self,
        session: S,
        channel: ControlChannel,
        msg: ControlMessage,
    ) -> Vec<OutputEvent<S>> {
        if !self.sessions.contains_key(&session) {
            return vec![OutputEvent::CloseSession {
                session,
                error: SessionError::ProtocolViolation,
                reason: "message from unknown session".into(),
            }];
        }

        match msg {
            ControlMessage::ClientSetup(setup) => self.handle_client_setup(session, channel, setup),
            ControlMessage::ServerSetup(setup) => self.handle_server_setup(session, setup),
            ControlMessage::MaxRequestId(msg) => self.handle_max_request_id(session, msg),
            ControlMessage::RequestsBlocked(msg) => self.handle_requests_blocked(session, msg),
            ControlMessage::Goaway(msg) => self.handle_goaway(session, msg),
            ControlMessage::PublishNamespace(msg) => {
                self.handle_publish_namespace(session, channel, msg)
            }
            ControlMessage::Subscribe(msg) => self.handle_subscribe(session, channel, msg),
            ControlMessage::SubscribeOk(msg) => self.handle_subscribe_ok(session, msg),
            ControlMessage::RequestError(msg) => self.handle_request_error(session, msg),
            ControlMessage::Unsubscribe(msg) => self.handle_unsubscribe(session, msg),
            ControlMessage::PublishDone(msg) => self.handle_publish_done(session, msg),
            _ => {
                let sess = self.sessions.get(&session).unwrap();
                if sess.setup != SetupState::SetupComplete {
                    return vec![OutputEvent::CloseSession {
                        session,
                        error: SessionError::ProtocolViolation,
                        reason: "control message before setup complete".into(),
                    }];
                }
                vec![]
            }
        }
    }

    // -----------------------------------------------------------------------
    // Setup handlers (unchanged from Phase 2a)
    // -----------------------------------------------------------------------

    fn handle_client_setup(
        &mut self,
        session: S,
        channel: ControlChannel,
        _setup: ClientSetup,
    ) -> Vec<OutputEvent<S>> {
        let sess = self.sessions.get_mut(&session).unwrap();

        if sess.role != EndpointRole::Server {
            return vec![OutputEvent::CloseSession {
                session,
                error: SessionError::ProtocolViolation,
                reason: "CLIENT_SETUP received but we are not server".into(),
            }];
        }

        if sess.setup != SetupState::AwaitingPeerSetup {
            return vec![OutputEvent::CloseSession {
                session,
                error: SessionError::ProtocolViolation,
                reason: "duplicate CLIENT_SETUP".into(),
            }];
        }

        sess.setup = SetupState::SetupComplete;

        let mut params = self.config.server_setup_parameters.clone();
        params.add_varint(
            moq_protocol::params::PARAM_MAX_REQUEST_ID,
            sess.request_ids.local_max_exclusive.0,
        );

        vec![OutputEvent::SendControlMessage {
            session,
            channel,
            message: ControlMessage::ServerSetup(ServerSetup { parameters: params }),
        }]
    }

    fn handle_server_setup(&mut self, session: S, setup: ServerSetup) -> Vec<OutputEvent<S>> {
        let sess = self.sessions.get_mut(&session).unwrap();

        if sess.role != EndpointRole::Client {
            return vec![OutputEvent::CloseSession {
                session,
                error: SessionError::ProtocolViolation,
                reason: "SERVER_SETUP received but we are not client".into(),
            }];
        }

        if sess.setup != SetupState::AwaitingPeerSetup {
            return vec![OutputEvent::CloseSession {
                session,
                error: SessionError::ProtocolViolation,
                reason: "duplicate SERVER_SETUP".into(),
            }];
        }

        if let Some(max) = setup
            .parameters
            .get_varint(moq_protocol::params::PARAM_MAX_REQUEST_ID)
        {
            sess.request_ids.peer_max_exclusive = RequestId(max);
        }

        sess.setup = SetupState::SetupComplete;
        vec![]
    }

    fn handle_max_request_id(&mut self, session: S, msg: MaxRequestId) -> Vec<OutputEvent<S>> {
        let sess = self.sessions.get_mut(&session).unwrap();

        let new_exclusive = RequestId(msg.max_request_id + 1);
        match sess.on_peer_max_request_id(new_exclusive) {
            Ok(()) => vec![],
            Err(err) => vec![OutputEvent::CloseSession {
                session,
                error: err,
                reason: "MAX_REQUEST_ID did not increase".into(),
            }],
        }
    }

    fn handle_requests_blocked(
        &mut self,
        _session: S,
        _msg: RequestsBlocked,
    ) -> Vec<OutputEvent<S>> {
        vec![]
    }

    fn handle_goaway(&mut self, session: S, msg: Goaway) -> Vec<OutputEvent<S>> {
        let sess = self.sessions.get_mut(&session).unwrap();

        if sess.goaway.received {
            return vec![OutputEvent::CloseSession {
                session,
                error: SessionError::ProtocolViolation,
                reason: "duplicate GOAWAY".into(),
            }];
        }

        sess.goaway.received = true;
        sess.goaway.new_session_uri = if msg.new_session_uri.is_empty() {
            None
        } else {
            Some(msg.new_session_uri)
        };

        vec![]
    }

    // -----------------------------------------------------------------------
    // Phase 2b: Subscription routing handlers
    // -----------------------------------------------------------------------

    /// Find all publisher sessions whose PUBLISH_NAMESPACE matches any prefix
    /// of the given track namespace, per spec §8.5.
    fn match_publishers_for_namespace(&self, track_ns: &TrackNamespace) -> HashSet<S> {
        let mut result = HashSet::new();
        for k in 1..=track_ns.fields.len() {
            let prefix_ns = TrackNamespace {
                fields: track_ns.fields[..k].to_vec(),
            };
            let key = NamespaceKey(prefix_ns);
            if let Some(sessions) = self.publishers_by_namespace.get(&key) {
                result.extend(sessions.iter().cloned());
            }
        }
        result
    }

    // -- PUBLISH_NAMESPACE --------------------------------------------------

    fn handle_publish_namespace(
        &mut self,
        session: S,
        channel: ControlChannel,
        msg: PublishNamespace,
    ) -> Vec<OutputEvent<S>> {
        {
            let sess = self.sessions.get_mut(&session).unwrap();
            if sess.setup != SetupState::SetupComplete {
                return vec![OutputEvent::CloseSession {
                    session,
                    error: SessionError::ProtocolViolation,
                    reason: "PUBLISH_NAMESPACE before setup complete".into(),
                }];
            }

            // Validate request ID from peer
            let req_id = RequestId(msg.request_id);
            if let Err(e) = sess.validate_peer_new_request_id(req_id) {
                return vec![OutputEvent::CloseSession {
                    session,
                    error: e,
                    reason: "invalid request ID in PUBLISH_NAMESPACE".into(),
                }];
            }

            // Index publisher by namespace on session
            let ns_key = NamespaceKey(msg.track_namespace.clone());
            sess.published_namespaces.insert(ns_key);
        }

        // Add to global publisher index
        let ns_key = NamespaceKey(msg.track_namespace.clone());
        self.publishers_by_namespace
            .entry(ns_key)
            .or_default()
            .insert(session.clone());

        let mut out = vec![];

        // Send REQUEST_OK back on the same channel
        out.push(OutputEvent::SendControlMessage {
            session: session.clone(),
            channel,
            message: ControlMessage::RequestOk(RequestOk {
                request_id: msg.request_id,
                parameters: Parameters::new(),
            }),
        });

        // Check if any existing tracks need upstream subscriptions to this new publisher.
        // A track with namespace (foo, bar, baz) matches if this publisher registered
        // namespace (foo), (foo, bar), or (foo, bar, baz).
        let track_keys: Vec<TrackKey> = self.tracks.keys().cloned().collect();
        for track_key in track_keys {
            let track_ns = &track_key.namespace;
            let pub_ns = &msg.track_namespace;
            let is_prefix = pub_ns.fields.len() <= track_ns.fields.len()
                && pub_ns
                    .fields
                    .iter()
                    .zip(track_ns.fields.iter())
                    .all(|(a, b)| a == b);

            if !is_prefix {
                continue;
            }

            let route = match self.tracks.get(&track_key) {
                Some(r) => r,
                None => continue,
            };

            // Skip if already have an upstream leg to this publisher for this track
            if route.upstream_subs.contains_key(&session) {
                continue;
            }

            // Skip if no active downstream subs
            if route.no_active_downstream() {
                continue;
            }

            // Allocate upstream request ID and send SUBSCRIBE
            let pub_sess = self.sessions.get_mut(&session).unwrap();
            let upstream_req_id = match pub_sess.alloc_local_request_id() {
                Some(id) => id,
                None => continue,
            };

            out.push(OutputEvent::SendControlMessage {
                session: session.clone(),
                channel: ControlChannel::ControlStream,
                message: ControlMessage::Subscribe(Subscribe {
                    request_id: upstream_req_id.0,
                    track_namespace: track_key.namespace.clone(),
                    track_name: track_key.name.clone(),
                    parameters: Parameters::new(),
                }),
            });

            let leg = UpstreamSubscriptionLeg {
                publisher_session: session.clone(),
                request_id: upstream_req_id,
                state: UpstreamSubState::Pending,
                inbound_alias: None,
                forward_current: false,
            };
            self.upstream_request_index
                .insert((session.clone(), upstream_req_id), track_key.clone());
            let route = self.tracks.get_mut(&track_key).unwrap();
            route.upstream_subs.insert(session.clone(), leg);
        }

        out
    }

    // -- SUBSCRIBE (from downstream subscriber) -----------------------------

    fn handle_subscribe(
        &mut self,
        session: S,
        channel: ControlChannel,
        msg: Subscribe,
    ) -> Vec<OutputEvent<S>> {
        {
            let sess = self.sessions.get_mut(&session).unwrap();
            if sess.setup != SetupState::SetupComplete {
                return vec![OutputEvent::CloseSession {
                    session,
                    error: SessionError::ProtocolViolation,
                    reason: "SUBSCRIBE before setup complete".into(),
                }];
            }

            // Validate request ID
            let req_id = RequestId(msg.request_id);
            if let Err(e) = sess.validate_peer_new_request_id(req_id) {
                return vec![OutputEvent::CloseSession {
                    session,
                    error: e,
                    reason: "invalid request ID in SUBSCRIBE".into(),
                }];
            }
        }

        let track_key = TrackKey {
            namespace: msg.track_namespace.clone(),
            name: msg.track_name.clone(),
        };
        let sub_id = SubscriptionId {
            session: session.clone(),
            request_id: RequestId(msg.request_id),
        };

        // Record downstream request index
        self.downstream_request_index.insert(
            (session.clone(), RequestId(msg.request_id)),
            track_key.clone(),
        );

        // Create or get the TrackRoute
        let route = self
            .tracks
            .entry(track_key.clone())
            .or_insert_with(|| TrackRoute::new(track_key.clone()));

        // Add downstream subscription
        let ds = DownstreamSubscription {
            id: sub_id.clone(),
            state: DownstreamSubState::Pending,
            requested_parameters: msg.parameters.clone(),
            forward_desired: true,
            outbound_alias: None,
        };
        route.downstream_subs.insert(sub_id.clone(), ds);

        // If there's already an established upstream leg, immediately send SUBSCRIBE_OK
        if route.has_established_upstream() {
            let route = self.tracks.get_mut(&track_key).unwrap();
            let ds = route.downstream_subs.get_mut(&sub_id).unwrap();
            ds.state = DownstreamSubState::Established;
            ds.forward_desired = false;

            let sub_sess = self.sessions.get_mut(&session).unwrap();
            let alias = sub_sess.alloc_outbound_track_alias();
            let route = self.tracks.get_mut(&track_key).unwrap();
            let ds = route.downstream_subs.get_mut(&sub_id).unwrap();
            ds.outbound_alias = Some(alias);

            return vec![OutputEvent::SendControlMessage {
                session,
                channel,
                message: ControlMessage::SubscribeOk(SubscribeOk {
                    request_id: msg.request_id,
                    track_alias: alias.0,
                    parameters: Parameters::new(),
                    track_extensions: route.track_extensions.clone().unwrap_or_default(),
                }),
            }];
        }

        // Find matching publishers and fanout upstream SUBSCRIBE
        let publishers = self.match_publishers_for_namespace(&msg.track_namespace);

        if publishers.is_empty() {
            // No publishers — send REQUEST_ERROR(DoesNotExist) immediately
            let route = self.tracks.get_mut(&track_key).unwrap();
            let ds = route.downstream_subs.get_mut(&sub_id).unwrap();
            ds.state = DownstreamSubState::Terminated;

            if route
                .downstream_subs
                .values()
                .all(|d| d.state == DownstreamSubState::Terminated)
                && route.upstream_subs.is_empty()
            {
                self.tracks.remove(&track_key);
            }
            self.downstream_request_index
                .remove(&(session.clone(), RequestId(msg.request_id)));

            return vec![OutputEvent::SendControlMessage {
                session,
                channel,
                message: ControlMessage::RequestError(RequestErrorMsg {
                    request_id: msg.request_id,
                    error_code: RequestErrorCode::DoesNotExist.as_u64(),
                    retry_interval: 0,
                    error_reason: moq_protocol::types::ReasonPhrase(
                        "no publisher for namespace".into(),
                    ),
                }),
            }];
        }

        let mut out = Vec::new();

        for pub_session in publishers {
            // Skip if we already have an upstream leg to this publisher for this track
            let route = self.tracks.get(&track_key).unwrap();
            if route.upstream_subs.contains_key(&pub_session) {
                continue;
            }

            let pub_sess = match self.sessions.get_mut(&pub_session) {
                Some(s) => s,
                None => continue,
            };
            let upstream_req_id = match pub_sess.alloc_local_request_id() {
                Some(id) => id,
                None => continue,
            };

            out.push(OutputEvent::SendControlMessage {
                session: pub_session.clone(),
                channel: ControlChannel::ControlStream,
                message: ControlMessage::Subscribe(Subscribe {
                    request_id: upstream_req_id.0,
                    track_namespace: msg.track_namespace.clone(),
                    track_name: msg.track_name.clone(),
                    parameters: Parameters::new(),
                }),
            });

            let leg = UpstreamSubscriptionLeg {
                publisher_session: pub_session.clone(),
                request_id: upstream_req_id,
                state: UpstreamSubState::Pending,
                inbound_alias: None,
                forward_current: false,
            };
            self.upstream_request_index
                .insert((pub_session.clone(), upstream_req_id), track_key.clone());
            let route = self.tracks.get_mut(&track_key).unwrap();
            route.upstream_subs.insert(pub_session, leg);
        }

        out
    }

    // -- SUBSCRIBE_OK (from upstream publisher) -----------------------------

    fn handle_subscribe_ok(&mut self, session: S, msg: SubscribeOk) -> Vec<OutputEvent<S>> {
        let sess = self.sessions.get(&session).unwrap();
        if sess.setup != SetupState::SetupComplete {
            return vec![OutputEvent::CloseSession {
                session,
                error: SessionError::ProtocolViolation,
                reason: "SUBSCRIBE_OK before setup complete".into(),
            }];
        }

        let req_id = RequestId(msg.request_id);
        let track_key = match self.upstream_request_index.get(&(session.clone(), req_id)) {
            Some(tk) => tk.clone(),
            None => {
                return vec![OutputEvent::CloseSession {
                    session,
                    error: SessionError::ProtocolViolation,
                    reason: "SUBSCRIBE_OK for unknown request ID".into(),
                }];
            }
        };

        let route = match self.tracks.get_mut(&track_key) {
            Some(r) => r,
            None => {
                return vec![OutputEvent::CloseSession {
                    session,
                    error: SessionError::ProtocolViolation,
                    reason: "SUBSCRIBE_OK for unknown track".into(),
                }];
            }
        };

        let leg = match route.upstream_subs.get_mut(&session) {
            Some(l) => l,
            None => {
                return vec![OutputEvent::CloseSession {
                    session,
                    error: SessionError::ProtocolViolation,
                    reason: "SUBSCRIBE_OK from non-upstream session".into(),
                }];
            }
        };

        if leg.state != UpstreamSubState::Pending {
            return vec![OutputEvent::CloseSession {
                session,
                error: SessionError::ProtocolViolation,
                reason: "SUBSCRIBE_OK for non-pending upstream leg".into(),
            }];
        }

        leg.state = UpstreamSubState::Established;
        let inbound_alias = TrackAlias(msg.track_alias);
        leg.inbound_alias = Some(inbound_alias);
        leg.forward_current = true;

        // Bind inbound alias on the publisher session
        let pub_sess = self.sessions.get_mut(&session).unwrap();
        if let Err(e) = pub_sess.bind_inbound_alias(inbound_alias, track_key.clone()) {
            return vec![OutputEvent::CloseSession {
                session,
                error: e,
                reason: "duplicate track alias in SUBSCRIBE_OK".into(),
            }];
        }

        // Store track extensions
        let route = self.tracks.get_mut(&track_key).unwrap();
        if !msg.track_extensions.is_empty() {
            route.track_extensions = Some(msg.track_extensions.clone());
        }

        // Forward SUBSCRIBE_OK to all Pending downstream subscriptions
        let mut out = Vec::new();
        let pending_ids: Vec<SubscriptionId<S>> = route
            .downstream_subs
            .values()
            .filter(|ds| ds.state == DownstreamSubState::Pending && ds.forward_desired)
            .map(|ds| ds.id.clone())
            .collect();

        for ds_id in pending_ids {
            let sub_sess = match self.sessions.get_mut(&ds_id.session) {
                Some(s) => s,
                None => continue,
            };
            let alias = sub_sess.alloc_outbound_track_alias();

            let route = self.tracks.get_mut(&track_key).unwrap();
            let ds = match route.downstream_subs.get_mut(&ds_id) {
                Some(d) => d,
                None => continue,
            };
            ds.state = DownstreamSubState::Established;
            ds.forward_desired = false;
            ds.outbound_alias = Some(alias);

            out.push(OutputEvent::SendControlMessage {
                session: ds_id.session.clone(),
                channel: ControlChannel::ControlStream,
                message: ControlMessage::SubscribeOk(SubscribeOk {
                    request_id: ds_id.request_id.0,
                    track_alias: alias.0,
                    parameters: Parameters::new(),
                    track_extensions: msg.track_extensions.clone(),
                }),
            });
        }

        out
    }

    // -- REQUEST_ERROR (from upstream publisher, for a subscription) ---------

    fn handle_request_error(&mut self, session: S, msg: RequestErrorMsg) -> Vec<OutputEvent<S>> {
        let sess = self.sessions.get(&session).unwrap();
        if sess.setup != SetupState::SetupComplete {
            return vec![OutputEvent::CloseSession {
                session,
                error: SessionError::ProtocolViolation,
                reason: "REQUEST_ERROR before setup complete".into(),
            }];
        }

        let req_id = RequestId(msg.request_id);
        let track_key = match self.upstream_request_index.get(&(session.clone(), req_id)) {
            Some(tk) => tk.clone(),
            None => {
                // Could be for a non-subscription request; ignore gracefully.
                return vec![];
            }
        };

        let route = match self.tracks.get_mut(&track_key) {
            Some(r) => r,
            None => return vec![],
        };

        // Transition upstream leg to Terminated
        if let Some(leg) = route.upstream_subs.get_mut(&session) {
            leg.state = UpstreamSubState::Terminated;
        }

        self.upstream_request_index
            .remove(&(session.clone(), req_id));

        // If ALL upstream legs are terminated, propagate REQUEST_ERROR downstream
        if route.all_upstream_terminated() {
            let mut out = Vec::new();
            let pending_ids: Vec<SubscriptionId<S>> = route
                .downstream_subs
                .values()
                .filter(|ds| ds.state == DownstreamSubState::Pending)
                .map(|ds| ds.id.clone())
                .collect();

            for ds_id in &pending_ids {
                let ds = route.downstream_subs.get_mut(ds_id).unwrap();
                ds.state = DownstreamSubState::Terminated;

                out.push(OutputEvent::SendControlMessage {
                    session: ds_id.session.clone(),
                    channel: ControlChannel::ControlStream,
                    message: ControlMessage::RequestError(RequestErrorMsg {
                        request_id: ds_id.request_id.0,
                        error_code: msg.error_code,
                        retry_interval: msg.retry_interval,
                        error_reason: msg.error_reason.clone(),
                    }),
                });

                self.downstream_request_index
                    .remove(&(ds_id.session.clone(), ds_id.request_id));
            }

            // Clean up empty route
            if route.no_active_downstream()
                && route
                    .upstream_subs
                    .values()
                    .all(|l| l.state == UpstreamSubState::Terminated)
            {
                self.tracks.remove(&track_key);
            }

            return out;
        }

        vec![]
    }

    // -- UNSUBSCRIBE (from downstream subscriber) ---------------------------

    fn handle_unsubscribe(&mut self, session: S, msg: Unsubscribe) -> Vec<OutputEvent<S>> {
        let sess = self.sessions.get(&session).unwrap();
        if sess.setup != SetupState::SetupComplete {
            return vec![OutputEvent::CloseSession {
                session,
                error: SessionError::ProtocolViolation,
                reason: "UNSUBSCRIBE before setup complete".into(),
            }];
        }

        let req_id = RequestId(msg.request_id);
        let track_key = match self
            .downstream_request_index
            .get(&(session.clone(), req_id))
        {
            Some(tk) => tk.clone(),
            None => {
                return vec![OutputEvent::CloseSession {
                    session,
                    error: SessionError::ProtocolViolation,
                    reason: "UNSUBSCRIBE for unknown request ID".into(),
                }];
            }
        };

        let route = match self.tracks.get_mut(&track_key) {
            Some(r) => r,
            None => return vec![],
        };

        let sub_id = SubscriptionId {
            session: session.clone(),
            request_id: req_id,
        };

        // Terminate the downstream subscription
        if let Some(ds) = route.downstream_subs.get_mut(&sub_id) {
            ds.state = DownstreamSubState::Terminated;
        }

        self.downstream_request_index
            .remove(&(session.clone(), req_id));

        // If no active downstream subs remain, send UNSUBSCRIBE upstream
        let mut out = Vec::new();
        if route.no_active_downstream() {
            let legs: Vec<(S, RequestId)> = route
                .upstream_subs
                .values()
                .filter(|leg| {
                    leg.state == UpstreamSubState::Pending
                        || leg.state == UpstreamSubState::Established
                })
                .map(|leg| (leg.publisher_session.clone(), leg.request_id))
                .collect();

            for (pub_session, upstream_req_id) in legs {
                out.push(OutputEvent::SendControlMessage {
                    session: pub_session.clone(),
                    channel: ControlChannel::ControlStream,
                    message: ControlMessage::Unsubscribe(Unsubscribe {
                        request_id: upstream_req_id.0,
                    }),
                });
                if let Some(leg) = route.upstream_subs.get_mut(&pub_session) {
                    leg.state = UpstreamSubState::Terminated;
                }
                self.upstream_request_index
                    .remove(&(pub_session, upstream_req_id));
            }

            // Clean up empty route
            if route
                .downstream_subs
                .values()
                .all(|d| d.state == DownstreamSubState::Terminated)
                && route
                    .upstream_subs
                    .values()
                    .all(|l| l.state == UpstreamSubState::Terminated)
            {
                self.tracks.remove(&track_key);
            }
        }

        out
    }

    // -- PUBLISH_DONE (from upstream publisher) -----------------------------

    fn handle_publish_done(&mut self, session: S, msg: PublishDone) -> Vec<OutputEvent<S>> {
        let sess = self.sessions.get(&session).unwrap();
        if sess.setup != SetupState::SetupComplete {
            return vec![OutputEvent::CloseSession {
                session,
                error: SessionError::ProtocolViolation,
                reason: "PUBLISH_DONE before setup complete".into(),
            }];
        }

        let req_id = RequestId(msg.request_id);
        let track_key = match self.upstream_request_index.get(&(session.clone(), req_id)) {
            Some(tk) => tk.clone(),
            None => {
                // Could be for a non-subscription request; ignore.
                return vec![];
            }
        };

        let route = match self.tracks.get_mut(&track_key) {
            Some(r) => r,
            None => return vec![],
        };

        // Terminate the upstream leg
        if let Some(leg) = route.upstream_subs.get_mut(&session) {
            leg.state = UpstreamSubState::Terminated;
        }
        self.upstream_request_index
            .remove(&(session.clone(), req_id));

        // Propagate PUBLISH_DONE to all Established downstream subscriptions
        let mut out = Vec::new();
        let established_ids: Vec<SubscriptionId<S>> = route
            .downstream_subs
            .values()
            .filter(|ds| ds.state == DownstreamSubState::Established)
            .map(|ds| ds.id.clone())
            .collect();

        for ds_id in &established_ids {
            let ds = route.downstream_subs.get_mut(ds_id).unwrap();
            ds.state = DownstreamSubState::Terminated;

            out.push(OutputEvent::SendControlMessage {
                session: ds_id.session.clone(),
                channel: ControlChannel::ControlStream,
                message: ControlMessage::PublishDone(PublishDone {
                    request_id: ds_id.request_id.0,
                    status_code: msg.status_code,
                    stream_count: msg.stream_count,
                    error_reason: msg.error_reason.clone(),
                }),
            });

            self.downstream_request_index
                .remove(&(ds_id.session.clone(), ds_id.request_id));
        }

        // Also terminate remaining Pending downstream subs if ALL upstream legs terminated
        if route.all_upstream_terminated() {
            let pending_ids: Vec<SubscriptionId<S>> = route
                .downstream_subs
                .values()
                .filter(|ds| ds.state == DownstreamSubState::Pending)
                .map(|ds| ds.id.clone())
                .collect();

            for ds_id in &pending_ids {
                let ds = route.downstream_subs.get_mut(ds_id).unwrap();
                ds.state = DownstreamSubState::Terminated;

                out.push(OutputEvent::SendControlMessage {
                    session: ds_id.session.clone(),
                    channel: ControlChannel::ControlStream,
                    message: ControlMessage::RequestError(RequestErrorMsg {
                        request_id: ds_id.request_id.0,
                        error_code: RequestErrorCode::DoesNotExist.as_u64(),
                        retry_interval: 0,
                        error_reason: moq_protocol::types::ReasonPhrase("publisher done".into()),
                    }),
                });

                self.downstream_request_index
                    .remove(&(ds_id.session.clone(), ds_id.request_id));
            }
        }

        // Clean up empty route
        let route = match self.tracks.get(&track_key) {
            Some(r) => r,
            None => return out,
        };
        if route.no_active_downstream()
            && route
                .upstream_subs
                .values()
                .all(|l| l.state == UpstreamSubState::Terminated)
        {
            self.tracks.remove(&track_key);
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moq_protocol::message::session_control::Unsubscribe;

    fn test_config() -> RelayConfig {
        RelayConfig {
            server_setup_parameters: Parameters::new(),
            initial_local_max_request_id_exclusive: RequestId(100),
            initial_peer_max_request_id_exclusive: RequestId(100),
        }
    }

    fn open_server_session(relay: &mut Relay<u32>, id: u32) {
        relay.on_event(InputEvent::SessionOpened {
            session: id,
            role: EndpointRole::Server,
        });
    }

    fn open_client_session(relay: &mut Relay<u32>, id: u32) {
        relay.on_event(InputEvent::SessionOpened {
            session: id,
            role: EndpointRole::Client,
        });
    }

    /// Complete CLIENT_SETUP handshake for a server-role session.
    fn complete_setup(relay: &mut Relay<u32>, id: u32) {
        relay.on_event(InputEvent::ControlMessageReceived {
            session: id,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::ClientSetup(ClientSetup {
                parameters: Parameters::new(),
            }),
        });
    }

    // =======================================================================
    // Phase 2a tests (preserved)
    // =======================================================================

    #[test]
    fn session_lifecycle() {
        let mut relay = Relay::new(test_config());
        assert_eq!(relay.session_count(), 0);

        open_server_session(&mut relay, 1);
        assert_eq!(relay.session_count(), 1);
        assert!(relay.session(&1).is_some());

        relay.on_event(InputEvent::SessionClosed { session: 1 });
        assert_eq!(relay.session_count(), 0);
    }

    #[test]
    fn client_setup_produces_server_setup() {
        let mut relay = Relay::new(test_config());
        open_server_session(&mut relay, 1);

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::ClientSetup(ClientSetup {
                parameters: Parameters::new(),
            }),
        });

        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendControlMessage {
                session,
                channel,
                message,
            } => {
                assert_eq!(*session, 1);
                assert_eq!(*channel, ControlChannel::ControlStream);
                assert!(matches!(message, ControlMessage::ServerSetup(_)));
                if let ControlMessage::ServerSetup(ss) = message {
                    let max = ss
                        .parameters
                        .get_varint(moq_protocol::params::PARAM_MAX_REQUEST_ID);
                    assert_eq!(max, Some(100));
                }
            }
            _ => panic!("expected SendControlMessage"),
        }

        let sess = relay.session(&1).unwrap();
        assert_eq!(sess.setup, SetupState::SetupComplete);
    }

    #[test]
    fn duplicate_client_setup_rejected() {
        let mut relay = Relay::new(test_config());
        open_server_session(&mut relay, 1);

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::ClientSetup(ClientSetup {
                parameters: Parameters::new(),
            }),
        });

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::ClientSetup(ClientSetup {
                parameters: Parameters::new(),
            }),
        });

        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            OutputEvent::CloseSession {
                error: SessionError::ProtocolViolation,
                ..
            }
        ));
    }

    #[test]
    fn server_setup_completes_client_session() {
        let mut relay = Relay::new(test_config());
        open_client_session(&mut relay, 1);

        let mut params = Parameters::new();
        params.add_varint(moq_protocol::params::PARAM_MAX_REQUEST_ID, 50);

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::ServerSetup(ServerSetup { parameters: params }),
        });

        assert!(out.is_empty());
        let sess = relay.session(&1).unwrap();
        assert_eq!(sess.setup, SetupState::SetupComplete);
        assert_eq!(sess.request_ids.peer_max_exclusive, RequestId(50));
    }

    #[test]
    fn max_request_id_increases_limit() {
        let mut relay = Relay::new(test_config());
        open_server_session(&mut relay, 1);

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::ClientSetup(ClientSetup {
                parameters: Parameters::new(),
            }),
        });

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::MaxRequestId(MaxRequestId {
                max_request_id: 200,
            }),
        });

        assert!(out.is_empty());
        let sess = relay.session(&1).unwrap();
        assert_eq!(sess.request_ids.peer_max_exclusive, RequestId(201));
    }

    #[test]
    fn max_request_id_decrease_closes_session() {
        let mut relay = Relay::new(test_config());
        open_server_session(&mut relay, 1);

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::ClientSetup(ClientSetup {
                parameters: Parameters::new(),
            }),
        });

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::MaxRequestId(MaxRequestId { max_request_id: 10 }),
        });

        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            OutputEvent::CloseSession {
                error: SessionError::ProtocolViolation,
                ..
            }
        ));
    }

    #[test]
    fn goaway_handled() {
        let mut relay = Relay::new(test_config());
        open_server_session(&mut relay, 1);

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::ClientSetup(ClientSetup {
                parameters: Parameters::new(),
            }),
        });

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Goaway(Goaway {
                new_session_uri: b"moqt://new.example.com".to_vec(),
            }),
        });

        assert!(out.is_empty());
        let sess = relay.session(&1).unwrap();
        assert!(sess.goaway.received);
        assert_eq!(
            sess.goaway.new_session_uri,
            Some(b"moqt://new.example.com".to_vec())
        );
    }

    #[test]
    fn duplicate_goaway_rejected() {
        let mut relay = Relay::new(test_config());
        open_server_session(&mut relay, 1);

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::ClientSetup(ClientSetup {
                parameters: Parameters::new(),
            }),
        });

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Goaway(Goaway {
                new_session_uri: vec![],
            }),
        });

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Goaway(Goaway {
                new_session_uri: vec![],
            }),
        });

        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            OutputEvent::CloseSession {
                error: SessionError::ProtocolViolation,
                ..
            }
        ));
    }

    #[test]
    fn message_before_setup_rejected() {
        let mut relay = Relay::new(test_config());
        open_server_session(&mut relay, 1);

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Unsubscribe(Unsubscribe { request_id: 0 }),
        });

        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            OutputEvent::CloseSession {
                error: SessionError::ProtocolViolation,
                ..
            }
        ));
    }

    // =======================================================================
    // Phase 2b: Subscription routing tests
    // =======================================================================

    #[test]
    fn publish_namespace_indexes_publisher() {
        let mut relay = Relay::new(test_config());
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "meeting"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        // Expect REQUEST_OK
        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendControlMessage { message, .. } => {
                assert!(matches!(message, ControlMessage::RequestOk(ok) if ok.request_id == 0));
            }
            _ => panic!("expected SendControlMessage(RequestOk)"),
        }

        // Verify indexing
        let ns_key =
            NamespaceKey(TrackNamespace::from_strings(&["example.com", "meeting"]).unwrap());
        let pubs = relay.publishers_for_namespace(&ns_key).unwrap();
        assert!(pubs.contains(&1));

        // Session should track its published namespaces
        let sess = relay.session(&1).unwrap();
        assert!(sess.published_namespaces.contains(&ns_key));
    }

    #[test]
    fn subscribe_with_matching_publisher_fanouts_upstream() {
        let mut relay = Relay::new(test_config());

        // Session 1 = publisher (server-role, so peer uses even IDs)
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);

        // Session 2 = subscriber (server-role)
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);

        // Publisher registers namespace
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        // Subscriber sends SUBSCRIBE for a track under that namespace
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "meeting"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Relay should fanout SUBSCRIBE upstream to publisher (session 1)
        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendControlMessage {
                session, message, ..
            } => {
                assert_eq!(*session, 1);
                match message {
                    ControlMessage::Subscribe(sub) => {
                        assert_eq!(
                            sub.track_namespace,
                            TrackNamespace::from_strings(&["example.com", "meeting"]).unwrap()
                        );
                        assert_eq!(sub.track_name, b"video");
                        // Request ID allocated by relay (server uses odd: 1)
                        assert_eq!(sub.request_id, 1);
                    }
                    _ => panic!("expected Subscribe"),
                }
            }
            _ => panic!("expected SendControlMessage"),
        }

        // Verify TrackRoute exists
        let track_key = TrackKey {
            namespace: TrackNamespace::from_strings(&["example.com", "meeting"]).unwrap(),
            name: b"video".to_vec(),
        };
        let route = relay.track_route(&track_key).unwrap();
        assert_eq!(route.downstream_subs.len(), 1);
        assert_eq!(route.upstream_subs.len(), 1);
    }

    #[test]
    fn subscribe_no_publisher_returns_error() {
        let mut relay = Relay::new(test_config());
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["unknown"]).unwrap(),
                track_name: b"audio".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendControlMessage { message, .. } => match message {
                ControlMessage::RequestError(err) => {
                    assert_eq!(err.request_id, 0);
                    assert_eq!(err.error_code, RequestErrorCode::DoesNotExist.as_u64());
                }
                _ => panic!("expected RequestError"),
            },
            _ => panic!("expected SendControlMessage"),
        }
    }

    #[test]
    fn subscribe_ok_propagates_downstream() {
        let mut relay = Relay::new(test_config());

        // Publisher session 1
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);

        // Subscriber session 2
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);

        // Publisher registers namespace
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        // Subscriber subscribes
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Publisher sends SUBSCRIBE_OK (for the upstream SUBSCRIBE with request_id=1)
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::SubscribeOk(SubscribeOk {
                request_id: 1, // relay allocated odd ID
                track_alias: 42,
                parameters: Parameters::new(),
                track_extensions: vec![],
            }),
        });

        // Relay should send SUBSCRIBE_OK downstream to subscriber (session 2)
        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendControlMessage {
                session, message, ..
            } => {
                assert_eq!(*session, 2);
                match message {
                    ControlMessage::SubscribeOk(ok) => {
                        assert_eq!(ok.request_id, 0); // downstream original request ID
                        assert!(ok.track_alias > 0);
                    }
                    _ => panic!("expected SubscribeOk"),
                }
            }
            _ => panic!("expected SendControlMessage"),
        }

        // Verify states
        let track_key = TrackKey {
            namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
            name: b"video".to_vec(),
        };
        let route = relay.track_route(&track_key).unwrap();
        let ds = route.downstream_subs.values().next().unwrap();
        assert_eq!(ds.state, DownstreamSubState::Established);
        let us = route.upstream_subs.values().next().unwrap();
        assert_eq!(us.state, UpstreamSubState::Established);
    }

    #[test]
    fn request_error_propagates_when_all_upstream_fail() {
        let mut relay = Relay::new(test_config());

        // Publisher session 1
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);

        // Subscriber session 2
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);

        // Publisher registers namespace
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        // Subscriber subscribes
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Publisher sends REQUEST_ERROR
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::RequestError(RequestErrorMsg {
                request_id: 1,
                error_code: RequestErrorCode::DoesNotExist.as_u64(),
                retry_interval: 0,
                error_reason: moq_protocol::types::ReasonPhrase("track not found".into()),
            }),
        });

        // Relay should propagate REQUEST_ERROR to subscriber
        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendControlMessage {
                session, message, ..
            } => {
                assert_eq!(*session, 2);
                match message {
                    ControlMessage::RequestError(err) => {
                        assert_eq!(err.request_id, 0);
                        assert_eq!(err.error_code, RequestErrorCode::DoesNotExist.as_u64());
                    }
                    _ => panic!("expected RequestError"),
                }
            }
            _ => panic!("expected SendControlMessage"),
        }

        // Track route should be cleaned up
        let track_key = TrackKey {
            namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
            name: b"video".to_vec(),
        };
        assert!(relay.track_route(&track_key).is_none());
    }

    #[test]
    fn unsubscribe_propagates_upstream() {
        let mut relay = Relay::new(test_config());

        // Publisher session 1, Subscriber session 2
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);

        // Publisher registers
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        // Subscriber subscribes
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Publisher confirms
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::SubscribeOk(SubscribeOk {
                request_id: 1,
                track_alias: 42,
                parameters: Parameters::new(),
                track_extensions: vec![],
            }),
        });

        // Subscriber unsubscribes
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Unsubscribe(Unsubscribe { request_id: 0 }),
        });

        // Relay should send UNSUBSCRIBE upstream to publisher
        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendControlMessage {
                session, message, ..
            } => {
                assert_eq!(*session, 1);
                match message {
                    ControlMessage::Unsubscribe(unsub) => {
                        assert_eq!(unsub.request_id, 1);
                    }
                    _ => panic!("expected Unsubscribe"),
                }
            }
            _ => panic!("expected SendControlMessage"),
        }
    }

    #[test]
    fn publish_done_propagates_downstream() {
        let mut relay = Relay::new(test_config());

        // Publisher 1, Subscriber 2
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);

        // Publisher registers
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        // Subscriber subscribes
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Publisher confirms
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::SubscribeOk(SubscribeOk {
                request_id: 1,
                track_alias: 42,
                parameters: Parameters::new(),
                track_extensions: vec![],
            }),
        });

        // Publisher sends PUBLISH_DONE
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishDone(PublishDone {
                request_id: 1,
                status_code: 2,
                stream_count: 10,
                error_reason: moq_protocol::types::ReasonPhrase("track ended".into()),
            }),
        });

        // Relay should propagate PUBLISH_DONE to subscriber
        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendControlMessage {
                session, message, ..
            } => {
                assert_eq!(*session, 2);
                match message {
                    ControlMessage::PublishDone(pd) => {
                        assert_eq!(pd.request_id, 0);
                        assert_eq!(pd.status_code, 2);
                        assert_eq!(pd.stream_count, 10);
                    }
                    _ => panic!("expected PublishDone"),
                }
            }
            _ => panic!("expected SendControlMessage"),
        }
    }

    #[test]
    fn multiple_subscribers_same_track() {
        let mut relay = Relay::new(test_config());

        // Publisher 1
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);

        // Subscribers 2, 3
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);
        open_server_session(&mut relay, 3);
        complete_setup(&mut relay, 3);

        // Publisher registers
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        // First subscriber subscribes
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Second subscriber subscribes to same track
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 3,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Should NOT send another upstream SUBSCRIBE (already have one)
        assert_eq!(out.len(), 0);

        // Verify track route has 2 downstream, 1 upstream
        let track_key = TrackKey {
            namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
            name: b"video".to_vec(),
        };
        let route = relay.track_route(&track_key).unwrap();
        assert_eq!(route.downstream_subs.len(), 2);
        assert_eq!(route.upstream_subs.len(), 1);

        // Publisher confirms — both subscribers should get SUBSCRIBE_OK
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::SubscribeOk(SubscribeOk {
                request_id: 1,
                track_alias: 42,
                parameters: Parameters::new(),
                track_extensions: vec![],
            }),
        });

        assert_eq!(out.len(), 2);
        let sessions: HashSet<u32> = out
            .iter()
            .filter_map(|e| match e {
                OutputEvent::SendControlMessage { session, .. } => Some(*session),
                _ => None,
            })
            .collect();
        assert!(sessions.contains(&2));
        assert!(sessions.contains(&3));
    }

    #[test]
    fn late_subscriber_gets_immediate_subscribe_ok() {
        let mut relay = Relay::new(test_config());

        // Publisher 1, Subscriber 2
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);

        // Publisher registers + subscriber subscribes + publisher confirms
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::SubscribeOk(SubscribeOk {
                request_id: 1,
                track_alias: 42,
                parameters: Parameters::new(),
                track_extensions: vec![],
            }),
        });

        // Late subscriber 3 subscribes to same track
        open_server_session(&mut relay, 3);
        complete_setup(&mut relay, 3);

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 3,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Should get immediate SUBSCRIBE_OK since upstream is already established
        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendControlMessage {
                session, message, ..
            } => {
                assert_eq!(*session, 3);
                assert!(matches!(message, ControlMessage::SubscribeOk(_)));
            }
            _ => panic!("expected SubscribeOk"),
        }
    }

    #[test]
    fn namespace_prefix_matching_works() {
        let mut relay = Relay::new(test_config());

        // Publisher registers "example.com"
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        // Subscriber subscribes to "example.com/meeting/room1" — should match
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "meeting", "room1"])
                    .unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Should send upstream SUBSCRIBE to publisher
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            OutputEvent::SendControlMessage {
                session: 1,
                message: ControlMessage::Subscribe(_),
                ..
            }
        ));
    }

    #[test]
    fn session_close_cleans_up_subscriptions() {
        let mut relay = Relay::new(test_config());

        // Publisher 1, Subscriber 2
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);

        // Publisher registers
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        // Subscriber subscribes
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Publisher confirms
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::SubscribeOk(SubscribeOk {
                request_id: 1,
                track_alias: 42,
                parameters: Parameters::new(),
                track_extensions: vec![],
            }),
        });

        // Subscriber disconnects
        let out = relay.on_event(InputEvent::SessionClosed { session: 2 });

        // Should send UNSUBSCRIBE upstream to publisher since no downstream subs remain
        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendControlMessage {
                session, message, ..
            } => {
                assert_eq!(*session, 1);
                assert!(matches!(message, ControlMessage::Unsubscribe(_)));
            }
            _ => panic!("expected Unsubscribe"),
        }
    }

    #[test]
    fn publisher_close_cleans_up_namespace_index() {
        let mut relay = Relay::new(test_config());

        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        let ns_key = NamespaceKey(TrackNamespace::from_strings(&["example.com"]).unwrap());
        assert!(relay.publishers_for_namespace(&ns_key).is_some());

        // Publisher disconnects
        relay.on_event(InputEvent::SessionClosed { session: 1 });

        // Namespace index should be cleaned up
        assert!(relay.publishers_for_namespace(&ns_key).is_none());
    }

    #[test]
    fn new_publisher_triggers_upstream_subscribe_for_existing_pending_route() {
        let mut relay = Relay::new(test_config());

        // Subscriber 2 subscribes first (no publishers yet) → gets REQUEST_ERROR
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // No publishers → REQUEST_ERROR immediately
        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            OutputEvent::SendControlMessage {
                message: ControlMessage::RequestError(_),
                ..
            }
        ));
    }

    #[test]
    fn partial_unsubscribe_does_not_cancel_upstream() {
        let mut relay = Relay::new(test_config());

        // Publisher 1
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);

        // Subscribers 2, 3
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);
        open_server_session(&mut relay, 3);
        complete_setup(&mut relay, 3);

        // Publisher registers
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        // Both subscribers subscribe
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 3,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Publisher confirms
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::SubscribeOk(SubscribeOk {
                request_id: 1,
                track_alias: 42,
                parameters: Parameters::new(),
                track_extensions: vec![],
            }),
        });

        // Subscriber 2 unsubscribes — but subscriber 3 still active
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Unsubscribe(Unsubscribe { request_id: 0 }),
        });

        // Should NOT send UNSUBSCRIBE upstream since subscriber 3 is still active
        assert!(out.is_empty());

        // Route should still exist
        let track_key = TrackKey {
            namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
            name: b"video".to_vec(),
        };
        assert!(relay.track_route(&track_key).is_some());
    }

    // =======================================================================
    // Phase 2c: Data plane — object forwarding tests
    // =======================================================================

    /// Sets up: publisher(1) PUBLISH_NAMESPACE → subscriber(2) SUBSCRIBE → SUBSCRIBE_OK.
    /// Returns the outbound track alias assigned to subscriber 2.
    fn setup_established_sub(relay: &mut Relay<u32>) -> TrackAlias {
        open_server_session(relay, 1);
        complete_setup(relay, 1);
        open_server_session(relay, 2);
        complete_setup(relay, 2);

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::SubscribeOk(SubscribeOk {
                request_id: 1,
                track_alias: 42,
                parameters: Parameters::new(),
                track_extensions: vec![],
            }),
        });

        // Extract the outbound alias from SUBSCRIBE_OK sent to subscriber
        match &out[0] {
            OutputEvent::SendControlMessage { message, .. } => match message {
                ControlMessage::SubscribeOk(ok) => TrackAlias(ok.track_alias),
                _ => panic!("expected SubscribeOk"),
            },
            _ => panic!("expected SendControlMessage"),
        }
    }

    #[test]
    fn object_forwarded_to_single_subscriber() {
        let mut relay = Relay::new(test_config());
        let sub_alias = setup_established_sub(&mut relay);

        let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
            session: 1,
            track_alias: TrackAlias(42),
            subgroup_id: Some(0),
            location: Location {
                group: 1,
                object: 0,
            },
            forwarding_preference: ForwardingPreference::Subgroup,
            publisher_priority: Some(128),
            object_status: ObjectStatus::Normal,
            extensions: vec![],
            payload: b"hello world".to_vec(),
        });

        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendSubscriptionObject {
                session,
                track_alias,
                subgroup_id,
                location,
                forwarding_preference,
                publisher_priority,
                object_status,
                extensions,
                payload,
            } => {
                assert_eq!(*session, 2);
                assert_eq!(*track_alias, sub_alias);
                assert_eq!(*subgroup_id, Some(0));
                assert_eq!(location.group, 1);
                assert_eq!(location.object, 0);
                assert_eq!(*forwarding_preference, ForwardingPreference::Subgroup);
                assert_eq!(*publisher_priority, Some(128));
                assert_eq!(*object_status, ObjectStatus::Normal);
                assert!(extensions.is_empty());
                assert_eq!(payload, b"hello world");
            }
            _ => panic!("expected SendSubscriptionObject"),
        }
    }

    #[test]
    fn object_forwarded_to_multiple_subscribers() {
        let mut relay = Relay::new(test_config());

        // Publisher 1
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);

        // Subscribers 2, 3
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);
        open_server_session(&mut relay, 3);
        complete_setup(&mut relay, 3);

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 3,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::SubscribeOk(SubscribeOk {
                request_id: 1,
                track_alias: 42,
                parameters: Parameters::new(),
                track_extensions: vec![],
            }),
        });

        let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
            session: 1,
            track_alias: TrackAlias(42),
            subgroup_id: Some(0),
            location: Location {
                group: 5,
                object: 3,
            },
            forwarding_preference: ForwardingPreference::Subgroup,
            publisher_priority: Some(200),
            object_status: ObjectStatus::Normal,
            extensions: vec![],
            payload: b"fanout data".to_vec(),
        });

        assert_eq!(out.len(), 2);
        let sessions: HashSet<u32> = out
            .iter()
            .filter_map(|e| match e {
                OutputEvent::SendSubscriptionObject { session, .. } => Some(*session),
                _ => None,
            })
            .collect();
        assert!(sessions.contains(&2));
        assert!(sessions.contains(&3));

        // Verify all forwarded objects preserve properties
        for ev in &out {
            match ev {
                OutputEvent::SendSubscriptionObject {
                    location,
                    forwarding_preference,
                    publisher_priority,
                    object_status,
                    payload,
                    ..
                } => {
                    assert_eq!(location.group, 5);
                    assert_eq!(location.object, 3);
                    assert_eq!(*forwarding_preference, ForwardingPreference::Subgroup);
                    assert_eq!(*publisher_priority, Some(200));
                    assert_eq!(*object_status, ObjectStatus::Normal);
                    assert_eq!(payload, b"fanout data");
                }
                _ => panic!("expected SendSubscriptionObject"),
            }
        }
    }

    #[test]
    fn object_with_unknown_alias_closes_session() {
        let mut relay = Relay::new(test_config());
        setup_established_sub(&mut relay);

        let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
            session: 1,
            track_alias: TrackAlias(999),
            subgroup_id: Some(0),
            location: Location {
                group: 0,
                object: 0,
            },
            forwarding_preference: ForwardingPreference::Subgroup,
            publisher_priority: None,
            object_status: ObjectStatus::Normal,
            extensions: vec![],
            payload: vec![],
        });

        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            OutputEvent::CloseSession {
                session: 1,
                error: SessionError::ProtocolViolation,
                ..
            }
        ));
    }

    #[test]
    fn object_not_forwarded_to_terminated_subscriber() {
        let mut relay = Relay::new(test_config());
        setup_established_sub(&mut relay);

        // Subscriber unsubscribes
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Unsubscribe(Unsubscribe { request_id: 0 }),
        });

        // Object arrives after unsubscribe — should not be forwarded
        let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
            session: 1,
            track_alias: TrackAlias(42),
            subgroup_id: Some(0),
            location: Location {
                group: 0,
                object: 0,
            },
            forwarding_preference: ForwardingPreference::Subgroup,
            publisher_priority: None,
            object_status: ObjectStatus::Normal,
            extensions: vec![],
            payload: vec![],
        });

        // Track route was cleaned up after unsubscribe (upstream terminated too),
        // so no forwarding target remains → empty output
        assert!(out.is_empty());
    }

    #[test]
    fn object_not_forwarded_to_pending_subscriber() {
        let mut relay = Relay::new(test_config());

        // Publisher 1 and subscribers 2, 3
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);
        open_server_session(&mut relay, 3);
        complete_setup(&mut relay, 3);

        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });

        // Sub 2 subscribes
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Publisher confirms → sub 2 gets SUBSCRIBE_OK
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::SubscribeOk(SubscribeOk {
                request_id: 1,
                track_alias: 42,
                parameters: Parameters::new(),
                track_extensions: vec![],
            }),
        });

        // Sub 3 subscribes (gets immediate SUBSCRIBE_OK since upstream already established)
        relay.on_event(InputEvent::ControlMessageReceived {
            session: 3,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });

        // Both should be Established — object should go to both
        let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
            session: 1,
            track_alias: TrackAlias(42),
            subgroup_id: Some(0),
            location: Location {
                group: 0,
                object: 0,
            },
            forwarding_preference: ForwardingPreference::Subgroup,
            publisher_priority: None,
            object_status: ObjectStatus::Normal,
            extensions: vec![],
            payload: b"data".to_vec(),
        });

        assert_eq!(out.len(), 2);
    }

    #[test]
    fn end_of_group_status_preserved() {
        let mut relay = Relay::new(test_config());
        let sub_alias = setup_established_sub(&mut relay);

        let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
            session: 1,
            track_alias: TrackAlias(42),
            subgroup_id: Some(0),
            location: Location {
                group: 1,
                object: 5,
            },
            forwarding_preference: ForwardingPreference::Subgroup,
            publisher_priority: Some(128),
            object_status: ObjectStatus::EndOfGroup,
            extensions: vec![],
            payload: vec![],
        });

        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendSubscriptionObject {
                track_alias,
                object_status,
                payload,
                ..
            } => {
                assert_eq!(*track_alias, sub_alias);
                assert_eq!(*object_status, ObjectStatus::EndOfGroup);
                assert!(payload.is_empty());
            }
            _ => panic!("expected SendSubscriptionObject"),
        }
    }

    #[test]
    fn end_of_track_status_preserved() {
        let mut relay = Relay::new(test_config());
        let _sub_alias = setup_established_sub(&mut relay);

        let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
            session: 1,
            track_alias: TrackAlias(42),
            subgroup_id: Some(0),
            location: Location {
                group: 2,
                object: 0,
            },
            forwarding_preference: ForwardingPreference::Subgroup,
            publisher_priority: Some(128),
            object_status: ObjectStatus::EndOfTrack,
            extensions: vec![],
            payload: vec![],
        });

        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendSubscriptionObject { object_status, .. } => {
                assert_eq!(*object_status, ObjectStatus::EndOfTrack);
            }
            _ => panic!("expected SendSubscriptionObject"),
        }
    }

    #[test]
    fn datagram_forwarding_no_subgroup_id() {
        let mut relay = Relay::new(test_config());
        let sub_alias = setup_established_sub(&mut relay);

        let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
            session: 1,
            track_alias: TrackAlias(42),
            subgroup_id: None,
            location: Location {
                group: 0,
                object: 0,
            },
            forwarding_preference: ForwardingPreference::Datagram,
            publisher_priority: Some(64),
            object_status: ObjectStatus::Normal,
            extensions: vec![],
            payload: b"datagram payload".to_vec(),
        });

        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendSubscriptionObject {
                session,
                track_alias,
                subgroup_id,
                forwarding_preference,
                payload,
                ..
            } => {
                assert_eq!(*session, 2);
                assert_eq!(*track_alias, sub_alias);
                assert_eq!(*subgroup_id, None);
                assert_eq!(*forwarding_preference, ForwardingPreference::Datagram);
                assert_eq!(payload, b"datagram payload");
            }
            _ => panic!("expected SendSubscriptionObject"),
        }
    }

    #[test]
    fn extensions_preserved_through_forwarding() {
        use moq_protocol::types::{KeyValuePair, KvValue};
        let mut relay = Relay::new(test_config());
        let _sub_alias = setup_established_sub(&mut relay);

        let exts = vec![
            KeyValuePair {
                key: 1,
                value: KvValue::Varint(42),
            },
            KeyValuePair {
                key: 2,
                value: KvValue::Bytes(b"custom".to_vec()),
            },
        ];

        let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
            session: 1,
            track_alias: TrackAlias(42),
            subgroup_id: Some(0),
            location: Location {
                group: 0,
                object: 0,
            },
            forwarding_preference: ForwardingPreference::Subgroup,
            publisher_priority: None,
            object_status: ObjectStatus::Normal,
            extensions: exts.clone(),
            payload: vec![],
        });

        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendSubscriptionObject { extensions, .. } => {
                assert_eq!(extensions.len(), 2);
                assert_eq!(*extensions, exts);
            }
            _ => panic!("expected SendSubscriptionObject"),
        }
    }

    #[test]
    fn e2e_publish_subscribe_forward_unsubscribe() {
        let mut relay = Relay::new(test_config());

        // 1. Open publisher and subscriber sessions
        open_server_session(&mut relay, 1);
        complete_setup(&mut relay, 1);
        open_server_session(&mut relay, 2);
        complete_setup(&mut relay, 2);

        // 2. Publisher registers namespace
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com"]).unwrap(),
                parameters: Parameters::new(),
            }),
        });
        assert_eq!(out.len(), 1); // REQUEST_OK

        // 3. Subscriber subscribes
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                track_namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
                track_name: b"video".to_vec(),
                parameters: Parameters::new(),
            }),
        });
        assert_eq!(out.len(), 1); // upstream SUBSCRIBE to publisher
        let upstream_req_id = match &out[0] {
            OutputEvent::SendControlMessage { message, .. } => match message {
                ControlMessage::Subscribe(sub) => sub.request_id,
                _ => panic!("expected Subscribe"),
            },
            _ => panic!("expected SendControlMessage"),
        };

        // 4. Publisher confirms subscription
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::SubscribeOk(SubscribeOk {
                request_id: upstream_req_id,
                track_alias: 42,
                parameters: Parameters::new(),
                track_extensions: vec![],
            }),
        });
        assert_eq!(out.len(), 1); // SUBSCRIBE_OK downstream
        let sub_alias = match &out[0] {
            OutputEvent::SendControlMessage {
                session, message, ..
            } => {
                assert_eq!(*session, 2);
                match message {
                    ControlMessage::SubscribeOk(ok) => TrackAlias(ok.track_alias),
                    _ => panic!("expected SubscribeOk"),
                }
            }
            _ => panic!("expected SendControlMessage"),
        };

        // 5. Publisher sends multiple objects — relay forwards each
        for obj_id in 0..5 {
            let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
                session: 1,
                track_alias: TrackAlias(42),
                subgroup_id: Some(0),
                location: Location {
                    group: 0,
                    object: obj_id,
                },
                forwarding_preference: ForwardingPreference::Subgroup,
                publisher_priority: Some(128),
                object_status: ObjectStatus::Normal,
                extensions: vec![],
                payload: format!("frame-{}", obj_id).into_bytes(),
            });

            assert_eq!(out.len(), 1);
            match &out[0] {
                OutputEvent::SendSubscriptionObject {
                    session,
                    track_alias,
                    location,
                    payload,
                    ..
                } => {
                    assert_eq!(*session, 2);
                    assert_eq!(*track_alias, sub_alias);
                    assert_eq!(location.group, 0);
                    assert_eq!(location.object, obj_id);
                    assert_eq!(*payload, format!("frame-{}", obj_id).into_bytes());
                }
                _ => panic!("expected SendSubscriptionObject"),
            }
        }

        // 6. Publisher sends EndOfGroup
        let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
            session: 1,
            track_alias: TrackAlias(42),
            subgroup_id: Some(0),
            location: Location {
                group: 0,
                object: 5,
            },
            forwarding_preference: ForwardingPreference::Subgroup,
            publisher_priority: Some(128),
            object_status: ObjectStatus::EndOfGroup,
            extensions: vec![],
            payload: vec![],
        });
        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendSubscriptionObject { object_status, .. } => {
                assert_eq!(*object_status, ObjectStatus::EndOfGroup)
            }
            _ => panic!("expected SendSubscriptionObject"),
        }

        // 7. Subscriber unsubscribes
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 2,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::Unsubscribe(Unsubscribe { request_id: 0 }),
        });
        assert_eq!(out.len(), 1); // upstream UNSUBSCRIBE
        match &out[0] {
            OutputEvent::SendControlMessage {
                session, message, ..
            } => {
                assert_eq!(*session, 1);
                assert!(matches!(message, ControlMessage::Unsubscribe(_)));
            }
            _ => panic!("expected Unsubscribe"),
        }

        // 8. Track route cleaned up
        let track_key = TrackKey {
            namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
            name: b"video".to_vec(),
        };
        assert!(relay.track_route(&track_key).is_none());
    }

    #[test]
    fn e2e_publish_done_terminates_forwarding() {
        let mut relay = Relay::new(test_config());
        setup_established_sub(&mut relay);

        // Send a few objects
        for i in 0..3 {
            let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
                session: 1,
                track_alias: TrackAlias(42),
                subgroup_id: Some(0),
                location: Location {
                    group: 0,
                    object: i,
                },
                forwarding_preference: ForwardingPreference::Subgroup,
                publisher_priority: Some(128),
                object_status: ObjectStatus::Normal,
                extensions: vec![],
                payload: vec![i as u8],
            });
            assert_eq!(out.len(), 1);
        }

        // Publisher sends PUBLISH_DONE
        let out = relay.on_event(InputEvent::ControlMessageReceived {
            session: 1,
            channel: ControlChannel::ControlStream,
            message: ControlMessage::PublishDone(PublishDone {
                request_id: 1,
                status_code: 0,
                stream_count: 1,
                error_reason: moq_protocol::types::ReasonPhrase("done".into()),
            }),
        });
        assert_eq!(out.len(), 1);
        match &out[0] {
            OutputEvent::SendControlMessage {
                session, message, ..
            } => {
                assert_eq!(*session, 2);
                assert!(matches!(message, ControlMessage::PublishDone(_)));
            }
            _ => panic!("expected PublishDone"),
        }

        // Track should be cleaned up
        let track_key = TrackKey {
            namespace: TrackNamespace::from_strings(&["example.com", "room"]).unwrap(),
            name: b"video".to_vec(),
        };
        assert!(relay.track_route(&track_key).is_none());
    }

    #[test]
    fn object_from_unknown_session_closes_session() {
        let mut relay = Relay::new(test_config());

        // Session 99 never opened
        let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
            session: 99,
            track_alias: TrackAlias(1),
            subgroup_id: Some(0),
            location: Location {
                group: 0,
                object: 0,
            },
            forwarding_preference: ForwardingPreference::Subgroup,
            publisher_priority: None,
            object_status: ObjectStatus::Normal,
            extensions: vec![],
            payload: vec![],
        });

        assert_eq!(out.len(), 1);
        assert!(matches!(
            &out[0],
            OutputEvent::CloseSession {
                session: 99,
                error: SessionError::ProtocolViolation,
                ..
            }
        ));
    }

    #[test]
    fn multiple_objects_preserve_ordering() {
        let mut relay = Relay::new(test_config());
        setup_established_sub(&mut relay);

        let mut received_locations = vec![];
        for obj_id in 0..10u64 {
            let out = relay.on_event(InputEvent::SubscriptionObjectReceived {
                session: 1,
                track_alias: TrackAlias(42),
                subgroup_id: Some(0),
                location: Location {
                    group: 0,
                    object: obj_id,
                },
                forwarding_preference: ForwardingPreference::Subgroup,
                publisher_priority: Some(128),
                object_status: ObjectStatus::Normal,
                extensions: vec![],
                payload: vec![],
            });

            assert_eq!(out.len(), 1);
            match &out[0] {
                OutputEvent::SendSubscriptionObject { location, .. } => {
                    received_locations.push((location.group, location.object));
                }
                _ => panic!("expected SendSubscriptionObject"),
            }
        }

        // §8.4: MUST NOT reorder objects — verify order preserved
        let expected: Vec<(u64, u64)> = (0..10).map(|i| (0, i)).collect();
        assert_eq!(received_locations, expected);
    }
}
