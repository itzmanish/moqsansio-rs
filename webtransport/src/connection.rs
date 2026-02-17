use std::collections::{HashMap, HashSet, VecDeque};

use crate::capsule::Capsule;
use crate::config::Config;
use crate::error::{self, Error, Result};
use crate::h3::{H3Connection, H3Event, PeerSettings};
use crate::session::{Session, SessionState};
use crate::stream::{self, StreamType, WtStream};
use crate::varint;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    SessionRequest {
        session_id: u64,
        authority: String,
        path: String,
        origin: Option<String>,
        headers: Vec<(String, String)>,
    },
    SessionEstablished(u64),
    SessionClosed {
        session_id: u64,
        error_code: u32,
        reason: String,
    },
    SessionDraining(u64),
    UniStreamReceived {
        session_id: u64,
        stream_id: u64,
    },
    BidiStreamReceived {
        session_id: u64,
        stream_id: u64,
    },
    StreamData(u64),
    StreamFinished(u64),
    Datagram {
        session_id: u64,
        payload: Vec<u8>,
    },
    GoAway,
}

pub struct ResetInfo {
    pub h3_error_code: u64,
    pub min_reliable_size: u64,
}

pub struct CloseSessionResult {
    pub capsule_data: Vec<u8>,
    pub streams_to_reset: Vec<u64>,
    pub reset_error_code: u64,
}

pub struct Connection {
    h3: H3Connection,
    is_server: bool,
    config: Config,

    sessions: HashMap<u64, Session>,
    connect_stream_to_session: HashMap<u64, u64>,
    wt_stream_to_session: HashMap<u64, u64>,

    pending_wt_streams: HashMap<u64, WtStream>,
    pending_events: VecDeque<Event>,

    classified_streams: HashSet<u64>,
    h3_initialized: bool,

    /// §7.1: CONNECT requests received before peer SETTINGS are buffered here.
    deferred_requests: Vec<(u64, Vec<(String, String)>)>,

    previous_settings: Option<PeerSettings>,
}

impl Connection {
    pub fn new(is_server: bool, config: Config) -> Self {
        let h3 = H3Connection::new(is_server, config.clone());
        Self {
            h3,
            is_server,
            config,
            sessions: HashMap::new(),
            connect_stream_to_session: HashMap::new(),
            wt_stream_to_session: HashMap::new(),
            pending_wt_streams: HashMap::new(),
            pending_events: VecDeque::new(),
            classified_streams: HashSet::new(),
            h3_initialized: false,
            deferred_requests: Vec::new(),
            previous_settings: None,
        }
    }

    /// Initialize the H3 layer. Returns (stream_id, data) pairs to write
    /// to the QUIC connection.
    pub fn initialize(&mut self) -> Result<Vec<(u64, Vec<u8>)>> {
        if self.h3_initialized {
            return Ok(Vec::new());
        }
        self.h3_initialized = true;
        self.h3.initialize()
    }

    // -----------------------------------------------------------------------
    // Client-side session management
    // -----------------------------------------------------------------------

    pub fn connect(
        &mut self,
        authority: &str,
        path: &str,
        origin: Option<&str>,
        extra_headers: &[(&str, &str)],
    ) -> Result<(u64, Vec<u8>)> {
        if !self.h3.has_settings() {
            return Err(Error::ProtocolViolation(
                "cannot connect before peer SETTINGS received".into(),
            ));
        }

        let active_sessions = self
            .sessions
            .values()
            .filter(|s| s.state != SessionState::Closed)
            .count() as u64;

        if let Some(max) = self.h3.peer_settings.wt_max_sessions {
            if active_sessions >= max {
                return Err(Error::SessionLimitReached);
            }
        }

        let ps = &self.h3.peer_settings;
        let fc_enabled = ps.wt_initial_max_streams_bidi > 0
            || ps.wt_initial_max_streams_uni > 0
            || ps.wt_initial_max_data > 0;
        if !fc_enabled && active_sessions >= 1 {
            return Err(Error::SessionLimitReached);
        }

        let (stream_id, data) =
            self.h3
                .send_connect_request(authority, path, origin, extra_headers)?;

        let session = Session::new(
            stream_id,
            self.config.initial_max_streams_bidi,
            self.config.initial_max_streams_uni,
            self.config.initial_max_data,
        );
        self.sessions.insert(stream_id, session);
        self.connect_stream_to_session.insert(stream_id, stream_id);

        Ok((stream_id, data))
    }

    // -----------------------------------------------------------------------
    // Server-side session management
    // -----------------------------------------------------------------------

    pub fn accept_session(
        &mut self,
        session_id: u64,
        extra_headers: &[(&str, &str)],
    ) -> Result<Vec<u8>> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;

        if session.state != SessionState::Connecting {
            return Err(Error::ProtocolViolation(
                "session not in connecting state".into(),
            ));
        }

        session.state = SessionState::Established;

        if self.h3.has_settings() {
            let ps = &self.h3.peer_settings;
            session.flow_control.set_peer_initial_limits(
                ps.wt_initial_max_streams_bidi,
                ps.wt_initial_max_streams_uni,
                ps.wt_initial_max_data,
            );
            let fc_enabled = ps.wt_initial_max_streams_bidi > 0
                || ps.wt_initial_max_streams_uni > 0
                || ps.wt_initial_max_data > 0
                || ps.wt_max_sessions.is_some_and(|v| v > 1);
            session.flow_control.set_enabled(fc_enabled);
        }

