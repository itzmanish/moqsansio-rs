use std::hash::{Hash, Hasher};

use moq_protocol::types::{TrackNamespace, TrackNamespacePrefix};

pub type StreamId = u64;

pub trait SessionKey: Clone + Eq + Hash + std::fmt::Debug {}
impl<T: Clone + Eq + Hash + std::fmt::Debug> SessionKey for T {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrackAlias(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackKey {
    pub namespace: TrackNamespace,
    pub name: Vec<u8>,
}

impl Hash for TrackKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.namespace.fields.hash(state);
        self.name.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceKey(pub TrackNamespace);

impl Hash for NamespaceKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.fields.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespacePrefixKey(pub TrackNamespacePrefix);

impl Hash for NamespacePrefixKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.fields.hash(state);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointRole {
    Client,
    Server,
}

impl EndpointRole {
    pub fn local_first_request_id(self) -> RequestId {
        match self {
            Self::Client => RequestId(0),
            Self::Server => RequestId(1),
        }
    }

    pub fn peer_first_request_id(self) -> RequestId {
        match self {
            Self::Client => RequestId(1),
            Self::Server => RequestId(0),
        }
    }

    pub fn is_valid_peer_request_id(self, id: RequestId) -> bool {
        let expected_lsb = match self {
            Self::Client => 1, // peer is server, uses odd
            Self::Server => 0, // peer is client, uses even
        };
        id.0 & 1 == expected_lsb
    }
}

/// Object Status per spec §10.2.1.1.
///
/// Allows the publisher to explicitly communicate that a specific range
/// of objects does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectStatus {
    /// 0x0 — Normal object. Implicit for any non-zero length object.
    /// Zero-length objects explicitly encode the Normal status.
    Normal,
    /// 0x3 — End of Group. No objects with the same Group ID and an Object ID
    /// greater than or equal to the one specified exist.
    EndOfGroup,
    /// 0x4 — End of Track. No objects with a location equal to or greater
    /// than the one specified exist.
    EndOfTrack,
}

impl ObjectStatus {
    pub fn from_u64(v: u64) -> Option<Self> {
        match v {
            0x0 => Some(Self::Normal),
            0x3 => Some(Self::EndOfGroup),
            0x4 => Some(Self::EndOfTrack),
            _ => None,
        }
    }

    pub fn as_u64(self) -> u64 {
        match self {
            Self::Normal => 0x0,
            Self::EndOfGroup => 0x3,
            Self::EndOfTrack => 0x4,
        }
    }
}

/// Object Forwarding Preference per spec §10.2.1.
///
/// An enumeration indicating how a publisher sends an object.
/// In a subscription, an Object MUST be sent according to its
/// Object Forwarding Preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardingPreference {
    /// Object is sent on a Subgroup stream (§10.4.2).
    Subgroup,
    /// Object is sent in a Datagram (§10.3).
    Datagram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlChannel {
    ControlStream,
    BidiStream(StreamId),
}

pub fn prefix_matches_namespace(prefix: &TrackNamespacePrefix, ns: &TrackNamespace) -> bool {
    if prefix.fields.len() > ns.fields.len() {
        return false;
    }
    prefix
        .fields
        .iter()
        .zip(ns.fields.iter())
        .all(|(a, b)| a == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_role_first_ids() {
        assert_eq!(EndpointRole::Client.local_first_request_id(), RequestId(0));
        assert_eq!(EndpointRole::Client.peer_first_request_id(), RequestId(1));
        assert_eq!(EndpointRole::Server.local_first_request_id(), RequestId(1));
        assert_eq!(EndpointRole::Server.peer_first_request_id(), RequestId(0));
    }

    #[test]
    fn endpoint_role_parity_check() {
        let server = EndpointRole::Server;
        assert!(server.is_valid_peer_request_id(RequestId(0)));
        assert!(server.is_valid_peer_request_id(RequestId(2)));
        assert!(!server.is_valid_peer_request_id(RequestId(1)));
        assert!(!server.is_valid_peer_request_id(RequestId(3)));

        let client = EndpointRole::Client;
        assert!(client.is_valid_peer_request_id(RequestId(1)));
        assert!(client.is_valid_peer_request_id(RequestId(3)));
        assert!(!client.is_valid_peer_request_id(RequestId(0)));
        assert!(!client.is_valid_peer_request_id(RequestId(2)));
    }

    #[test]
    fn prefix_matching() {
        let ns = TrackNamespace::from_strings(&["example.com", "meeting", "room1"]).unwrap();

        let empty_prefix = TrackNamespacePrefix { fields: vec![] };
        assert!(prefix_matches_namespace(&empty_prefix, &ns));

        let one_field = TrackNamespacePrefix {
            fields: vec![b"example.com".to_vec()],
        };
        assert!(prefix_matches_namespace(&one_field, &ns));

        let two_fields = TrackNamespacePrefix {
            fields: vec![b"example.com".to_vec(), b"meeting".to_vec()],
        };
        assert!(prefix_matches_namespace(&two_fields, &ns));

        let exact = TrackNamespacePrefix {
            fields: vec![
                b"example.com".to_vec(),
                b"meeting".to_vec(),
                b"room1".to_vec(),
            ],
        };
        assert!(prefix_matches_namespace(&exact, &ns));

        let too_long = TrackNamespacePrefix {
            fields: vec![
                b"example.com".to_vec(),
                b"meeting".to_vec(),
                b"room1".to_vec(),
                b"extra".to_vec(),
            ],
        };
        assert!(!prefix_matches_namespace(&too_long, &ns));

        let wrong = TrackNamespacePrefix {
            fields: vec![b"other.com".to_vec()],
        };
        assert!(!prefix_matches_namespace(&wrong, &ns));
    }

    #[test]
    fn track_key_hash_equality() {
        use std::collections::HashMap;
        let k1 = TrackKey {
            namespace: TrackNamespace::from_strings(&["a", "b"]).unwrap(),
            name: b"video".to_vec(),
        };
        let k2 = TrackKey {
            namespace: TrackNamespace::from_strings(&["a", "b"]).unwrap(),
            name: b"video".to_vec(),
        };
        let k3 = TrackKey {
            namespace: TrackNamespace::from_strings(&["a", "b"]).unwrap(),
            name: b"audio".to_vec(),
        };
        let mut map = HashMap::new();
        map.insert(k1.clone(), 1);
        assert_eq!(map.get(&k2), Some(&1));
        assert_eq!(map.get(&k3), None);
    }

    #[test]
    fn namespace_key_hash_equality() {
        use std::collections::HashMap;
        let k1 = NamespaceKey(TrackNamespace::from_strings(&["a", "b"]).unwrap());
        let k2 = NamespaceKey(TrackNamespace::from_strings(&["a", "b"]).unwrap());
        let k3 = NamespaceKey(TrackNamespace::from_strings(&["a", "c"]).unwrap());
        let mut map = HashMap::new();
        map.insert(k1, 1);
        assert_eq!(map.get(&k2), Some(&1));
        assert_eq!(map.get(&k3), None);
    }
}
