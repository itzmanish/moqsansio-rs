use crate::error::{Error, Result};
use crate::varint;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H3Frame {
    pub frame_type: u64,
    pub payload: Vec<u8>,
}

impl H3Frame {
    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<usize> {
        let start = buf.len();

        let mut tmp = [0u8; 8];
        let n = varint::encode(self.frame_type, &mut tmp)?;
        buf.extend_from_slice(&tmp[..n]);

        let n = varint::encode(self.payload.len() as u64, &mut tmp)?;
        buf.extend_from_slice(&tmp[..n]);

        buf.extend_from_slice(&self.payload);
        Ok(buf.len() - start)
    }

    pub fn decode(buf: &[u8]) -> Result<(Self, usize)> {
        let mut offset = 0;

        let (frame_type, n) = varint::decode(&buf[offset..])?;
        offset += n;

        let (payload_len, n) = varint::decode(&buf[offset..])?;
        offset += n;
        let payload_len = payload_len as usize;

        if buf.len() < offset + payload_len {
            return Err(Error::BufferTooShort);
        }

        let payload = buf[offset..offset + payload_len].to_vec();
        offset += payload_len;

        Ok((
            H3Frame {
                frame_type,
                payload,
            },
            offset,
        ))
    }
}

pub fn encode_settings(settings: &[(u64, u64)]) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let mut tmp = [0u8; 8];

    for &(id, value) in settings {
        let n = varint::encode(id, &mut tmp)?;
        payload.extend_from_slice(&tmp[..n]);
        let n = varint::encode(value, &mut tmp)?;
        payload.extend_from_slice(&tmp[..n]);
    }

    let frame = H3Frame {
        frame_type: crate::frame::H3_FRAME_SETTINGS,
        payload,
    };
    let mut buf = Vec::new();
    frame.encode(&mut buf)?;
    Ok(buf)
}

pub fn decode_settings(payload: &[u8]) -> Result<Vec<(u64, u64)>> {
    let mut settings = Vec::new();
    let mut offset = 0;

    while offset < payload.len() {
        let (id, n) = varint::decode(&payload[offset..])?;
        offset += n;
        let (value, n) = varint::decode(&payload[offset..])?;
        offset += n;
        settings.push((id, value));
    }

    Ok(settings)
}

pub fn encode_goaway(stream_id: u64) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let mut tmp = [0u8; 8];
    let n = varint::encode(stream_id, &mut tmp)?;
    payload.extend_from_slice(&tmp[..n]);

    let frame = H3Frame {
        frame_type: crate::frame::H3_FRAME_GOAWAY,
        payload,
    };
    let mut buf = Vec::new();
    frame.encode(&mut buf)?;
    Ok(buf)
}

/// Incremental H3 frame parser for a single stream.
#[derive(Debug)]
pub struct FrameParser {
    buf: Vec<u8>,
}

impl FrameParser {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<Result<H3Frame>> {
        self.buf.extend_from_slice(data);
        let mut results = Vec::new();

        loop {
            match H3Frame::decode(&self.buf) {
                Ok((frame, consumed)) => {
                    self.buf.drain(..consumed);
                    results.push(Ok(frame));
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

    #[test]
    fn settings_roundtrip() {
        let settings = vec![(0x01, 4096), (0x06, 8192), (0x33, 1)];
        let encoded = encode_settings(&settings).unwrap();

        let (frame, consumed) = H3Frame::decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(frame.frame_type, crate::frame::H3_FRAME_SETTINGS);

        let decoded = decode_settings(&frame.payload).unwrap();
        assert_eq!(decoded, settings);
    }

    #[test]
    fn frame_roundtrip() {
        let frame = H3Frame {
            frame_type: 0x01,
            payload: vec![1, 2, 3, 4, 5],
        };
        let mut buf = Vec::new();
        frame.encode(&mut buf).unwrap();

        let (decoded, consumed) = H3Frame::decode(&buf).unwrap();
        assert_eq!(consumed, buf.len());
        assert_eq!(decoded, frame);
    }

    #[test]
    fn goaway_encode() {
        let encoded = encode_goaway(4).unwrap();
        let (frame, _) = H3Frame::decode(&encoded).unwrap();
        assert_eq!(frame.frame_type, crate::frame::H3_FRAME_GOAWAY);
        let (stream_id, _) = varint::decode(&frame.payload).unwrap();
        assert_eq!(stream_id, 4);
    }

    #[test]
    fn incremental_parser() {
        let f1 = H3Frame {
            frame_type: 0x04,
            payload: vec![0x33, 0x01],
        };
        let f2 = H3Frame {
            frame_type: 0x00,
            payload: vec![10, 20, 30],
        };

        let mut buf = Vec::new();
        f1.encode(&mut buf).unwrap();
        f2.encode(&mut buf).unwrap();

        let mut parser = FrameParser::new();
        let mut frames = Vec::new();
        for &b in &buf {
            for r in parser.feed(&[b]) {
                frames.push(r.unwrap());
            }
        }

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], f1);
        assert_eq!(frames[1], f2);
    }
}
