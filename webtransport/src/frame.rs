//! Wire constants from draft-ietf-webtrans-http3-14.
//!
//! All numeric codepoints defined in the RFC are collected here to avoid
//! magic numbers scattered across the codebase.

// ---------------------------------------------------------------------------
// HTTP/3 SETTINGS parameters (§9.2)
// ---------------------------------------------------------------------------

/// Maximum number of concurrent WebTransport sessions the endpoint will
/// accept. Default 0 = endpoint does not support WebTransport.
pub const SETTINGS_WT_MAX_SESSIONS: u64 = 0x14e9cd29;

/// Initial value for unidirectional session-level stream limit.
pub const SETTINGS_WT_INITIAL_MAX_STREAMS_UNI: u64 = 0x2b64;

/// Initial value for bidirectional session-level stream limit.
pub const SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI: u64 = 0x2b65;

/// Initial value for the session-level data limit.
pub const SETTINGS_WT_INITIAL_MAX_DATA: u64 = 0x2b61;

/// Enable extended CONNECT protocol (RFC 9220).
pub const SETTINGS_ENABLE_CONNECT_PROTOCOL: u64 = 0x08;

/// HTTP/3 datagrams (RFC 9297).
pub const SETTINGS_H3_DATAGRAM: u64 = 0x33;

/// QPACK max dynamic table capacity.
pub const SETTINGS_QPACK_MAX_TABLE_CAPACITY: u64 = 0x01;

/// QPACK max blocked streams.
pub const SETTINGS_QPACK_BLOCKED_STREAMS: u64 = 0x07;

/// Max header list size.
pub const SETTINGS_MAX_FIELD_SECTION_SIZE: u64 = 0x06;

// ---------------------------------------------------------------------------
// HTTP/3 frame types
// ---------------------------------------------------------------------------

/// DATA frame.
pub const H3_FRAME_DATA: u64 = 0x00;

/// HEADERS frame.
pub const H3_FRAME_HEADERS: u64 = 0x01;

/// SETTINGS frame.
pub const H3_FRAME_SETTINGS: u64 = 0x04;

/// GOAWAY frame.
pub const H3_FRAME_GOAWAY: u64 = 0x07;

// ---------------------------------------------------------------------------
// HTTP/3 unidirectional stream types
// ---------------------------------------------------------------------------

/// H3 control stream.
pub const H3_STREAM_TYPE_CONTROL: u64 = 0x00;

/// H3 push stream.
pub const H3_STREAM_TYPE_PUSH: u64 = 0x01;

/// QPACK encoder stream.
pub const H3_STREAM_TYPE_QPACK_ENCODER: u64 = 0x02;

/// QPACK decoder stream.
pub const H3_STREAM_TYPE_QPACK_DECODER: u64 = 0x03;

// ---------------------------------------------------------------------------
// WebTransport stream types & signals (§4.2, §4.3, §9.3, §9.4)
// ---------------------------------------------------------------------------

/// Unidirectional WebTransport stream type (§4.2).
pub const WT_UNI_STREAM_TYPE: u64 = 0x54;

/// Bidirectional WebTransport stream signal value (§4.3).
pub const WT_BIDI_SIGNAL: u64 = 0x41;

// ---------------------------------------------------------------------------
// HTTP Capsule types (§9.6)
// ---------------------------------------------------------------------------

/// Close the WebTransport session with an error code and message.
pub const CAPSULE_CLOSE_SESSION: u64 = 0x2843;

/// Request graceful session shutdown.
pub const CAPSULE_DRAIN_SESSION: u64 = 0x78ae;

/// Maximum session data (§5.6.4).
pub const CAPSULE_MAX_DATA: u64 = 0x190B4D3D;

/// Maximum bidirectional streams in a session (§5.6.2).
pub const CAPSULE_MAX_STREAMS_BIDI: u64 = 0x190B4D3F;

/// Maximum unidirectional streams in a session (§5.6.2).
pub const CAPSULE_MAX_STREAMS_UNI: u64 = 0x190B4D40;

/// Sender is blocked on session data limit (§5.6.5).
pub const CAPSULE_DATA_BLOCKED: u64 = 0x190B4D41;

/// Sender is blocked on bidirectional stream limit (§5.6.3).
pub const CAPSULE_STREAMS_BLOCKED_BIDI: u64 = 0x190B4D43;

/// Sender is blocked on unidirectional stream limit (§5.6.3).
pub const CAPSULE_STREAMS_BLOCKED_UNI: u64 = 0x190B4D44;

// ---------------------------------------------------------------------------
// HTTP/3 error codes (§9.5)
// ---------------------------------------------------------------------------

/// Data stream rejected because no session could be associated.
pub const WT_BUFFERED_STREAM_REJECTED: u64 = 0x3994bd84;

/// Data stream aborted because the session is gone.
pub const WT_SESSION_GONE: u64 = 0x170d7b68;

/// Session aborted due to flow-control violation.
pub const WT_FLOW_CONTROL_ERROR: u64 = 0x045d4487;

// ---------------------------------------------------------------------------
// WebTransport application error code range (§4.4)
// ---------------------------------------------------------------------------

/// First H3 error code reserved for WT application errors.
pub const WT_APP_ERROR_FIRST: u64 = 0x52e4a40fa8db;

/// Last H3 error code reserved for WT application errors.
pub const WT_APP_ERROR_LAST: u64 = 0x52e5ac983162;

/// H3 error: general frame error.
pub const H3_FRAME_ERROR: u64 = 0x0106;

/// H3 error: ID error.
pub const H3_ID_ERROR: u64 = 0x0108;

/// H3 error: settings error.
pub const H3_SETTINGS_ERROR: u64 = 0x0109;

/// H3 error: request rejected.
pub const H3_REQUEST_REJECTED: u64 = 0x010b;

/// H3 error: message error.
pub const H3_MESSAGE_ERROR: u64 = 0x010e;

/// PRIORITY_UPDATE frame for request streams (RFC 9218).
pub const H3_FRAME_PRIORITY_UPDATE: u64 = 0x0f;
