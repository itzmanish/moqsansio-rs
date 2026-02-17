use std::collections::HashMap;

use moq_protocol::error::SessionError;
use moq_protocol::message::session_control::{Goaway, MaxRequestId, RequestsBlocked};
use moq_protocol::message::setup::{ClientSetup, ServerSetup};
use moq_protocol::message::ControlMessage;
use moq_protocol::params::Parameters;

use crate::events::*;
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
}

impl<S: SessionKey> Relay<S> {
    pub fn new(config: RelayConfig) -> Self {
        Self {
            config,
            sessions: HashMap::new(),
        }
    }

    pub fn session(&self, id: &S) -> Option<&Session<S>> {
        self.sessions.get(id)
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
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
            InputEvent::SubscriptionObjectReceived { .. } => vec![],
            InputEvent::FetchObjectReceived { .. } => vec![],
        }
    }

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
        self.sessions.remove(&session);
        vec![]
    }

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

        // Spec §9.5: MAX_REQUEST_ID value is the max ID the peer allows us to use (exclusive = value + 1).
        // The wire value is the actual max ID; exclusive upper bound = wire_value + 1.
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
        // Spec §9.6: informational only. The peer is telling us it's blocked.
        // We may choose to increase their MAX_REQUEST_ID in response.
        // For now, no action.
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
}
