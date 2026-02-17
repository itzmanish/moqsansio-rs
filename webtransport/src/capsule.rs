use crate::error::{Error, Result};
use crate::frame;
use crate::varint;

const MAX_CLOSE_MESSAGE_LEN: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capsule {
    CloseSession {
        error_code: u32,
        error_message: String,
    },
    DrainSession,
    MaxStreamsBidi(u64),
    MaxStreamsUni(u64),
    StreamsBlockedBidi(u64),
    StreamsBlockedUni(u64),
    MaxData(u64),
    DataBlocked(u64),
}

impl Capsule {
    pub fn capsule_type(&self) -> u64 {
        match self {
            Capsule::CloseSession { .. } => frame::CAPSULE_CLOSE_SESSION,
            Capsule::DrainSession => frame::CAPSULE_DRAIN_SESSION,
            Capsule::MaxStreamsBidi(_) => frame::CAPSULE_MAX_STREAMS_BIDI,
            Capsule::MaxStreamsUni(_) => frame::CAPSULE_MAX_STREAMS_UNI,
            Capsule::StreamsBlockedBidi(_) => frame::CAPSULE_STREAMS_BLOCKED_BIDI,
            Capsule::StreamsBlockedUni(_) => frame::CAPSULE_STREAMS_BLOCKED_UNI,
            Capsule::MaxData(_) => frame::CAPSULE_MAX_DATA,
            Capsule::DataBlocked(_) => frame::CAPSULE_DATA_BLOCKED,
        }
    }

    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<usize> {
        let start = buf.len();

        let mut type_buf = [0u8; 8];
        let type_len = varint::encode(self.capsule_type(), &mut type_buf)?;
        buf.extend_from_slice(&type_buf[..type_len]);

        match self {
            Capsule::CloseSession {
                error_code,
                error_message,
            } => {
                if error_message.len() > MAX_CLOSE_MESSAGE_LEN {
                    return Err(Error::CloseMessageTooLong);
                }
                let payload_len = 4 + error_message.len();
                let mut len_buf = [0u8; 8];
                let len_n = varint::encode(payload_len as u64, &mut len_buf)?;
                buf.extend_from_slice(&len_buf[..len_n]);
                buf.extend_from_slice(&error_code.to_be_bytes());
                buf.extend_from_slice(error_message.as_bytes());
            }
            Capsule::DrainSession => {
                buf.push(0x00);
            }
            Capsule::MaxStreamsBidi(v)
            | Capsule::MaxStreamsUni(v)
            | Capsule::StreamsBlockedBidi(v)
            | Capsule::StreamsBlockedUni(v)
            | Capsule::MaxData(v)
            | Capsule::DataBlocked(v) => {
                let value_len = varint::varint_len(*v);
                let mut len_buf = [0u8; 8];
                let len_n = varint::encode(value_len as u64, &mut len_buf)?;
                buf.extend_from_slice(&len_buf[..len_n]);

                let mut val_buf = [0u8; 8];
                let val_n = varint::encode(*v, &mut val_buf)?;
                buf.extend_from_slice(&val_buf[..val_n]);
            }
        }

        Ok(buf.len() - start)
    }

