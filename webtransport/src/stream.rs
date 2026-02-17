use crate::error::{Error, Result};
use crate::frame;
use crate::varint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    UniDirectional,
    BiDirectional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderState {
    AwaitingSignal,
    AwaitingSessionId,
    Done,
}

#[derive(Debug, Clone)]
pub struct WtStream {
    pub quic_stream_id: u64,
    pub session_id: u64,
    pub stream_type: StreamType,
    pub header_len: u64,
    header_state: HeaderState,
    signal_decoder: varint::VarintDecoder,
    session_id_decoder: varint::VarintDecoder,
}

impl WtStream {
    pub fn new_outgoing(quic_stream_id: u64, session_id: u64, stream_type: StreamType) -> Self {
        Self {
            quic_stream_id,
            session_id,
            stream_type,
            header_len: 0,
            header_state: HeaderState::Done,
            signal_decoder: varint::VarintDecoder::new(),
            session_id_decoder: varint::VarintDecoder::new(),
        }
    }

    pub fn new_incoming(quic_stream_id: u64, stream_type: StreamType) -> Self {
        Self {
            quic_stream_id,
            session_id: 0,
            stream_type,
            header_len: 0,
            header_state: HeaderState::AwaitingSignal,
            signal_decoder: varint::VarintDecoder::new(),
            session_id_decoder: varint::VarintDecoder::new(),
        }
    }

    pub fn is_header_complete(&self) -> bool {
        self.header_state == HeaderState::Done
    }

    /// Feed bytes from the stream, consuming the WT header prefix.
    /// Returns the number of header bytes consumed from `data`. Any bytes
    /// beyond that belong to the application payload.
    pub fn parse_header(&mut self, data: &[u8]) -> Result<usize> {
        let mut consumed = 0;

        for &byte in data {
            match self.header_state {
                HeaderState::AwaitingSignal => {
                    if let Some((signal, n)) = self.signal_decoder.feed(byte) {
                        let expected = match self.stream_type {
                            StreamType::UniDirectional => frame::WT_UNI_STREAM_TYPE,
                            StreamType::BiDirectional => frame::WT_BIDI_SIGNAL,
                        };
                        if signal != expected {
                            return Err(Error::ProtocolViolation(format!(
                                "unexpected stream signal: {signal:#x}, expected {expected:#x}"
                            )));
                        }
                        self.header_len = n as u64;
                        self.header_state = HeaderState::AwaitingSessionId;
                    }
                    consumed += 1;
                }
                HeaderState::AwaitingSessionId => {
                    if let Some((sid, n)) = self.session_id_decoder.feed(byte) {
                        self.session_id = sid;
                        self.header_len += n as u64;
                        self.header_state = HeaderState::Done;
                        consumed += 1;
                        return Ok(consumed);
                    }
                    consumed += 1;
                }
                HeaderState::Done => return Ok(consumed),
            }
        }

        Ok(consumed)
    }

    /// Parse only the session ID portion (when the stream type/signal byte
    /// was already consumed during classification).
    pub fn parse_session_id_only(&mut self, data: &[u8]) -> Result<usize> {
        if self.header_state == HeaderState::AwaitingSignal {
            self.header_state = HeaderState::AwaitingSessionId;
        }

        let mut consumed = 0;
        for &byte in data {
            match self.header_state {
                HeaderState::AwaitingSessionId => {
                    if let Some((sid, n)) = self.session_id_decoder.feed(byte) {
                        self.session_id = sid;
                        self.header_len += n as u64;
                        self.header_state = HeaderState::Done;
                        consumed += 1;
                        return Ok(consumed);
                    }
                    consumed += 1;
                }
                HeaderState::Done => return Ok(consumed),
                HeaderState::AwaitingSignal => return Ok(consumed),
            }
        }
        Ok(consumed)
    }
}

/// Encode the WT stream header (signal + session_id) for an outgoing stream.
pub fn encode_stream_header(session_id: u64, stream_type: StreamType) -> Result<Vec<u8>> {
    let signal = match stream_type {
        StreamType::UniDirectional => frame::WT_UNI_STREAM_TYPE,
        StreamType::BiDirectional => frame::WT_BIDI_SIGNAL,
    };

    let mut buf = Vec::with_capacity(16);
    let mut tmp = [0u8; 8];

    let n = varint::encode(signal, &mut tmp)?;
    buf.extend_from_slice(&tmp[..n]);

    let n = varint::encode(session_id, &mut tmp)?;
    buf.extend_from_slice(&tmp[..n]);

    Ok(buf)
}

/// Classify a QUIC stream ID.  Returns true if the stream is client-initiated.
pub fn is_client_initiated(stream_id: u64) -> bool {
    stream_id & 0x01 == 0
}

/// Returns true if the QUIC stream is bidirectional.
pub fn is_bidi(stream_id: u64) -> bool {
    stream_id & 0x02 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_then_parse_uni() {
        let session_id = 42u64;
        let header = encode_stream_header(session_id, StreamType::UniDirectional).unwrap();

        let mut stream = WtStream::new_incoming(6, StreamType::UniDirectional);
        let consumed = stream.parse_header(&header).unwrap();
        assert_eq!(consumed, header.len());
        assert!(stream.is_header_complete());
        assert_eq!(stream.session_id, session_id);
    }

    #[test]
    fn encode_then_parse_bidi() {
        let session_id = 0u64;
        let header = encode_stream_header(session_id, StreamType::BiDirectional).unwrap();

        let mut stream = WtStream::new_incoming(4, StreamType::BiDirectional);
        let consumed = stream.parse_header(&header).unwrap();
        assert_eq!(consumed, header.len());
        assert!(stream.is_header_complete());
        assert_eq!(stream.session_id, session_id);
    }

    #[test]
    fn incremental_parsing() {
        let session_id = 16384u64;
        let header = encode_stream_header(session_id, StreamType::UniDirectional).unwrap();

        let mut stream = WtStream::new_incoming(6, StreamType::UniDirectional);
        let mut total = 0;
        for &b in &header {
            total += stream.parse_header(&[b]).unwrap();
            if stream.is_header_complete() {
                break;
            }
        }
        assert_eq!(total, header.len());
        assert_eq!(stream.session_id, session_id);
    }

    #[test]
    fn wrong_signal_rejected() {
        // Feed the varint-encoded bidi signal (value 0x41 = 65) to a uni stream.
        // 0x41 as a QUIC varint is 2 bytes: [0x40, 0x41] (top bits 01 = 2-byte).
        let mut stream = WtStream::new_incoming(6, StreamType::UniDirectional);
        let result = stream.parse_header(&[0x40, 0x41]);
        assert!(result.is_err());
    }

    #[test]
    fn stream_id_classification() {
        assert!(is_client_initiated(0));
        assert!(is_client_initiated(4));
        assert!(!is_client_initiated(1));
        assert!(!is_client_initiated(3));

        assert!(is_bidi(0));
        assert!(is_bidi(1));
        assert!(!is_bidi(2));
        assert!(!is_bidi(3));
    }
}
