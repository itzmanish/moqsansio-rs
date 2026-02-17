//! Error types and WebTransport ↔ HTTP/3 error code mapping (§4.4).

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    // -- transport / framing ---------------------------------------------------
    #[error("buffer too short")]
    BufferTooShort,

    #[error("varint exceeds maximum value (2^62 - 1)")]
    VarintOverflow,

    #[error("incomplete data: need more bytes")]
    Incomplete,

    // -- H3 / QPACK ------------------------------------------------------------
    #[error("H3 frame error: {0}")]
    H3FrameError(String),

    #[error("H3 settings error: {0}")]
    H3SettingsError(String),

    #[error("QPACK decoding error: {0}")]
    QpackError(String),

    #[error("H3 connection closed: {0}")]
    H3ConnectionClosed(String),

    // -- WebTransport protocol --------------------------------------------------
    #[error("session not found: {0}")]
    SessionNotFound(u64),

    #[error("session already closed")]
    SessionClosed,

    #[error("session limit reached")]
    SessionLimitReached,

    #[error("flow control error")]
    FlowControlError,

    #[error("invalid session id: {0}")]
    InvalidSessionId(u64),

    #[error("stream not found: {0}")]
    StreamNotFound(u64),

    #[error("protocol violation: {0}")]
    ProtocolViolation(String),

    #[error("capsule decode error: {0}")]
    CapsuleError(String),

    #[error("close message too long (max 1024 bytes)")]
    CloseMessageTooLong,

    // -- operational ------------------------------------------------------------
    #[error("no more events")]
    Done,

    #[error("stream blocked on flow control")]
    StreamBlocked,

    #[error("transport error: {0}")]
    TransportError(String),
}

// ---------------------------------------------------------------------------
// §4.4  Application error code mapping
//
//   first = 0x52e4a40fa8db
//   last  = 0x52e5ac983162
//
//   webtransport_code_to_http_code(n):
//       return first + n + floor(n / 0x1e)
//
//   http_code_to_webtransport_code(h):
//       assert(first <= h <= last)
//       assert((h - 0x21) % 0x1f != 0)
//       shifted = h - first
//       return shifted - floor(shifted / 0x1f)
// ---------------------------------------------------------------------------

use crate::frame::{WT_APP_ERROR_FIRST, WT_APP_ERROR_LAST};

/// Convert a WebTransport application error code (0..0xFFFF_FFFF) to its
/// corresponding HTTP/3 error code in the reserved range.
pub fn webtransport_to_http3_error(n: u32) -> u64 {
    let n = n as u64;
    WT_APP_ERROR_FIRST + n + n / 0x1e
}

/// Convert an HTTP/3 error code back to a WebTransport application error code.
///
/// Returns `None` if `h` is outside the reserved range or falls on a
/// reserved greased codepoint.
pub fn http3_to_webtransport_error(h: u64) -> Option<u32> {
    if h < WT_APP_ERROR_FIRST || h > WT_APP_ERROR_LAST {
        return None;
    }
    // Skip reserved codepoints of form  0x1f * N + 0x21
    if (h.wrapping_sub(0x21)) % 0x1f == 0 {
        return None;
    }
    let shifted = h - WT_APP_ERROR_FIRST;
    let wt = shifted - shifted / 0x1f;
    if wt > u32::MAX as u64 {
        return None;
    }
    Some(wt as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_roundtrip() {
        // 0 → first codepoint
        assert_eq!(webtransport_to_http3_error(0), WT_APP_ERROR_FIRST);
        let rt = http3_to_webtransport_error(webtransport_to_http3_error(0));
        assert_eq!(rt, Some(0));

        // max → last codepoint
        assert_eq!(webtransport_to_http3_error(0xFFFF_FFFF), WT_APP_ERROR_LAST);
        let rt = http3_to_webtransport_error(webtransport_to_http3_error(0xFFFF_FFFF));
        assert_eq!(rt, Some(0xFFFF_FFFF));
    }

    #[test]
    fn error_code_roundtrip_samples() {
        for n in [0u32, 1, 29, 30, 31, 100, 1000, 0xFFFF, 0xFFFF_FFFF] {
            let h = webtransport_to_http3_error(n);
            assert!(h >= WT_APP_ERROR_FIRST);
            assert!(h <= WT_APP_ERROR_LAST);
            let back = http3_to_webtransport_error(h);
            assert_eq!(back, Some(n), "roundtrip failed for n={n}");
        }
    }

    #[test]
    fn reserved_codepoints_rejected() {
        // codepoints of form 0x1f * N + 0x21 inside the range should return None
        let greased = 0x1f * 0x2b32c4cf33u64 + 0x21;
        if greased >= WT_APP_ERROR_FIRST && greased <= WT_APP_ERROR_LAST {
            assert_eq!(http3_to_webtransport_error(greased), None);
        }
    }

    #[test]
    fn out_of_range_rejected() {
        assert_eq!(http3_to_webtransport_error(0), None);
        assert_eq!(http3_to_webtransport_error(WT_APP_ERROR_FIRST - 1), None);
        assert_eq!(http3_to_webtransport_error(WT_APP_ERROR_LAST + 1), None);
    }
}