    /// Decode a capsule from `buf`. Returns `(capsule, bytes_consumed)`.
    pub fn decode(buf: &[u8]) -> Result<(Self, usize)> {
        let mut offset = 0;

        let (capsule_type, n) = varint::decode(&buf[offset..])?;
        offset += n;

        let (payload_len, n) = varint::decode(&buf[offset..])?;
        offset += n;
        let payload_len = payload_len as usize;

        if buf.len() < offset + payload_len {
            return Err(Error::BufferTooShort);
        }

        let payload = &buf[offset..offset + payload_len];

        let capsule = match capsule_type {
            frame::CAPSULE_CLOSE_SESSION => {
                if payload_len < 4 {
                    return Err(Error::CapsuleError(
                        "CLOSE_SESSION payload too short".into(),
                    ));
                }
                let error_code =
                    u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let msg_bytes = &payload[4..];
                if msg_bytes.len() > MAX_CLOSE_MESSAGE_LEN {
                    return Err(Error::CloseMessageTooLong);
                }
                let error_message = String::from_utf8(msg_bytes.to_vec()).map_err(|_| {
                    Error::CapsuleError("CLOSE_SESSION message is not valid UTF-8".into())
                })?;
                Capsule::CloseSession {
                    error_code,
                    error_message,
                }
            }

            frame::CAPSULE_DRAIN_SESSION => {
                if payload_len != 0 {
                    return Err(Error::CapsuleError(
                        "DRAIN_SESSION must have empty payload".into(),
                    ));
                }
                Capsule::DrainSession
            }

            frame::CAPSULE_MAX_STREAMS_BIDI => {
                let (v, _) = varint::decode(payload)?;
                Capsule::MaxStreamsBidi(v)
            }
            frame::CAPSULE_MAX_STREAMS_UNI => {
                let (v, _) = varint::decode(payload)?;
                Capsule::MaxStreamsUni(v)
            }
            frame::CAPSULE_STREAMS_BLOCKED_BIDI => {
                let (v, _) = varint::decode(payload)?;
                Capsule::StreamsBlockedBidi(v)
            }
            frame::CAPSULE_STREAMS_BLOCKED_UNI => {
                let (v, _) = varint::decode(payload)?;
                Capsule::StreamsBlockedUni(v)
            }
            frame::CAPSULE_MAX_DATA => {
                let (v, _) = varint::decode(payload)?;
                Capsule::MaxData(v)
            }
            frame::CAPSULE_DATA_BLOCKED => {
                let (v, _) = varint::decode(payload)?;
                Capsule::DataBlocked(v)
            }

            _ => {
                return Err(Error::CapsuleError(format!(
                    "unknown capsule type: {capsule_type:#x}"
                )));
            }
        };

        Ok((capsule, offset + payload_len))
    }
}

/// Incremental capsule parser for reading capsules from a byte stream.
#[derive(Debug)]
pub struct CapsuleParser {
    buf: Vec<u8>,
}

impl CapsuleParser {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feed bytes and try to extract complete capsules.
    pub fn feed(&mut self, data: &[u8]) -> Vec<Result<Capsule>> {
        self.buf.extend_from_slice(data);
        let mut results = Vec::new();

        loop {
            match Capsule::decode(&self.buf) {
                Ok((capsule, consumed)) => {
                    self.buf.drain(..consumed);
                    results.push(Ok(capsule));
                }
                Err(Error::BufferTooShort) => break,
                Err(e) => {
                    results.push(Err(e));
                    break;
                }
            }
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(capsule: &Capsule) {
        let mut buf = Vec::new();
        capsule.encode(&mut buf).unwrap();
        let (decoded, consumed) = Capsule::decode(&buf).unwrap();
        assert_eq!(consumed, buf.len());
        assert_eq!(&decoded, capsule);
    }

    #[test]
    fn close_session_roundtrip() {
        roundtrip(&Capsule::CloseSession {
            error_code: 42,
            error_message: "goodbye".into(),
        });
    }

    #[test]
    fn close_session_empty_message() {
        roundtrip(&Capsule::CloseSession {
            error_code: 0,
            error_message: String::new(),
        });
    }

    #[test]
    fn drain_session_roundtrip() {
        roundtrip(&Capsule::DrainSession);
    }

    #[test]
    fn max_streams_roundtrip() {
        roundtrip(&Capsule::MaxStreamsBidi(100));
        roundtrip(&Capsule::MaxStreamsUni(200));
    }

    #[test]
    fn streams_blocked_roundtrip() {
        roundtrip(&Capsule::StreamsBlockedBidi(50));
        roundtrip(&Capsule::StreamsBlockedUni(75));
    }

    #[test]
    fn max_data_roundtrip() {
        roundtrip(&Capsule::MaxData(1_000_000));
    }

    #[test]
    fn data_blocked_roundtrip() {
        roundtrip(&Capsule::DataBlocked(500_000));
    }

    #[test]
    fn close_message_too_long() {
        let capsule = Capsule::CloseSession {
            error_code: 0,
            error_message: "x".repeat(1025),
        };
        let mut buf = Vec::new();
        assert!(matches!(
            capsule.encode(&mut buf),
            Err(Error::CloseMessageTooLong)
        ));
    }

    #[test]
    fn incremental_parser() {
        let c1 = Capsule::MaxData(42);
        let c2 = Capsule::DrainSession;

        let mut buf = Vec::new();
        c1.encode(&mut buf).unwrap();
        c2.encode(&mut buf).unwrap();

        let mut parser = CapsuleParser::new();

        // Feed one byte at a time
        let mut all_capsules = Vec::new();
        for &b in &buf {
            let results = parser.feed(&[b]);
            for r in results {
                all_capsules.push(r.unwrap());
            }
        }

        assert_eq!(all_capsules.len(), 2);
        assert_eq!(all_capsules[0], c1);
        assert_eq!(all_capsules[1], c2);
    }
}
