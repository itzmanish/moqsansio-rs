use std::collections::HashMap;

use crate::capsule::CapsuleParser;
use crate::flow_control::FlowController;
use crate::stream::WtStream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Connecting,
    Established,
    Draining,
    Closed,
}

#[derive(Debug)]
pub struct Session {
    pub id: u64,
    pub state: SessionState,
    pub flow_control: FlowController,
    pub streams: HashMap<u64, WtStream>,
    pub capsule_parser: CapsuleParser,
    pub authority: String,
    pub path: String,
    pub origin: Option<String>,
    pub offered_protocols: Vec<String>,
    pub negotiated_protocol: Option<String>,
}

impl Session {
    pub fn new(
        id: u64,
        initial_max_streams_bidi: u64,
        initial_max_streams_uni: u64,
        initial_max_data: u64,
    ) -> Self {
        Self {
            id,
            state: SessionState::Connecting,
            flow_control: FlowController::new(
                initial_max_streams_bidi,
                initial_max_streams_uni,
                initial_max_data,
            ),
            streams: HashMap::new(),
            capsule_parser: CapsuleParser::new(),
            authority: String::new(),
            path: String::new(),
            origin: None,
            offered_protocols: Vec::new(),
            negotiated_protocol: None,
        }
    }

    pub fn is_established(&self) -> bool {
        self.state == SessionState::Established
    }

    pub fn is_closed(&self) -> bool {
        self.state == SessionState::Closed
    }

    pub fn register_stream(&mut self, stream: WtStream) {
        self.streams.insert(stream.quic_stream_id, stream);
    }

    pub fn remove_stream(&mut self, quic_stream_id: u64) -> Option<WtStream> {
        self.streams.remove(&quic_stream_id)
    }
}