        let data = self
            .h3
            .send_connect_response(session_id, 200, extra_headers)?;
        Ok(data)
    }

    pub fn reject_session(&mut self, session_id: u64, status: u16) -> Result<Vec<u8>> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;
        session.state = SessionState::Closed;

        let data = self.h3.send_connect_response(session_id, status, &[])?;
        Ok(data)
    }

    // -----------------------------------------------------------------------
    // Streams
    // -----------------------------------------------------------------------

    pub fn open_uni_stream(&mut self, session_id: u64) -> Result<(u64, Vec<u8>)> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;

        if !session.is_established() {
            return Err(Error::SessionClosed);
        }

        if !session.flow_control.can_open_uni() {
            return Err(Error::StreamBlocked);
        }

        session.flow_control.on_stream_opened_local(false);

        let header = stream::encode_stream_header(session_id, StreamType::UniDirectional)?;

        Ok((session_id, header))
    }

    pub fn open_bidi_stream(&mut self, session_id: u64) -> Result<(u64, Vec<u8>)> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;

        if !session.is_established() {
            return Err(Error::SessionClosed);
        }

        if !session.flow_control.can_open_bidi() {
            return Err(Error::StreamBlocked);
        }

        session.flow_control.on_stream_opened_local(true);

        let header = stream::encode_stream_header(session_id, StreamType::BiDirectional)?;
        Ok((session_id, header))
    }

    /// Register a WT data stream that was opened at the QUIC level.
    /// `quic_stream_id` is the actual QUIC stream ID allocated by the
    /// caller.
    pub fn register_outgoing_stream(
        &mut self,
        quic_stream_id: u64,
        session_id: u64,
        stream_type: StreamType,
    ) {
        let wt_stream = WtStream::new_outgoing(quic_stream_id, session_id, stream_type);
        self.wt_stream_to_session.insert(quic_stream_id, session_id);
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.register_stream(wt_stream);
        }
    }

    pub fn check_send(&self, quic_stream_id: u64, len: u64) -> Result<()> {
        if let Some(&session_id) = self.wt_stream_to_session.get(&quic_stream_id) {
            if let Some(session) = self.sessions.get(&session_id) {
                if !session.flow_control.can_send(len) {
                    return Err(Error::StreamBlocked);
                }
            }
        }
        Ok(())
    }

    pub fn on_data_sent(&mut self, quic_stream_id: u64, len: u64) {
        if let Some(&session_id) = self.wt_stream_to_session.get(&quic_stream_id) {
            if let Some(session) = self.sessions.get_mut(&session_id) {
                session.flow_control.on_data_sent(len);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Datagrams
    // -----------------------------------------------------------------------

    /// Encode a datagram for sending. Returns the raw bytes to pass to
    /// `quiche::Connection::dgram_send`. The caller prepends the Quarter
    /// Stream ID (session_id / 4) as a QUIC varint.
    pub fn encode_datagram(&self, session_id: u64, payload: &[u8]) -> Result<Vec<u8>> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;
        if session.is_closed() {
            return Err(Error::SessionClosed);
        }

        let quarter_stream_id = session_id / 4;
        let mut buf = Vec::with_capacity(8 + payload.len());
        let mut tmp = [0u8; 8];
        let n = varint::encode(quarter_stream_id, &mut tmp)?;
        buf.extend_from_slice(&tmp[..n]);
        buf.extend_from_slice(payload);
        Ok(buf)
    }

    /// Decode a received QUIC datagram. Returns (session_id, payload).
    pub fn decode_datagram(&self, data: &[u8]) -> Result<(u64, Vec<u8>)> {
        let (quarter_stream_id, n) = varint::decode(data)?;
        let session_id = quarter_stream_id * 4;

        let _ = self
            .sessions
            .get(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;

        let payload = data[n..].to_vec();
        Ok((session_id, payload))
    }

    // -----------------------------------------------------------------------
    // Session termination
    // -----------------------------------------------------------------------

    pub fn close_session(
        &mut self,
        session_id: u64,
        error_code: u32,
        reason: &str,
    ) -> Result<CloseSessionResult> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;
        session.state = SessionState::Closed;
        let streams_to_reset: Vec<u64> = session.streams.keys().copied().collect();

        let capsule = Capsule::CloseSession {
            error_code,
            error_message: reason.to_string(),
        };
        let mut capsule_bytes = Vec::new();
        capsule.encode(&mut capsule_bytes)?;

        let capsule_data = self.h3.encode_body_as_h3_data(&capsule_bytes)?;
        Ok(CloseSessionResult {
            capsule_data,
            streams_to_reset,
            reset_error_code: crate::frame::WT_SESSION_GONE,
        })
    }

    pub fn drain_session(&mut self, session_id: u64) -> Result<Vec<u8>> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;
        session.state = SessionState::Draining;

        let capsule = Capsule::DrainSession;
        let mut capsule_bytes = Vec::new();
        capsule.encode(&mut capsule_bytes)?;

        self.h3.encode_body_as_h3_data(&capsule_bytes)
    }

    /// Returns QUIC stream IDs of all data streams belonging to a session,
    /// so the caller can reset them with WT_SESSION_GONE.
    pub fn session_stream_ids(&self, session_id: u64) -> Vec<u64> {
        self.sessions
            .get(&session_id)
            .map(|s| s.streams.keys().copied().collect())
            .unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // Event processing
    // -----------------------------------------------------------------------

    pub fn poll_event(&mut self) -> Option<Event> {
        self.pending_events.pop_front()
    }

    /// Process data received on a QUIC stream. The caller should call
    /// this for every stream that has readable data. Returns true if the
    /// data was consumed by the WebTransport layer.
    pub fn process_stream_data(
        &mut self,
        quic_stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> Result<bool> {
        // 1. Check if this is a known WT data stream
        if let Some(&session_id) = self.wt_stream_to_session.get(&quic_stream_id) {
            self.process_wt_stream_data(quic_stream_id, session_id, data, fin)?;
            return Ok(true);
        }

        // 2. Check if this is a pending (header-incomplete) WT stream
        if self.pending_wt_streams.contains_key(&quic_stream_id) {
            self.process_pending_wt_stream(quic_stream_id, data, fin)?;
            return Ok(true);
        }

        // 3. Check if H3 owns this stream
        if self.h3.is_h3_stream(quic_stream_id) {
            self.h3.process_stream_data(quic_stream_id, data, fin)?;
            self.process_h3_events()?;
            return Ok(true);
        }

        // 4. New unclassified stream — classify it
        if !self.classified_streams.contains(&quic_stream_id) {
            return self.classify_new_stream(quic_stream_id, data, fin);
        }

        Ok(false)
    }

    /// Process a received QUIC datagram (raw bytes from dgram_recv).
    pub fn process_datagram(&mut self, data: &[u8]) -> Result<()> {
        let (session_id, payload) = self.decode_datagram(data)?;
        self.pending_events.push_back(Event::Datagram {
            session_id,
            payload,
        });
        Ok(())
    }

    fn classify_new_stream(&mut self, quic_stream_id: u64, data: &[u8], fin: bool) -> Result<bool> {
        self.classified_streams.insert(quic_stream_id);

        let is_uni = !stream::is_bidi(quic_stream_id);

        if is_uni {
            // Read stream type to classify
            let stream_type_result = self.h3.classify_uni_stream(quic_stream_id, data)?;
            match stream_type_result {
                Some(crate::frame::WT_UNI_STREAM_TYPE) => {
                    let (_, type_len) = varint::decode(data)?;
                    let mut wt_stream =
                        WtStream::new_incoming(quic_stream_id, StreamType::UniDirectional);
                    // Skip the type byte(s) — already classified
                    wt_stream.header_len = type_len as u64;

                    let remaining = &data[type_len..];
                    if !remaining.is_empty() {
                        let consumed = wt_stream.parse_session_id_only(remaining)?;
                        if wt_stream.is_header_complete() {
                            let session_id = wt_stream.session_id;
                            self.associate_incoming_stream(wt_stream)?;
                            let extra_data = &remaining[consumed..];
                            if !extra_data.is_empty() || fin {
                                self.process_wt_stream_data(
                                    quic_stream_id,
                                    session_id,
                                    extra_data,
                                    fin,
                                )?;
                            }
                        } else {
                            self.pending_wt_streams.insert(quic_stream_id, wt_stream);
                        }
                    } else {
                        self.pending_wt_streams.insert(quic_stream_id, wt_stream);
                    }
                    return Ok(true);
                }
                Some(_) => {
                    self.process_h3_events()?;
                    return Ok(true);
                }
                None => return Ok(false),
            }
        } else {
            // Bidirectional stream — peek at first byte to distinguish
            // CONNECT (H3 request) vs WT_STREAM (0x41)
            if data.is_empty() {
                return Ok(false);
            }

            let (first_varint, _) = varint::decode(data)?;
            if first_varint == crate::frame::WT_BIDI_SIGNAL {
                // WebTransport bidirectional stream
                let mut wt_stream =
                    WtStream::new_incoming(quic_stream_id, StreamType::BiDirectional);
                let consumed = wt_stream.parse_header(data)?;
                if wt_stream.is_header_complete() {
                    let session_id = wt_stream.session_id;
                    self.associate_incoming_stream(wt_stream)?;
                    let extra_data = &data[consumed..];
                    if !extra_data.is_empty() || fin {
                        self.process_wt_stream_data(quic_stream_id, session_id, extra_data, fin)?;
                    }
                } else {
                    self.pending_wt_streams.insert(quic_stream_id, wt_stream);
                }
                return Ok(true);
            } else {
                // H3 request stream (CONNECT or other)
                self.h3.register_request_stream(quic_stream_id);
                self.h3.process_stream_data(quic_stream_id, data, fin)?;
                self.process_h3_events()?;
                return Ok(true);
            }
        }
    }

    fn associate_incoming_stream(&mut self, wt_stream: WtStream) -> Result<()> {
        let session_id = wt_stream.session_id;
        if session_id % 4 != 0 {
            return Err(Error::InvalidSessionId(session_id));
        }
        let quic_stream_id = wt_stream.quic_stream_id;
        let is_bidi = wt_stream.stream_type == StreamType::BiDirectional;

        self.wt_stream_to_session.insert(quic_stream_id, session_id);

        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.flow_control.on_stream_opened_remote(is_bidi);
            session.register_stream(wt_stream);

            let event = if is_bidi {
                Event::BidiStreamReceived {
                    session_id,
                    stream_id: quic_stream_id,
                }
            } else {
                Event::UniStreamReceived {
                    session_id,
                    stream_id: quic_stream_id,
                }
            };
            self.pending_events.push_back(event);
        } else {
            // Buffer for later association (§4.6)
            if self.pending_wt_streams.len() < self.config.max_buffered_streams {
                // Already tracked in pending — will be resolved when session appears
            }
        }

        Ok(())
    }

    fn process_pending_wt_stream(
        &mut self,
        quic_stream_id: u64,
        data: &[u8],
        fin: bool,
    ) -> Result<()> {
        let mut wt_stream = self
            .pending_wt_streams
            .remove(&quic_stream_id)
            .ok_or(Error::StreamNotFound(quic_stream_id))?;

        let consumed =
            if wt_stream.header_len > 0 && wt_stream.stream_type == StreamType::UniDirectional {
                wt_stream.parse_session_id_only(data)?
            } else {
                wt_stream.parse_header(data)?
            };

        if wt_stream.is_header_complete() {
            let session_id = wt_stream.session_id;
            self.associate_incoming_stream(wt_stream)?;
            let extra_data = &data[consumed..];
            if !extra_data.is_empty() || fin {
                self.process_wt_stream_data(quic_stream_id, session_id, extra_data, fin)?;
            }
        } else {
            self.pending_wt_streams.insert(quic_stream_id, wt_stream);
        }

        Ok(())
    }

    fn process_wt_stream_data(
        &mut self,
        quic_stream_id: u64,
        session_id: u64,
        data: &[u8],
        fin: bool,
    ) -> Result<()> {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            if !data.is_empty() {
                session.flow_control.on_data_received(data.len() as u64);
                self.pending_events
                    .push_back(Event::StreamData(quic_stream_id));
            }
            if fin {
                self.pending_events
                    .push_back(Event::StreamFinished(quic_stream_id));
            }
        }
        Ok(())
    }

    fn process_h3_events(&mut self) -> Result<()> {
        let events = self.h3.drain_events();
        for event in events {
            match event {
                H3Event::SettingsReceived(peer_settings) => {
                    if let Some(ref prev) = self.previous_settings {
                        let violated = peer_settings
                            .wt_max_sessions
                            .zip(prev.wt_max_sessions)
                            .is_some_and(|(new, old)| new < old)
                            || peer_settings.wt_initial_max_streams_bidi
                                < prev.wt_initial_max_streams_bidi
                            || peer_settings.wt_initial_max_streams_uni
                                < prev.wt_initial_max_streams_uni
                            || peer_settings.wt_initial_max_data < prev.wt_initial_max_data;
                        if violated {
                            return Err(Error::H3SettingsError(
                                "0-RTT: new SETTINGS reduce limits below previous session".into(),
                            ));
                        }
                    }
                    for session in self.sessions.values_mut() {
                        session.flow_control.set_peer_initial_limits(
                            peer_settings.wt_initial_max_streams_bidi,
                            peer_settings.wt_initial_max_streams_uni,
                            peer_settings.wt_initial_max_data,
                        );
                    }
                    let deferred = std::mem::take(&mut self.deferred_requests);
                    for (stream_id, headers) in deferred {
                        self.create_session_from_connect(stream_id, headers);
                    }
                }
                H3Event::HeadersReceived { stream_id, headers } => {
                    self.process_h3_headers(stream_id, headers);
                }
                H3Event::DataReceived { stream_id, data } => {
                    self.process_connect_stream_data(stream_id, &data)?;
                }
                H3Event::GoAway { .. } => {
                    self.pending_events.push_back(Event::GoAway);
                }
                H3Event::StreamFinished(stream_id) => {
                    if let Some(&session_id) = self.connect_stream_to_session.get(&stream_id) {
                        self.handle_session_close(session_id, 0, String::new());
                    }
                }
            }
        }
        Ok(())
    }

    fn process_h3_headers(&mut self, stream_id: u64, headers: Vec<(String, String)>) {
        let get = |name: &str| -> Option<String> {
            headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };

        if self.is_server {
            let method = get(":method").unwrap_or_default();
            let protocol = get(":protocol").unwrap_or_default();

            if method == "CONNECT" && protocol == "webtransport" {
                if !self.h3.has_settings() {
                    self.deferred_requests.push((stream_id, headers));
                    return;
                }
                self.create_session_from_connect(stream_id, headers);
            }
        } else {
            // Client received response
            if let Some(&session_id) = self.connect_stream_to_session.get(&stream_id) {
                let status = get(":status")
                    .and_then(|s| s.parse::<u16>().ok())
                    .unwrap_or(0);

                if (200..300).contains(&status) {
                    if let Some(session) = self.sessions.get_mut(&session_id) {
                        session.state = SessionState::Established;

                        if self.h3.has_settings() {
                            let ps = &self.h3.peer_settings;
                            session.flow_control.set_peer_initial_limits(
                                ps.wt_initial_max_streams_bidi,
                                ps.wt_initial_max_streams_uni,
                                ps.wt_initial_max_data,
                            );
                        }

                        let wt_protocol = get("wt-protocol");
                        if let Some(ref proto) = wt_protocol {
                            if session.offered_protocols.iter().any(|p| p == proto) {
                                session.negotiated_protocol = Some(proto.clone());
                            }
                        }
                    }
                    self.pending_events
                        .push_back(Event::SessionEstablished(session_id));
                } else {
                    self.handle_session_close(session_id, 0, format!("rejected: {status}"));
                }
            }
        }
    }

    fn process_connect_stream_data(&mut self, stream_id: u64, data: &[u8]) -> Result<()> {
        if let Some(&session_id) = self.connect_stream_to_session.get(&stream_id) {
            if let Some(session) = self.sessions.get_mut(&session_id) {
                if session.is_closed() {
                    return Err(Error::ProtocolViolation(
                        "data on CONNECT stream after session closed".into(),
                    ));
                }
                let capsule_results = session.capsule_parser.feed(data);
                for result in capsule_results {
                    match result {
                        Ok(capsule) => {
                            self.process_capsule(session_id, capsule);
                        }
                        Err(_) => {
                            self.handle_session_close(session_id, 0, "capsule error".into());
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn process_capsule(&mut self, session_id: u64, capsule: Capsule) {
        match capsule {
            Capsule::CloseSession {
                error_code,
                error_message,
            } => {
                self.handle_session_close(session_id, error_code, error_message);
            }
            Capsule::DrainSession => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.state = SessionState::Draining;
                }
                self.pending_events
                    .push_back(Event::SessionDraining(session_id));
            }
            Capsule::MaxStreamsBidi(v) => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    if session.flow_control.apply_peer_max_streams_bidi(v).is_err() {
                        self.handle_session_close(session_id, 0, "flow control error".into());
                    }
                }
            }
            Capsule::MaxStreamsUni(v) => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    if session.flow_control.apply_peer_max_streams_uni(v).is_err() {
                        self.handle_session_close(session_id, 0, "flow control error".into());
                    }
                }
            }
            Capsule::MaxData(v) => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    if session.flow_control.apply_peer_max_data(v).is_err() {
                        self.handle_session_close(session_id, 0, "flow control error".into());
                    }
                }
            }
            Capsule::StreamsBlockedBidi(_)
            | Capsule::StreamsBlockedUni(_)
            | Capsule::DataBlocked(_) => {
                // Informational — no action needed
            }
        }
    }

    fn create_session_from_connect(&mut self, stream_id: u64, headers: Vec<(String, String)>) {
        let get = |name: &str| -> Option<String> {
            headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };

        let authority = get(":authority").unwrap_or_default();
        let path = get(":path").unwrap_or_default();
        let origin = get("origin");

        let mut session = Session::new(
            stream_id,
            self.config.initial_max_streams_bidi,
            self.config.initial_max_streams_uni,
            self.config.initial_max_data,
        );
        session.authority = authority.clone();
        session.path = path.clone();
        session.origin = origin.clone();

        self.sessions.insert(stream_id, session);
        self.connect_stream_to_session.insert(stream_id, stream_id);

        self.pending_events.push_back(Event::SessionRequest {
            session_id: stream_id,
            authority,
            path,
            origin,
            headers,
        });
    }

    fn handle_session_close(&mut self, session_id: u64, error_code: u32, reason: String) {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            if session.state == SessionState::Closed {
                return;
            }
            session.state = SessionState::Closed;
        }
        self.pending_events.push_back(Event::SessionClosed {
            session_id,
            error_code,
            reason,
        });
    }

    pub fn session_state(&self, session_id: u64) -> Option<SessionState> {
        self.sessions.get(&session_id).map(|s| s.state)
    }

    pub fn has_peer_settings(&self) -> bool {
        self.h3.has_settings()
    }

    pub fn peer_supports_webtransport(&self) -> bool {
        self.h3.peer_settings.supports_webtransport()
    }

    // -----------------------------------------------------------------------
    // §4.4: RESET_STREAM_AT
    // -----------------------------------------------------------------------

    pub fn prepare_reset_stream(
        &self,
        quic_stream_id: u64,
        app_error_code: u32,
    ) -> Result<ResetInfo> {
        let &session_id = self
            .wt_stream_to_session
            .get(&quic_stream_id)
            .ok_or(Error::StreamNotFound(quic_stream_id))?;

        let session = self
            .sessions
            .get(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;

        let min_reliable_size = session
            .streams
            .get(&quic_stream_id)
            .map(|s| s.header_len)
            .unwrap_or(0);

        Ok(ResetInfo {
            h3_error_code: error::webtransport_to_http3_error(app_error_code),
            min_reliable_size,
        })
    }

    // -----------------------------------------------------------------------
    // §3.4: Priority
    // -----------------------------------------------------------------------

    pub fn encode_priority_update(
        &self,
        session_id: u64,
        priority_field_value: &str,
    ) -> Result<Vec<u8>> {
        let _ = self
            .sessions
            .get(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;

        let mut payload = Vec::new();
        let mut tmp = [0u8; 8];
        let n = varint::encode(session_id, &mut tmp)?;
        payload.extend_from_slice(&tmp[..n]);
        payload.extend_from_slice(priority_field_value.as_bytes());

        let frame = crate::h3::frame::H3Frame {
            frame_type: crate::frame::H3_FRAME_PRIORITY_UPDATE,
            payload,
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf)?;
        Ok(buf)
    }

    // -----------------------------------------------------------------------
    // §3.2: 0-RTT
    // -----------------------------------------------------------------------

    pub fn set_0rtt_settings(&mut self, settings: PeerSettings) {
        self.previous_settings = Some(settings);
    }

    // -----------------------------------------------------------------------
    // §3.3: Protocol Negotiation
    // -----------------------------------------------------------------------

    pub fn connect_with_protocols(
        &mut self,
        authority: &str,
        path: &str,
        origin: Option<&str>,
        extra_headers: &[(&str, &str)],
        protocols: &[&str],
    ) -> Result<(u64, Vec<u8>)> {
        let proto_value = protocols
            .iter()
            .map(|p| format!("\"{}\"", p))
            .collect::<Vec<_>>()
            .join(", ");

        let mut headers: Vec<(&str, &str)> = extra_headers.to_vec();
        headers.push(("wt-available-protocols", &proto_value));

        let (session_id, data) = self.connect(authority, path, origin, &headers)?;

        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.offered_protocols = protocols.iter().map(|s| s.to_string()).collect();
        }

        Ok((session_id, data))
    }

    pub fn offered_protocols(&self, session_id: u64) -> Option<Vec<String>> {
        self.sessions.get(&session_id).map(|s| {
            if s.offered_protocols.is_empty() {
                return Vec::new();
            }
            s.offered_protocols.clone()
        })
    }

    pub fn negotiated_protocol(&self, session_id: u64) -> Option<String> {
        self.sessions
            .get(&session_id)
            .and_then(|s| s.negotiated_protocol.clone())
    }

    // -----------------------------------------------------------------------
    // §4.8: TLS Exporter Context
    // -----------------------------------------------------------------------

    pub fn build_exporter_context(session_id: u64, label: &[u8], context: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8 + 1 + label.len() + 1 + context.len());
        buf.extend_from_slice(&session_id.to_be_bytes());
        buf.push(label.len() as u8);
        buf.extend_from_slice(label);
        buf.push(context.len() as u8);
        buf.extend_from_slice(context);
        buf
    }

    // -----------------------------------------------------------------------
    // §5.6: Blocked Capsule Emission
    // -----------------------------------------------------------------------

    pub fn encode_streams_blocked_bidi(&self, session_id: u64) -> Result<Vec<u8>> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;

        let max = session.flow_control.peer_max_streams_bidi();
        let capsule = Capsule::StreamsBlockedBidi(max);
        let mut capsule_bytes = Vec::new();
        capsule.encode(&mut capsule_bytes)?;
        self.h3.encode_body_as_h3_data(&capsule_bytes)
    }

    pub fn encode_streams_blocked_uni(&self, session_id: u64) -> Result<Vec<u8>> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;

        let max = session.flow_control.peer_max_streams_uni();
        let capsule = Capsule::StreamsBlockedUni(max);
        let mut capsule_bytes = Vec::new();
        capsule.encode(&mut capsule_bytes)?;
        self.h3.encode_body_as_h3_data(&capsule_bytes)
    }

    pub fn encode_data_blocked(&self, session_id: u64) -> Result<Vec<u8>> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or(Error::SessionNotFound(session_id))?;

        let max = session.flow_control.peer_max_data();
        let capsule = Capsule::DataBlocked(max);
        let mut capsule_bytes = Vec::new();
        capsule.encode(&mut capsule_bytes)?;
        self.h3.encode_body_as_h3_data(&capsule_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::h3::frame as h3frame;
    use crate::h3::qpack;
    use crate::stream;

    fn client_conn() -> Connection {
        Connection::new(false, Config::default())
    }

    fn server_conn() -> Connection {
        Connection::new(true, Config::default())
    }

    fn wt_settings() -> Vec<(u64, u64)> {
        vec![
            (crate::frame::SETTINGS_WT_MAX_SESSIONS, 1),
            (crate::frame::SETTINGS_ENABLE_CONNECT_PROTOCOL, 1),
            (crate::frame::SETTINGS_H3_DATAGRAM, 1),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_UNI, 100),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI, 100),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_DATA, 1_048_576),
        ]
    }

    fn control_stream_data(settings: &[(u64, u64)]) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8];
        let n = varint::encode(crate::frame::H3_STREAM_TYPE_CONTROL, &mut tmp).unwrap();
        buf.extend_from_slice(&tmp[..n]);
        buf.extend_from_slice(&h3frame::encode_settings(settings).unwrap());
        buf
    }

    fn connect_request_frame(authority: &str, path: &str) -> Vec<u8> {
        let headers = vec![
            (":method", "CONNECT"),
            (":protocol", "webtransport"),
            (":scheme", "https"),
            (":authority", authority),
            (":path", path),
        ];
        let block = qpack::encode_header_block(&headers);
        let frame = h3frame::H3Frame {
            frame_type: crate::frame::H3_FRAME_HEADERS,
            payload: block,
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf).unwrap();
        buf
    }

    fn response_frame(status: u16) -> Vec<u8> {
        let s = status.to_string();
        let headers = vec![(":status", s.as_str())];
        let block = qpack::encode_header_block(&headers);
        let frame = h3frame::H3Frame {
            frame_type: crate::frame::H3_FRAME_HEADERS,
            payload: block,
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf).unwrap();
        buf
    }

    fn h3_data_frame(payload: &[u8]) -> Vec<u8> {
        let frame = h3frame::H3Frame {
            frame_type: crate::frame::H3_FRAME_DATA,
            payload: payload.to_vec(),
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf).unwrap();
        buf
    }

    fn established_server() -> (Connection, u64) {
        let mut s = server_conn();
        s.initialize().unwrap();
        let ctrl = control_stream_data(&wt_settings());
        s.process_stream_data(2, &ctrl, false).unwrap();
        let req = connect_request_frame("example.com", "/wt");
        s.process_stream_data(0, &req, false).unwrap();
        while s.poll_event().is_some() {}
        s.accept_session(0, &[]).unwrap();
        (s, 0)
    }

    fn feed_peer_settings(c: &mut Connection, stream_id: u64) {
        let ctrl = control_stream_data(&wt_settings());
        c.process_stream_data(stream_id, &ctrl, false).unwrap();
    }

    fn established_client() -> (Connection, u64) {
        let mut c = client_conn();
        c.initialize().unwrap();
        feed_peer_settings(&mut c, 3);
        let (sid, _) = c.connect("example.com", "/wt", None, &[]).unwrap();
        let resp = response_frame(200);
        c.process_stream_data(sid, &resp, false).unwrap();
        while c.poll_event().is_some() {}
        (c, sid)
    }

    // === Initialization ===

    #[test]
    fn init_client_produces_three_uni_streams() {
        let mut c = client_conn();
        let sends = c.initialize().unwrap();
        assert_eq!(sends.len(), 3);
        assert_eq!(sends[0].0, 2);
        assert_eq!(sends[1].0, 6);
        assert_eq!(sends[2].0, 10);
    }

    #[test]
    fn init_server_produces_three_uni_streams() {
        let mut s = server_conn();
        let sends = s.initialize().unwrap();
        assert_eq!(sends.len(), 3);
        assert_eq!(sends[0].0, 3);
        assert_eq!(sends[1].0, 7);
        assert_eq!(sends[2].0, 11);
    }

    #[test]
    fn init_idempotent() {
        let mut c = client_conn();
        c.initialize().unwrap();
        assert!(c.initialize().unwrap().is_empty());
    }

    // === Client Connect ===

    #[test]
    fn client_connect_allocates_first_bidi_stream() {
        let mut c = client_conn();
        c.initialize().unwrap();
        feed_peer_settings(&mut c, 3);
        let (sid, data) = c.connect("example.com", "/wt", None, &[]).unwrap();
        assert_eq!(sid, 0);
        assert!(!data.is_empty());
    }

    #[test]
    fn client_connect_session_starts_connecting() {
        let mut c = client_conn();
        c.initialize().unwrap();
        feed_peer_settings(&mut c, 3);
        let (sid, _) = c.connect("example.com", "/wt", None, &[]).unwrap();
        assert_eq!(c.session_state(sid), Some(SessionState::Connecting));
    }

    #[test]
    fn client_multiple_sessions_get_distinct_ids() {
        let mut c = client_conn();
        c.initialize().unwrap();
        let settings = vec![
            (crate::frame::SETTINGS_WT_MAX_SESSIONS, 10),
            (crate::frame::SETTINGS_ENABLE_CONNECT_PROTOCOL, 1),
            (crate::frame::SETTINGS_H3_DATAGRAM, 1),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_UNI, 100),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI, 100),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_DATA, 1_048_576),
        ];
        let ctrl = control_stream_data(&settings);
        c.process_stream_data(3, &ctrl, false).unwrap();
        let (s1, _) = c.connect("a.com", "/1", None, &[]).unwrap();
        let (s2, _) = c.connect("b.com", "/2", None, &[]).unwrap();
        assert_ne!(s1, s2);
    }

    // === Server Accept / Reject ===

    #[test]
    fn server_accept_transitions_to_established() {
        let (s, sid) = established_server();
        assert_eq!(s.session_state(sid), Some(SessionState::Established));
    }

    #[test]
    fn server_accept_unknown_session_errors() {
        let mut s = server_conn();
        s.initialize().unwrap();
        assert!(s.accept_session(999, &[]).is_err());
    }

    #[test]
    fn server_accept_already_established_errors() {
        let (mut s, sid) = established_server();
        assert!(s.accept_session(sid, &[]).is_err());
    }

    #[test]
    fn server_reject_closes_session() {
        let mut s = server_conn();
        s.initialize().unwrap();
        let ctrl = control_stream_data(&wt_settings());
        s.process_stream_data(2, &ctrl, false).unwrap();
        let req = connect_request_frame("example.com", "/wt");
        s.process_stream_data(0, &req, false).unwrap();
        while s.poll_event().is_some() {}
        s.reject_session(0, 403).unwrap();
        assert_eq!(s.session_state(0), Some(SessionState::Closed));
    }

    // === Stream Operations ===

    #[test]
    fn open_uni_on_established_session() {
        let (mut s, sid) = established_server();
        let (_, header) = s.open_uni_stream(sid).unwrap();
        assert!(!header.is_empty());
    }

    #[test]
    fn open_bidi_on_established_session() {
        let (mut s, sid) = established_server();
        assert!(s.open_bidi_stream(sid).is_ok());
    }

    #[test]
    fn open_stream_unknown_session_errors() {
        let (mut s, _) = established_server();
        assert!(s.open_uni_stream(999).is_err());
        assert!(s.open_bidi_stream(999).is_err());
    }

    #[test]
    fn open_stream_on_closed_session_errors() {
        let (mut s, sid) = established_server();
        s.close_session(sid, 0, "done").unwrap();
        assert!(s.open_uni_stream(sid).is_err());
    }

    #[test]
    fn flow_control_blocks_excess_uni_streams() {
        let mut s = server_conn();
        s.initialize().unwrap();
        let settings = vec![
            (crate::frame::SETTINGS_WT_MAX_SESSIONS, 1),
            (crate::frame::SETTINGS_ENABLE_CONNECT_PROTOCOL, 1),
            (crate::frame::SETTINGS_H3_DATAGRAM, 1),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_UNI, 1),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI, 100),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_DATA, 1_048_576),
        ];
        let ctrl = control_stream_data(&settings);
        s.process_stream_data(2, &ctrl, false).unwrap();
        let req = connect_request_frame("example.com", "/wt");
        s.process_stream_data(0, &req, false).unwrap();
        while s.poll_event().is_some() {}
        s.accept_session(0, &[]).unwrap();

        assert!(s.open_uni_stream(0).is_ok());
        assert!(s.open_uni_stream(0).is_err());
    }

    #[test]
    fn check_send_respects_data_limit() {
        let (mut s, sid) = established_server();
        s.register_outgoing_stream(100, sid, StreamType::UniDirectional);
        assert!(s.check_send(100, 1_048_576).is_ok());
        s.on_data_sent(100, 1_048_576);
        assert!(s.check_send(100, 1).is_err());
    }

    // === Datagrams ===

    #[test]
    fn datagram_encode_decode_roundtrip() {
        let (s, sid) = established_server();
        let encoded = s.encode_datagram(sid, b"hello").unwrap();
        let (decoded_sid, payload) = s.decode_datagram(&encoded).unwrap();
        assert_eq!(decoded_sid, sid);
        assert_eq!(payload, b"hello");
    }

    #[test]
    fn datagram_unknown_session_errors() {
        let (s, _) = established_server();
        assert!(s.encode_datagram(999, b"x").is_err());
    }

    // === Session Termination ===

    #[test]
    fn close_session_produces_h3_data_and_closes() {
        let (mut s, sid) = established_server();
        let result = s.close_session(sid, 42, "bye").unwrap();
        assert!(!result.capsule_data.is_empty());
        assert_eq!(s.session_state(sid), Some(SessionState::Closed));
    }

    #[test]
    fn drain_session_transitions_to_draining() {
        let (mut s, sid) = established_server();
        let data = s.drain_session(sid).unwrap();
        assert!(!data.is_empty());
        assert_eq!(s.session_state(sid), Some(SessionState::Draining));
    }

    #[test]
    fn close_unknown_session_errors() {
        let (mut s, _) = established_server();
        assert!(s.close_session(999, 0, "").is_err());
    }

    #[test]
    fn session_stream_ids_tracks_registered_streams() {
        let (mut s, sid) = established_server();
        assert!(s.session_stream_ids(sid).is_empty());
        s.register_outgoing_stream(100, sid, StreamType::UniDirectional);
        s.register_outgoing_stream(104, sid, StreamType::BiDirectional);
        let ids = s.session_stream_ids(sid);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&100));
        assert!(ids.contains(&104));
    }

    // === H3 Event Processing ===

    #[test]
    fn server_receives_connect_request_event() {
        let mut s = server_conn();
        s.initialize().unwrap();
        let ctrl = control_stream_data(&wt_settings());
        s.process_stream_data(2, &ctrl, false).unwrap();
        let req = connect_request_frame("example.com", "/wt");
        s.process_stream_data(0, &req, false).unwrap();

        match s.poll_event() {
            Some(Event::SessionRequest {
                session_id,
                authority,
                path,
                ..
            }) => {
                assert_eq!(session_id, 0);
                assert_eq!(authority, "example.com");
                assert_eq!(path, "/wt");
            }
            other => panic!("expected SessionRequest, got {other:?}"),
        }
    }

    #[test]
    fn client_receives_200_establishes_session() {
        let mut c = client_conn();
        c.initialize().unwrap();
        feed_peer_settings(&mut c, 3);
        let (sid, _) = c.connect("example.com", "/wt", None, &[]).unwrap();
        let resp = response_frame(200);
        c.process_stream_data(sid, &resp, false).unwrap();

        assert_eq!(c.session_state(sid), Some(SessionState::Established));
        match c.poll_event() {
            Some(Event::SessionEstablished(id)) => assert_eq!(id, sid),
            other => panic!("expected SessionEstablished, got {other:?}"),
        }
    }

    #[test]
    fn client_receives_rejection_closes_session() {
        let mut c = client_conn();
        c.initialize().unwrap();
        feed_peer_settings(&mut c, 3);
        let (sid, _) = c.connect("example.com", "/wt", None, &[]).unwrap();
        let resp = response_frame(403);
        c.process_stream_data(sid, &resp, false).unwrap();

        assert_eq!(c.session_state(sid), Some(SessionState::Closed));
    }

    // === Incoming WT Streams ===

    #[test]
    fn incoming_wt_uni_stream_emits_event() {
        let (mut s, sid) = established_server();
        let mut data = stream::encode_stream_header(sid, StreamType::UniDirectional).unwrap();
        data.extend_from_slice(b"payload");
        // client-initiated uni stream 14 (after 2, 6, 10 used for control/qpack)
        s.process_stream_data(14, &data, false).unwrap();

        match s.poll_event() {
            Some(Event::UniStreamReceived {
                session_id,
                stream_id,
            }) => {
                assert_eq!(session_id, sid);
                assert_eq!(stream_id, 14);
            }
            other => panic!("expected UniStreamReceived, got {other:?}"),
        }
    }

    #[test]
    fn incoming_wt_bidi_stream_emits_event() {
        let (mut s, sid) = established_server();
        let mut data = stream::encode_stream_header(sid, StreamType::BiDirectional).unwrap();
        data.extend_from_slice(b"payload");
        // client-initiated bidi stream 4 (after 0 used for CONNECT)
        s.process_stream_data(4, &data, false).unwrap();

        match s.poll_event() {
            Some(Event::BidiStreamReceived {
                session_id,
                stream_id,
            }) => {
                assert_eq!(session_id, sid);
                assert_eq!(stream_id, 4);
            }
            other => panic!("expected BidiStreamReceived, got {other:?}"),
        }
    }

    // === Incoming Datagrams ===

    #[test]
    fn incoming_datagram_emits_event() {
        let (mut s, sid) = established_server();
        let encoded = s.encode_datagram(sid, b"dgram").unwrap();
        s.process_datagram(&encoded).unwrap();

        match s.poll_event() {
            Some(Event::Datagram {
                session_id,
                payload,
            }) => {
                assert_eq!(session_id, sid);
                assert_eq!(payload, b"dgram");
            }
            other => panic!("expected Datagram, got {other:?}"),
        }
    }

    // === Capsule Processing ===

    #[test]
    fn incoming_close_capsule_closes_session() {
        let (mut c, sid) = established_client();
        let capsule = Capsule::CloseSession {
            error_code: 42,
            error_message: "goodbye".into(),
        };
        let mut capsule_bytes = Vec::new();
        capsule.encode(&mut capsule_bytes).unwrap();
        let frame_data = h3_data_frame(&capsule_bytes);
        c.process_stream_data(sid, &frame_data, false).unwrap();

        assert_eq!(c.session_state(sid), Some(SessionState::Closed));
        match c.poll_event() {
            Some(Event::SessionClosed {
                session_id,
                error_code,
                reason,
            }) => {
                assert_eq!(session_id, sid);
                assert_eq!(error_code, 42);
                assert_eq!(reason, "goodbye");
            }
            other => panic!("expected SessionClosed, got {other:?}"),
        }
    }

    #[test]
    fn incoming_drain_capsule_transitions_session() {
        let (mut c, sid) = established_client();
        let capsule = Capsule::DrainSession;
        let mut capsule_bytes = Vec::new();
        capsule.encode(&mut capsule_bytes).unwrap();
        let frame_data = h3_data_frame(&capsule_bytes);
        c.process_stream_data(sid, &frame_data, false).unwrap();

        assert_eq!(c.session_state(sid), Some(SessionState::Draining));
        match c.poll_event() {
            Some(Event::SessionDraining(id)) => assert_eq!(id, sid),
            other => panic!("expected SessionDraining, got {other:?}"),
        }
    }

    #[test]
    fn incoming_max_streams_capsule_updates_limit() {
        let (mut c, sid) = established_client();
        let capsule = Capsule::MaxStreamsBidi(200);
        let mut capsule_bytes = Vec::new();
        capsule.encode(&mut capsule_bytes).unwrap();
        let frame_data = h3_data_frame(&capsule_bytes);
        c.process_stream_data(sid, &frame_data, false).unwrap();

        // initial peer limit was 100, now 200 — 101st open should succeed
        for _ in 0..101 {
            assert!(c.open_bidi_stream(sid).is_ok());
        }
    }

    // === State Queries ===

    #[test]
    fn session_state_none_for_unknown() {
        let c = client_conn();
        assert_eq!(c.session_state(999), None);
    }

    #[test]
    fn peer_supports_wt_after_settings() {
        let mut c = client_conn();
        c.initialize().unwrap();
        assert!(!c.peer_supports_webtransport());

        let ctrl = control_stream_data(&wt_settings());
        c.process_stream_data(3, &ctrl, false).unwrap();
        assert!(c.peer_supports_webtransport());
    }

    // === CONNECT Stream FIN ===

    #[test]
    fn connect_stream_fin_closes_session() {
        let (mut c, sid) = established_client();
        c.process_stream_data(sid, &[], true).unwrap();
        assert_eq!(c.session_state(sid), Some(SessionState::Closed));
    }

    // === GoAway ===

    #[test]
    fn goaway_emits_event() {
        let (mut c, _sid) = established_client();
        let goaway = h3frame::encode_goaway(0).unwrap();
        c.process_stream_data(3, &goaway, false).unwrap();
        match c.poll_event() {
            Some(Event::GoAway) => {}
            other => panic!("expected GoAway, got {other:?}"),
        }
    }

    // ===================================================================
    // RFC Compliance Tests (§3.1, §4, §5, §6, §7.1)
    // These test normative MUST requirements from draft-ietf-webtrans-http3-14.
    // ===================================================================

    // === §3.1 + §7.1: SETTINGS Gate ===

    #[test]
    fn rfc_client_connect_before_settings_errors() {
        let mut c = client_conn();
        c.initialize().unwrap();
        // §7.1: "any WebTransport endpoint MUST wait for the peer's SETTINGS
        // frame before sending or processing any WebTransport traffic."
        assert!(c.connect("example.com", "/wt", None, &[]).is_err());
    }

    #[test]
    fn rfc_server_defers_connect_before_client_settings() {
        let mut s = server_conn();
        s.initialize().unwrap();
        // CONNECT arrives before peer SETTINGS
        let req = connect_request_frame("example.com", "/wt");
        s.process_stream_data(0, &req, false).unwrap();

        // §7.1: server MUST NOT process WT requests until client SETTINGS received
        assert_eq!(s.session_state(0), None);
        assert!(s.poll_event().is_none());

        // After client SETTINGS arrive, deferred CONNECT is processed
        let ctrl = control_stream_data(&wt_settings());
        s.process_stream_data(2, &ctrl, false).unwrap();
        match s.poll_event() {
            Some(Event::SessionRequest { session_id: 0, .. }) => {}
            other => panic!("expected deferred SessionRequest, got {other:?}"),
        }
    }

    // === §4: Session ID Validation ===

    #[test]
    fn rfc_invalid_session_id_server_initiated_bidi() {
        let (mut s, _) = established_server();
        // session_id=1 → server-initiated bidi (1 % 4 = 1), not valid
        // §4: "MUST close the connection with an H3_ID_ERROR"
        let mut data = Vec::new();
        let mut tmp = [0u8; 8];
        let n = varint::encode(crate::frame::WT_UNI_STREAM_TYPE, &mut tmp).unwrap();
        data.extend_from_slice(&tmp[..n]);
        let n = varint::encode(1u64, &mut tmp).unwrap();
        data.extend_from_slice(&tmp[..n]);
        data.extend_from_slice(b"payload");
        assert!(s.process_stream_data(14, &data, false).is_err());
    }

    #[test]
    fn rfc_invalid_session_id_uni_stream() {
        let (mut s, _) = established_server();
        // session_id=2 → client-initiated uni (2 % 4 = 2), not valid
        let mut data = Vec::new();
        let mut tmp = [0u8; 8];
        let n = varint::encode(crate::frame::WT_UNI_STREAM_TYPE, &mut tmp).unwrap();
        data.extend_from_slice(&tmp[..n]);
        let n = varint::encode(2u64, &mut tmp).unwrap();
        data.extend_from_slice(&tmp[..n]);
        data.extend_from_slice(b"payload");
        assert!(s.process_stream_data(14, &data, false).is_err());
    }

    // === §6: Session Termination ===

    #[test]
    fn rfc_encode_datagram_after_close_errors() {
        let (mut s, sid) = established_server();
        s.close_session(sid, 0, "bye").unwrap();
        // §6: "MUST NOT send any new datagrams" after session terminated
        assert!(s.encode_datagram(sid, b"data").is_err());
    }

    #[test]
    fn rfc_connect_data_after_close_capsule_errors() {
        let (mut c, sid) = established_client();
        // Receive CLOSE_SESSION capsule from server
        let capsule = Capsule::CloseSession {
            error_code: 0,
            error_message: String::new(),
        };
        let mut capsule_bytes = Vec::new();
        capsule.encode(&mut capsule_bytes).unwrap();
        c.process_stream_data(sid, &h3_data_frame(&capsule_bytes), false)
            .unwrap();
        while c.poll_event().is_some() {}

        // §6: "If any additional stream data is received on the CONNECT stream
        // after receiving a WT_CLOSE_SESSION capsule, the stream MUST be reset
        // with code H3_MESSAGE_ERROR."
        let extra = h3_data_frame(b"extra");
        assert!(c.process_stream_data(sid, &extra, false).is_err());
    }

    // === §5.2: Session Count Enforcement ===

    #[test]
    fn rfc_client_connect_exceeds_max_sessions() {
        let mut c = client_conn();
        c.initialize().unwrap();
        // Peer allows max 1 session via SETTINGS
        let settings = vec![
            (crate::frame::SETTINGS_WT_MAX_SESSIONS, 1),
            (crate::frame::SETTINGS_ENABLE_CONNECT_PROTOCOL, 1),
            (crate::frame::SETTINGS_H3_DATAGRAM, 1),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_UNI, 100),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI, 100),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_DATA, 1_048_576),
        ];
        let ctrl = control_stream_data(&settings);
        c.process_stream_data(3, &ctrl, false).unwrap();

        // §5.2: "client MUST NOT open more simultaneous sessions than indicated"
        assert!(c.connect("a.com", "/1", None, &[]).is_ok());
        assert!(c.connect("b.com", "/2", None, &[]).is_err());
    }

    // === §5.1: Single Session When Flow Control Disabled ===

    #[test]
    fn rfc_only_one_session_when_fc_disabled() {
        let cfg = Config {
            initial_max_streams_uni: 0,
            initial_max_streams_bidi: 0,
            initial_max_data: 0,
            ..Config::default()
        };
        let mut c = Connection::new(false, cfg);
        c.initialize().unwrap();
        // Server allows 10 sessions, but has no FC limits → FC disabled
        let settings = vec![
            (crate::frame::SETTINGS_WT_MAX_SESSIONS, 10),
            (crate::frame::SETTINGS_ENABLE_CONNECT_PROTOCOL, 1),
            (crate::frame::SETTINGS_H3_DATAGRAM, 1),
        ];
        let ctrl = control_stream_data(&settings);
        c.process_stream_data(3, &ctrl, false).unwrap();

        // §5.1: "clients MUST NOT attempt to establish more than one session"
        assert!(c.connect("a.com", "/1", None, &[]).is_ok());
        assert!(c.connect("b.com", "/2", None, &[]).is_err());
    }

    // ===================================================================
    // §4.4: RESET_STREAM_AT
    // ===================================================================

    #[test]
    fn rfc_reset_stream_returns_header_size_as_reliable_size() {
        let (mut s, sid) = established_server();
        let (_, header) = s.open_uni_stream(sid).unwrap();
        let quic_stream_id = 100;
        s.register_outgoing_stream(quic_stream_id, sid, StreamType::UniDirectional);
        let info = s.prepare_reset_stream(quic_stream_id, 0).unwrap();
        assert_eq!(
            info.h3_error_code,
            crate::error::webtransport_to_http3_error(0)
        );
        assert!(info.min_reliable_size <= header.len() as u64);
    }

    #[test]
    fn rfc_reset_stream_maps_app_error_to_h3() {
        let (mut s, sid) = established_server();
        let quic_stream_id = 100;
        s.register_outgoing_stream(quic_stream_id, sid, StreamType::UniDirectional);
        let info = s.prepare_reset_stream(quic_stream_id, 42).unwrap();
        assert_eq!(
            info.h3_error_code,
            crate::error::webtransport_to_http3_error(42)
        );
    }

    #[test]
    fn rfc_reset_unknown_stream_errors() {
        let (s, _) = established_server();
        assert!(s.prepare_reset_stream(999, 0).is_err());
    }

    // ===================================================================
    // §3.4: Priority
    // ===================================================================

    #[test]
    fn rfc_encode_priority_update_for_session() {
        let (c, sid) = established_client();
        let data = c.encode_priority_update(sid, "u=3, i").unwrap();
        assert!(!data.is_empty());
    }

    #[test]
    fn rfc_priority_update_unknown_session_errors() {
        let (c, _) = established_client();
        assert!(c.encode_priority_update(999, "u=3").is_err());
    }

    // ===================================================================
    // §3.2: 0-RTT
    // ===================================================================

    #[test]
    fn rfc_0rtt_server_rejects_reduced_max_sessions() {
        let mut s = server_conn();
        s.initialize().unwrap();
        s.set_0rtt_settings(PeerSettings {
            wt_max_sessions: Some(5),
            ..PeerSettings::default()
        });
        let settings = vec![
            (crate::frame::SETTINGS_WT_MAX_SESSIONS, 2),
            (crate::frame::SETTINGS_ENABLE_CONNECT_PROTOCOL, 1),
            (crate::frame::SETTINGS_H3_DATAGRAM, 1),
        ];
        let ctrl = control_stream_data(&settings);
        assert!(s.process_stream_data(2, &ctrl, false).is_err());
    }

    #[test]
    fn rfc_0rtt_server_rejects_reduced_fc_limits() {
        let mut s = server_conn();
        s.initialize().unwrap();
        s.set_0rtt_settings(PeerSettings {
            wt_initial_max_streams_bidi: 100,
            wt_initial_max_streams_uni: 100,
            wt_initial_max_data: 1_048_576,
            ..PeerSettings::default()
        });
        let settings = vec![
            (crate::frame::SETTINGS_WT_MAX_SESSIONS, 1),
            (crate::frame::SETTINGS_ENABLE_CONNECT_PROTOCOL, 1),
            (crate::frame::SETTINGS_H3_DATAGRAM, 1),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI, 50),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_UNI, 100),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_DATA, 1_048_576),
        ];
        let ctrl = control_stream_data(&settings);
        assert!(s.process_stream_data(2, &ctrl, false).is_err());
    }

    #[test]
    fn rfc_0rtt_server_accepts_same_or_higher_limits() {
        let mut s = server_conn();
        s.initialize().unwrap();
        s.set_0rtt_settings(PeerSettings {
            wt_max_sessions: Some(5),
            wt_initial_max_streams_bidi: 100,
            wt_initial_max_streams_uni: 100,
            wt_initial_max_data: 1_048_576,
            ..PeerSettings::default()
        });
        let settings = vec![
            (crate::frame::SETTINGS_WT_MAX_SESSIONS, 10),
            (crate::frame::SETTINGS_ENABLE_CONNECT_PROTOCOL, 1),
            (crate::frame::SETTINGS_H3_DATAGRAM, 1),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI, 200),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_UNI, 100),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_DATA, 2_000_000),
        ];
        let ctrl = control_stream_data(&settings);
        assert!(s.process_stream_data(2, &ctrl, false).is_ok());
    }

    // ===================================================================
    // §3.3: Protocol Negotiation
    // ===================================================================

    #[test]
    fn rfc_connect_with_protocols_tracks_offered() {
        let mut c = client_conn();
        c.initialize().unwrap();
        feed_peer_settings(&mut c, 3);
        let (sid, _) = c
            .connect_with_protocols("example.com", "/wt", None, &[], &["h3", "h3-29"])
            .unwrap();
        assert_eq!(
            c.offered_protocols(sid),
            Some(vec!["h3".to_string(), "h3-29".to_string()])
        );
    }

    #[test]
    fn rfc_client_session_established_with_negotiated_protocol() {
        let mut c = client_conn();
        c.initialize().unwrap();
        feed_peer_settings(&mut c, 3);
        let (sid, _) = c
            .connect_with_protocols("example.com", "/wt", None, &[], &["h3", "h3-29"])
            .unwrap();
        let headers = vec![(":status", "200"), ("wt-protocol", "h3")];
        let block = qpack::encode_header_block(&headers);
        let frame = h3frame::H3Frame {
            frame_type: crate::frame::H3_FRAME_HEADERS,
            payload: block,
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf).unwrap();
        c.process_stream_data(sid, &buf, false).unwrap();
        while c.poll_event().is_some() {}
        assert_eq!(c.negotiated_protocol(sid), Some("h3".to_string()));
    }

    #[test]
    fn rfc_client_ignores_server_protocol_not_in_offered_list() {
        let mut c = client_conn();
        c.initialize().unwrap();
        feed_peer_settings(&mut c, 3);
        let (sid, _) = c
            .connect_with_protocols("example.com", "/wt", None, &[], &["h3"])
            .unwrap();
        let headers = vec![(":status", "200"), ("wt-protocol", "unknown-proto")];
        let block = qpack::encode_header_block(&headers);
        let frame = h3frame::H3Frame {
            frame_type: crate::frame::H3_FRAME_HEADERS,
            payload: block,
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf).unwrap();
        c.process_stream_data(sid, &buf, false).unwrap();
        while c.poll_event().is_some() {}
        assert_eq!(c.negotiated_protocol(sid), None);
    }

    // ===================================================================
    // §4.8: TLS Exporter Context
    // ===================================================================

    #[test]
    fn rfc_exporter_context_serialization() {
        let ctx = Connection::build_exporter_context(0, b"test-label", b"test-context");
        assert_eq!(ctx.len(), 8 + 1 + 10 + 1 + 12);
        assert_eq!(&ctx[0..8], &[0u8; 8]);
        assert_eq!(ctx[8], 10); // label length
        assert_eq!(&ctx[9..19], b"test-label");
        assert_eq!(ctx[19], 12); // context length
        assert_eq!(&ctx[20..32], b"test-context");
    }

    #[test]
    fn rfc_exporter_context_empty_context() {
        let ctx = Connection::build_exporter_context(4, b"label", b"");
        assert_eq!(ctx.len(), 8 + 1 + 5 + 1);
        assert_eq!(&ctx[0..8], &[0u8, 0, 0, 0, 0, 0, 0, 4]);
        assert_eq!(ctx[8], 5);
        assert_eq!(&ctx[9..14], b"label");
        assert_eq!(ctx[14], 0);
    }

    // ===================================================================
    // §5.6: Blocked Capsule Emission
    // ===================================================================

    #[test]
    fn rfc_streams_blocked_bidi_capsule_emitted() {
        let mut s = server_conn();
        s.initialize().unwrap();
        let settings = vec![
            (crate::frame::SETTINGS_WT_MAX_SESSIONS, 1),
            (crate::frame::SETTINGS_ENABLE_CONNECT_PROTOCOL, 1),
            (crate::frame::SETTINGS_H3_DATAGRAM, 1),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI, 1),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_UNI, 100),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_DATA, 1_048_576),
        ];
        let ctrl = control_stream_data(&settings);
        s.process_stream_data(2, &ctrl, false).unwrap();
        let req = connect_request_frame("example.com", "/wt");
        s.process_stream_data(0, &req, false).unwrap();
        while s.poll_event().is_some() {}
        s.accept_session(0, &[]).unwrap();

        assert!(s.open_bidi_stream(0).is_ok());
        assert!(s.open_bidi_stream(0).is_err());
        let capsule_data = s.encode_streams_blocked_bidi(0).unwrap();
        assert!(!capsule_data.is_empty());
    }

    #[test]
    fn rfc_streams_blocked_uni_capsule_emitted() {
        let mut s = server_conn();
        s.initialize().unwrap();
        let settings = vec![
            (crate::frame::SETTINGS_WT_MAX_SESSIONS, 1),
            (crate::frame::SETTINGS_ENABLE_CONNECT_PROTOCOL, 1),
            (crate::frame::SETTINGS_H3_DATAGRAM, 1),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_UNI, 1),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI, 100),
            (crate::frame::SETTINGS_WT_INITIAL_MAX_DATA, 1_048_576),
        ];
        let ctrl = control_stream_data(&settings);
        s.process_stream_data(2, &ctrl, false).unwrap();
        let req = connect_request_frame("example.com", "/wt");
        s.process_stream_data(0, &req, false).unwrap();
        while s.poll_event().is_some() {}
        s.accept_session(0, &[]).unwrap();

        assert!(s.open_uni_stream(0).is_ok());
        assert!(s.open_uni_stream(0).is_err());
        let capsule_data = s.encode_streams_blocked_uni(0).unwrap();
        assert!(!capsule_data.is_empty());
    }

    #[test]
    fn rfc_data_blocked_capsule_emitted() {
        let (mut s, sid) = established_server();
        s.register_outgoing_stream(100, sid, StreamType::UniDirectional);
        s.on_data_sent(100, 1_048_576);
        let capsule_data = s.encode_data_blocked(sid).unwrap();
        assert!(!capsule_data.is_empty());
    }

    // ===================================================================
    // §6: CloseAction
    // ===================================================================

    #[test]
    fn rfc_close_session_returns_streams_to_reset() {
        let (mut s, sid) = established_server();
        s.register_outgoing_stream(100, sid, StreamType::UniDirectional);
        s.register_outgoing_stream(104, sid, StreamType::BiDirectional);
        let result = s.close_session(sid, 0, "bye").unwrap();
        assert!(!result.capsule_data.is_empty());
        assert_eq!(result.streams_to_reset.len(), 2);
        assert!(result.streams_to_reset.contains(&100));
        assert!(result.streams_to_reset.contains(&104));
        assert_eq!(result.reset_error_code, crate::frame::WT_SESSION_GONE);
    }

    #[test]
    fn rfc_close_session_no_streams_returns_empty() {
        let (mut s, sid) = established_server();
        let result = s.close_session(sid, 0, "bye").unwrap();
        assert!(!result.capsule_data.is_empty());
        assert!(result.streams_to_reset.is_empty());
    }
}
