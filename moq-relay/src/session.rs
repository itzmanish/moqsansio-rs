use std::collections::{HashMap, HashSet};

use moq_protocol::error::SessionError;

use crate::types::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupState {
    AwaitingPeerSetup,
    SetupComplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoawayState {
    pub received: bool,
    pub sent: bool,
    pub new_session_uri: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestIdState {
    pub peer_next_expected: RequestId,
    pub local_next: RequestId,
    pub local_max_exclusive: RequestId,
    pub peer_max_exclusive: RequestId,
    pub sent_requests_blocked_for: Option<RequestId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackAliasState {
    pub next_outbound: TrackAlias,
}

#[derive(Debug)]
pub struct Session<S: SessionKey> {
    pub id: S,
    pub role: EndpointRole,
    pub setup: SetupState,
    pub goaway: GoawayState,
    pub request_ids: RequestIdState,
    pub aliases: TrackAliasState,
    pub published_namespaces: HashSet<NamespaceKey>,
    pub subscribed_prefixes: HashSet<NamespacePrefixKey>,
    pub inbound_aliases: HashMap<TrackAlias, TrackKey>,
    pub outbound_aliases: HashMap<TrackKey, TrackAlias>,
}

impl<S: SessionKey> Session<S> {
    pub fn new(
        id: S,
        role: EndpointRole,
        initial_local_max: RequestId,
        initial_peer_max: RequestId,
    ) -> Self {
        Self {
            id,
            role,
            setup: SetupState::AwaitingPeerSetup,
            goaway: GoawayState {
                received: false,
                sent: false,
                new_session_uri: None,
            },
            request_ids: RequestIdState {
                peer_next_expected: role.peer_first_request_id(),
                local_next: role.local_first_request_id(),
                local_max_exclusive: initial_local_max,
                peer_max_exclusive: initial_peer_max,
                sent_requests_blocked_for: None,
            },
            aliases: TrackAliasState {
                next_outbound: TrackAlias(1),
            },
            published_namespaces: HashSet::new(),
            subscribed_prefixes: HashSet::new(),
            inbound_aliases: HashMap::new(),
            outbound_aliases: HashMap::new(),
        }
    }

    pub fn validate_peer_new_request_id(&mut self, id: RequestId) -> Result<(), SessionError> {
        if !self.role.is_valid_peer_request_id(id) {
            return Err(SessionError::InvalidRequestId);
        }
        if id != self.request_ids.peer_next_expected {
            return Err(SessionError::InvalidRequestId);
        }
        if id.0 >= self.request_ids.local_max_exclusive.0 {
            return Err(SessionError::TooManyRequests);
        }
        self.request_ids.peer_next_expected = RequestId(id.0 + 2);
        Ok(())
    }

    pub fn alloc_local_request_id(&mut self) -> Option<RequestId> {
        let id = self.request_ids.local_next;
        if id.0 >= self.request_ids.peer_max_exclusive.0 {
            return None;
        }
        self.request_ids.local_next = RequestId(id.0 + 2);
        Some(id)
    }

    pub fn on_peer_max_request_id(&mut self, max_exclusive: RequestId) -> Result<(), SessionError> {
        if max_exclusive.0 <= self.request_ids.peer_max_exclusive.0 {
            return Err(SessionError::ProtocolViolation);
        }
        self.request_ids.peer_max_exclusive = max_exclusive;
        self.request_ids.sent_requests_blocked_for = None;
        Ok(())
    }

    pub fn alloc_outbound_track_alias(&mut self) -> TrackAlias {
        let a = self.aliases.next_outbound;
        self.aliases.next_outbound = TrackAlias(a.0 + 1);
        a
    }

    pub fn bind_inbound_alias(
        &mut self,
        alias: TrackAlias,
        track: TrackKey,
    ) -> Result<(), SessionError> {
        if self.inbound_aliases.contains_key(&alias) {
            return Err(SessionError::DuplicateTrackAlias);
        }
        self.inbound_aliases.insert(alias, track);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server_session(local_max: u64, peer_max: u64) -> Session<u32> {
        Session::new(
            1u32,
            EndpointRole::Server,
            RequestId(local_max),
            RequestId(peer_max),
        )
    }

    #[test]
    fn validate_peer_request_id_sequential() {
        let mut s = server_session(100, 100);
        assert!(s.validate_peer_new_request_id(RequestId(0)).is_ok());
        assert!(s.validate_peer_new_request_id(RequestId(2)).is_ok());
        assert!(s.validate_peer_new_request_id(RequestId(4)).is_ok());
    }

    #[test]
    fn validate_peer_request_id_wrong_parity() {
        let mut s = server_session(100, 100);
        assert_eq!(
            s.validate_peer_new_request_id(RequestId(1)),
            Err(SessionError::InvalidRequestId)
        );
    }

    #[test]
    fn validate_peer_request_id_skip() {
        let mut s = server_session(100, 100);
        assert!(s.validate_peer_new_request_id(RequestId(0)).is_ok());
        assert_eq!(
            s.validate_peer_new_request_id(RequestId(4)),
            Err(SessionError::InvalidRequestId)
        );
    }

    #[test]
    fn validate_peer_request_id_exceeds_max() {
        let mut s = server_session(4, 100);
        assert!(s.validate_peer_new_request_id(RequestId(0)).is_ok());
        assert!(s.validate_peer_new_request_id(RequestId(2)).is_ok());
        assert_eq!(
            s.validate_peer_new_request_id(RequestId(4)),
            Err(SessionError::TooManyRequests)
        );
    }

    #[test]
    fn alloc_local_request_id_server() {
        let mut s = server_session(100, 100);
        assert_eq!(s.alloc_local_request_id(), Some(RequestId(1)));
        assert_eq!(s.alloc_local_request_id(), Some(RequestId(3)));
        assert_eq!(s.alloc_local_request_id(), Some(RequestId(5)));
    }

    #[test]
    fn alloc_local_request_id_blocked() {
        let mut s = server_session(100, 3);
        assert_eq!(s.alloc_local_request_id(), Some(RequestId(1)));
        assert_eq!(s.alloc_local_request_id(), None);
    }

    #[test]
    fn alloc_local_request_id_unblocked_by_max() {
        let mut s = server_session(100, 3);
        assert_eq!(s.alloc_local_request_id(), Some(RequestId(1)));
        assert_eq!(s.alloc_local_request_id(), None);
        assert!(s.on_peer_max_request_id(RequestId(10)).is_ok());
        assert_eq!(s.alloc_local_request_id(), Some(RequestId(3)));
    }

    #[test]
    fn peer_max_request_id_must_increase() {
        let mut s = server_session(100, 10);
        assert_eq!(
            s.on_peer_max_request_id(RequestId(5)),
            Err(SessionError::ProtocolViolation)
        );
        assert_eq!(
            s.on_peer_max_request_id(RequestId(10)),
            Err(SessionError::ProtocolViolation)
        );
        assert!(s.on_peer_max_request_id(RequestId(20)).is_ok());
    }

    #[test]
    fn track_alias_monotonic() {
        let mut s = server_session(100, 100);
        assert_eq!(s.alloc_outbound_track_alias(), TrackAlias(1));
        assert_eq!(s.alloc_outbound_track_alias(), TrackAlias(2));
        assert_eq!(s.alloc_outbound_track_alias(), TrackAlias(3));
    }

    #[test]
    fn bind_inbound_alias_duplicate_rejected() {
        let mut s = server_session(100, 100);
        let track = TrackKey {
            namespace: moq_protocol::types::TrackNamespace::from_strings(&["a"]).unwrap(),
            name: b"v".to_vec(),
        };
        assert!(s.bind_inbound_alias(TrackAlias(1), track.clone()).is_ok());
        let track2 = TrackKey {
            namespace: moq_protocol::types::TrackNamespace::from_strings(&["b"]).unwrap(),
            name: b"v".to_vec(),
        };
        assert_eq!(
            s.bind_inbound_alias(TrackAlias(1), track2),
            Err(SessionError::DuplicateTrackAlias)
        );
    }

    #[test]
    fn client_session_ids() {
        let mut s: Session<u32> =
            Session::new(1u32, EndpointRole::Client, RequestId(100), RequestId(100));
        assert_eq!(s.alloc_local_request_id(), Some(RequestId(0)));
        assert_eq!(s.alloc_local_request_id(), Some(RequestId(2)));
        assert!(s.validate_peer_new_request_id(RequestId(1)).is_ok());
        assert!(s.validate_peer_new_request_id(RequestId(3)).is_ok());
    }
}
