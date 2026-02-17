pub mod frame;
pub mod qpack;

use std::collections::HashMap;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::varint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamRole {
    Control,
    QpackEncoder,
    QpackDecoder,
    Request,
}

#[derive(Debug)]
struct StreamState {
    role: StreamRole,
    frame_parser: frame::FrameParser,
}

#[derive(Debug, Clone)]
pub struct PeerSettings {
    pub wt_max_sessions: Option<u64>,
    pub enable_connect_protocol: bool,
    pub h3_datagram: bool,
    pub wt_initial_max_streams_uni: u64,
    pub wt_initial_max_streams_bidi: u64,
    pub wt_initial_max_data: u64,
    pub raw: Vec<(u64, u64)>,
}

impl Default for PeerSettings {
    fn default() -> Self {
        Self {
            wt_max_sessions: None,
            enable_connect_protocol: false,
            h3_datagram: false,
            wt_initial_max_streams_uni: 0,
            wt_initial_max_streams_bidi: 0,
            wt_initial_max_data: 0,
            raw: Vec::new(),
        }
    }
}

impl PeerSettings {
    pub fn supports_webtransport(&self) -> bool {
        self.wt_max_sessions.is_some_and(|v| v > 0) && self.h3_datagram
    }

    fn apply(&mut self, settings: &[(u64, u64)]) {
        self.raw = settings.to_vec();
        for &(id, value) in settings {
            match id {
                crate::frame::SETTINGS_WT_MAX_SESSIONS => {
                    self.wt_max_sessions = Some(value);
                }
                crate::frame::SETTINGS_ENABLE_CONNECT_PROTOCOL => {
                    self.enable_connect_protocol = value == 1;
                }
                crate::frame::SETTINGS_H3_DATAGRAM => {
                    self.h3_datagram = value == 1;
                }
                crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_UNI => {
                    self.wt_initial_max_streams_uni = value;
                }
                crate::frame::SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI => {
                    self.wt_initial_max_streams_bidi = value;
                }
                crate::frame::SETTINGS_WT_INITIAL_MAX_DATA => {
                    self.wt_initial_max_data = value;
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug)]
pub enum H3Event {
    SettingsReceived(PeerSettings),
    HeadersReceived {
        stream_id: u64,
        headers: Vec<(String, String)>,
    },
    DataReceived {
        stream_id: u64,
        data: Vec<u8>,
    },
    GoAway {
        stream_id: u64,
    },
    StreamFinished(u64),
}

#[derive(Debug)]
pub struct H3Connection {
    #[allow(dead_code)] // Will be used for client/server role distinction
    is_server: bool,
    config: Config,

    local_control_stream_id: Option<u64>,
    local_encoder_stream_id: Option<u64>,
    local_decoder_stream_id: Option<u64>,

    peer_control_stream_id: Option<u64>,

    settings_sent: bool,
    settings_received: bool,
    pub peer_settings: PeerSettings,

    streams: HashMap<u64, StreamState>,
    pending_events: Vec<H3Event>,

    next_uni_stream_id: u64,
    next_bidi_stream_id: u64,
}

impl H3Connection {
    pub fn new(is_server: bool, config: Config) -> Self {
        let next_uni_stream_id = if is_server { 3 } else { 2 };
        let next_bidi_stream_id = if is_server { 1 } else { 0 };

        Self {
            is_server,
            config,
            local_control_stream_id: None,
            local_encoder_stream_id: None,
            local_decoder_stream_id: None,
            peer_control_stream_id: None,
            settings_sent: false,
            settings_received: false,
            peer_settings: PeerSettings::default(),
            streams: HashMap::new(),
            pending_events: Vec::new(),
            next_uni_stream_id,
            next_bidi_stream_id,
        }
    }

    /// Generate the bytes needed to initialize H3 on the QUIC connection.
    /// Returns a list of (stream_id, data) pairs to send.
    pub fn initialize(&mut self) -> Result<Vec<(u64, Vec<u8>)>> {
        if self.settings_sent {
            return Ok(Vec::new());
        }

        let mut sends = Vec::new();

        let control_id = self.alloc_uni_stream_id();
        self.local_control_stream_id = Some(control_id);
        let mut control_data = Vec::new();
        let mut tmp = [0u8; 8];
        let n = varint::encode(crate::frame::H3_STREAM_TYPE_CONTROL, &mut tmp)?;
        control_data.extend_from_slice(&tmp[..n]);
        let settings_frame = frame::encode_settings(&self.config.h3_settings())?;
        control_data.extend_from_slice(&settings_frame);
        sends.push((control_id, control_data));

        let encoder_id = self.alloc_uni_stream_id();
        self.local_encoder_stream_id = Some(encoder_id);
        let mut encoder_data = Vec::new();
        let n = varint::encode(crate::frame::H3_STREAM_TYPE_QPACK_ENCODER, &mut tmp)?;
        encoder_data.extend_from_slice(&tmp[..n]);
        sends.push((encoder_id, encoder_data));

        let decoder_id = self.alloc_uni_stream_id();
        self.local_decoder_stream_id = Some(decoder_id);
        let mut decoder_data = Vec::new();
        let n = varint::encode(crate::frame::H3_STREAM_TYPE_QPACK_DECODER, &mut tmp)?;
        decoder_data.extend_from_slice(&tmp[..n]);
        sends.push((decoder_id, decoder_data));

        self.settings_sent = true;
        Ok(sends)
    }

    pub fn send_connect_request(
        &mut self,
        authority: &str,
        path: &str,
        origin: Option<&str>,
        extra_headers: &[(&str, &str)],
    ) -> Result<(u64, Vec<u8>)> {
        let stream_id = self.alloc_bidi_stream_id();

        let mut headers = vec![
            (":method", "CONNECT"),
            (":protocol", "webtransport"),
            (":scheme", "https"),
            (":authority", authority),
            (":path", path),
        ];

        if let Some(o) = origin {
            headers.push(("origin", o));
        }

        for &(name, value) in extra_headers {
            headers.push((name, value));
        }

        let header_block = qpack::encode_header_block(&headers);
        let h3_frame = frame::H3Frame {
            frame_type: crate::frame::H3_FRAME_HEADERS,
            payload: header_block,
        };
        let mut buf = Vec::new();
        h3_frame.encode(&mut buf)?;

        self.streams.insert(
            stream_id,
            StreamState {
                role: StreamRole::Request,
                frame_parser: frame::FrameParser::new(),
            },
        );

        Ok((stream_id, buf))
    }

    pub fn send_connect_response(
        &mut self,
        _stream_id: u64,
        status: u16,
        extra_headers: &[(&str, &str)],
    ) -> Result<Vec<u8>> {
        let status_str = status.to_string();
        let mut headers: Vec<(&str, &str)> = vec![(":status", &status_str)];

        for &(name, value) in extra_headers {
            headers.push((name, value));
        }

        let header_block = qpack::encode_header_block(&headers);
        let h3_frame = frame::H3Frame {
            frame_type: crate::frame::H3_FRAME_HEADERS,
            payload: header_block,
        };
        let mut buf = Vec::new();
        h3_frame.encode(&mut buf)?;
        Ok(buf)
    }

    pub fn encode_body_as_h3_data(&self, data: &[u8]) -> Result<Vec<u8>> {
        let frame = frame::H3Frame {
            frame_type: crate::frame::H3_FRAME_DATA,
            payload: data.to_vec(),
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf)?;
        Ok(buf)
    }

    /// Process incoming data on a known H3 stream. Returns true if
    /// this stream is managed by H3 (control/qpack/request).
    pub fn process_stream_data(&mut self, stream_id: u64, data: &[u8], fin: bool) -> Result<bool> {
        let stream = match self.streams.get_mut(&stream_id) {
            Some(s) => s,
            None => return Ok(false),
        };

        let frames = stream.frame_parser.feed(data);
        let role = stream.role;

        for frame_result in frames {
            let frame = frame_result?;
            self.process_frame(stream_id, role, &frame)?;
        }

        if fin {
            if role == StreamRole::Request {
                self.pending_events.push(H3Event::StreamFinished(stream_id));
            }
        }

        Ok(true)
    }

    pub fn drain_events(&mut self) -> Vec<H3Event> {
        std::mem::take(&mut self.pending_events)
    }

    pub fn has_settings(&self) -> bool {
        self.settings_received
    }

    /// Try to classify a new incoming unidirectional stream based on
    /// its first byte(s). Returns the stream type value if classification
    /// is possible, or None if more data is needed.
    pub fn classify_uni_stream(&mut self, stream_id: u64, data: &[u8]) -> Result<Option<u64>> {
        if data.is_empty() {
            return Ok(None);
        }

        let (stream_type, type_len) = varint::decode(data)?;

        match stream_type {
            crate::frame::H3_STREAM_TYPE_CONTROL => {
                if self.peer_control_stream_id.is_some() {
                    return Err(Error::H3FrameError("duplicate control stream".into()));
                }
                self.peer_control_stream_id = Some(stream_id);
                self.streams.insert(
                    stream_id,
                    StreamState {
                        role: StreamRole::Control,
                        frame_parser: frame::FrameParser::new(),
                    },
                );
                if data.len() > type_len {
                    self.process_stream_data(stream_id, &data[type_len..], false)?;
                }
                Ok(Some(stream_type))
            }
            crate::frame::H3_STREAM_TYPE_QPACK_ENCODER => {
                self.streams.insert(
                    stream_id,
                    StreamState {
                        role: StreamRole::QpackEncoder,
                        frame_parser: frame::FrameParser::new(),
                    },
                );
                Ok(Some(stream_type))
            }
            crate::frame::H3_STREAM_TYPE_QPACK_DECODER => {
                self.streams.insert(
                    stream_id,
                    StreamState {
                        role: StreamRole::QpackDecoder,
                        frame_parser: frame::FrameParser::new(),
                    },
                );
                Ok(Some(stream_type))
            }
            _ => Ok(Some(stream_type)),
        }
    }

    pub fn register_request_stream(&mut self, stream_id: u64) {
        self.streams.insert(
            stream_id,
            StreamState {
                role: StreamRole::Request,
                frame_parser: frame::FrameParser::new(),
            },
        );
    }

    pub fn is_h3_stream(&self, stream_id: u64) -> bool {
        self.streams.contains_key(&stream_id)
    }

    fn process_frame(
        &mut self,
        stream_id: u64,
        role: StreamRole,
        h3_frame: &frame::H3Frame,
    ) -> Result<()> {
        match (role, h3_frame.frame_type) {
            (StreamRole::Control, crate::frame::H3_FRAME_SETTINGS) => {
                let settings = frame::decode_settings(&h3_frame.payload)?;
                self.peer_settings.apply(&settings);
                self.settings_received = true;
                self.pending_events
                    .push(H3Event::SettingsReceived(self.peer_settings.clone()));
            }
            (StreamRole::Control, crate::frame::H3_FRAME_GOAWAY) => {
                let (go_stream_id, _) = varint::decode(&h3_frame.payload)?;
                self.pending_events.push(H3Event::GoAway {
                    stream_id: go_stream_id,
                });
            }
            (StreamRole::Request, crate::frame::H3_FRAME_HEADERS) => {
                let headers = qpack::decode_header_block(&h3_frame.payload)?;
                self.pending_events
                    .push(H3Event::HeadersReceived { stream_id, headers });
            }
            (StreamRole::Request, crate::frame::H3_FRAME_DATA) => {
                self.pending_events.push(H3Event::DataReceived {
                    stream_id,
                    data: h3_frame.payload.clone(),
                });
            }
            _ => {
                // Unknown frame types are ignored per H3 spec
            }
        }
        Ok(())
    }

    fn alloc_uni_stream_id(&mut self) -> u64 {
        let id = self.next_uni_stream_id;
        self.next_uni_stream_id += 4;
        id
    }

    fn alloc_bidi_stream_id(&mut self) -> u64 {
        let id = self.next_bidi_stream_id;
        self.next_bidi_stream_id += 4;
        id
    }
}
