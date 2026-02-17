use crate::codec::{write_prefixed_bytes, write_varint, Cursor, Decode, Encode};
use crate::error::{Error, Result};

const MAX_TRACK_NAMESPACE_FIELDS: usize = 32;
const MAX_FULL_TRACK_NAME_BYTES: usize = 4096;
const MAX_REASON_PHRASE_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub group: u64,
    pub object: u64,
}

impl Encode for Location {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.group)?;
        write_varint(buf, self.object)?;
        Ok(())
    }
}

impl Decode for Location {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let group = cursor.read_varint()?;
        let object = cursor.read_varint()?;
        Ok(Self { group, object })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackNamespace {
    pub fields: Vec<Vec<u8>>,
}

impl TrackNamespace {
    pub fn new(fields: Vec<Vec<u8>>) -> Result<Self> {
        if fields.is_empty() || fields.len() > MAX_TRACK_NAMESPACE_FIELDS {
            return Err(Error::InvalidTrackNamespace(format!(
                "field count {} must be between 1 and {MAX_TRACK_NAMESPACE_FIELDS}",
                fields.len()
            )));
        }
        for f in &fields {
            if f.is_empty() {
                return Err(Error::InvalidTrackNamespace(
                    "namespace field must not be empty".into(),
                ));
            }
        }
        let total: usize = fields.iter().map(|f| f.len()).sum();
        if total > MAX_FULL_TRACK_NAME_BYTES {
            return Err(Error::InvalidTrackNamespace(format!(
                "total length {total} exceeds {MAX_FULL_TRACK_NAME_BYTES}"
            )));
        }
        Ok(Self { fields })
    }

    pub fn from_strings(parts: &[&str]) -> Result<Self> {
        let fields: Vec<Vec<u8>> = parts.iter().map(|s| s.as_bytes().to_vec()).collect();
        Self::new(fields)
    }

    pub fn is_prefix_of(&self, other: &TrackNamespace) -> bool {
        if self.fields.len() > other.fields.len() {
            return false;
        }
        self.fields
            .iter()
            .zip(other.fields.iter())
            .all(|(a, b)| a == b)
    }
}

impl Encode for TrackNamespace {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.fields.len() as u64)?;
        for field in &self.fields {
            write_prefixed_bytes(buf, field)?;
        }
        Ok(())
    }
}

impl Decode for TrackNamespace {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let count = cursor.read_varint()? as usize;
        if count == 0 || count > MAX_TRACK_NAMESPACE_FIELDS {
            return Err(Error::ProtocolViolation(format!(
                "track namespace field count {count} out of range 1..={MAX_TRACK_NAMESPACE_FIELDS}"
            )));
        }
        let mut fields = Vec::with_capacity(count);
        for _ in 0..count {
            let field = cursor.read_prefixed_bytes()?.to_vec();
            if field.is_empty() {
                return Err(Error::ProtocolViolation(
                    "track namespace field must not be empty".into(),
                ));
            }
            fields.push(field);
        }
        Ok(Self { fields })
    }
}

/// Track Namespace Prefix — used in SUBSCRIBE_NAMESPACE.
/// Unlike TrackNamespace, this allows 0 fields (empty prefix matches everything).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackNamespacePrefix {
    pub fields: Vec<Vec<u8>>,
}

impl Encode for TrackNamespacePrefix {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        write_varint(buf, self.fields.len() as u64)?;
        for field in &self.fields {
            write_prefixed_bytes(buf, field)?;
        }
        Ok(())
    }
}

impl Decode for TrackNamespacePrefix {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let count = cursor.read_varint()? as usize;
        if count > MAX_TRACK_NAMESPACE_FIELDS {
            return Err(Error::ProtocolViolation(format!(
                "track namespace prefix field count {count} exceeds {MAX_TRACK_NAMESPACE_FIELDS}"
            )));
        }
        let mut fields = Vec::with_capacity(count);
        for _ in 0..count {
            let field = cursor.read_prefixed_bytes()?.to_vec();
            fields.push(field);
        }
        Ok(Self { fields })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReasonPhrase(pub String);

impl Encode for ReasonPhrase {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        let bytes = self.0.as_bytes();
        if bytes.len() > MAX_REASON_PHRASE_BYTES {
            return Err(Error::ReasonPhraseTooLong(bytes.len()));
        }
        write_varint(buf, bytes.len() as u64)?;
        buf.extend_from_slice(bytes);
        Ok(())
    }
}

impl Decode for ReasonPhrase {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let len = cursor.read_varint()? as usize;
        if len > MAX_REASON_PHRASE_BYTES {
            return Err(Error::ProtocolViolation(format!(
                "reason phrase length {len} exceeds {MAX_REASON_PHRASE_BYTES}"
            )));
        }
        let bytes = cursor.read_bytes(len)?;
        let s = String::from_utf8(bytes.to_vec())
            .map_err(|_| Error::ProtocolViolation("reason phrase is not valid UTF-8".into()))?;
        Ok(Self(s))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterType {
    NextGroupStart,
    LargestObject,
    AbsoluteStart,
    AbsoluteRange,
}

impl FilterType {
    fn as_u64(&self) -> u64 {
        match self {
            Self::NextGroupStart => 0x1,
            Self::LargestObject => 0x2,
            Self::AbsoluteStart => 0x3,
            Self::AbsoluteRange => 0x4,
        }
    }

    fn from_u64(v: u64) -> Result<Self> {
        match v {
            0x1 => Ok(Self::NextGroupStart),
            0x2 => Ok(Self::LargestObject),
            0x3 => Ok(Self::AbsoluteStart),
            0x4 => Ok(Self::AbsoluteRange),
            _ => Err(Error::InvalidFilterType(v)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionFilter {
    NextGroupStart,
    LargestObject,
    AbsoluteStart { start: Location },
    AbsoluteRange { start: Location, end_group: u64 },
}

impl Encode for SubscriptionFilter {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::NextGroupStart => {
                write_varint(buf, FilterType::NextGroupStart.as_u64())?;
            }
            Self::LargestObject => {
                write_varint(buf, FilterType::LargestObject.as_u64())?;
            }
            Self::AbsoluteStart { start } => {
                write_varint(buf, FilterType::AbsoluteStart.as_u64())?;
                start.encode(buf)?;
            }
            Self::AbsoluteRange { start, end_group } => {
                write_varint(buf, FilterType::AbsoluteRange.as_u64())?;
                start.encode(buf)?;
                write_varint(buf, *end_group)?;
            }
        }
        Ok(())
    }
}

impl Decode for SubscriptionFilter {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let filter_type = FilterType::from_u64(cursor.read_varint()?)?;
        match filter_type {
            FilterType::NextGroupStart => Ok(Self::NextGroupStart),
            FilterType::LargestObject => Ok(Self::LargestObject),
            FilterType::AbsoluteStart => {
                let start = Location::decode(cursor)?;
                Ok(Self::AbsoluteStart { start })
            }
            FilterType::AbsoluteRange => {
                let start = Location::decode(cursor)?;
                let end_group = cursor.read_varint()?;
                Ok(Self::AbsoluteRange { start, end_group })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupOrder {
    Ascending,
    Descending,
}

impl GroupOrder {
    pub fn as_u64(&self) -> u64 {
        match self {
            Self::Ascending => 0x1,
            Self::Descending => 0x2,
        }
    }

    pub fn from_u64(v: u64) -> Result<Self> {
        match v {
            0x1 => Ok(Self::Ascending),
            0x2 => Ok(Self::Descending),
            _ => Err(Error::InvalidValue(format!("invalid group order: {v:#x}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeOption {
    Publish,
    Namespace,
    Both,
}

impl SubscribeOption {
    pub fn as_u64(&self) -> u64 {
        match self {
            Self::Publish => 0x00,
            Self::Namespace => 0x01,
            Self::Both => 0x02,
        }
    }

    pub fn from_u64(v: u64) -> Result<Self> {
        match v {
            0x00 => Ok(Self::Publish),
            0x01 => Ok(Self::Namespace),
            0x02 => Ok(Self::Both),
            _ => Err(Error::InvalidValue(format!(
                "invalid subscribe option: {v:#x}"
            ))),
        }
    }
}

/// Key-Value-Pair with delta-encoded types (Section 1.4.2).
///
/// The delta encoding is handled at the collection level (see `params.rs`),
/// so this struct stores the absolute type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyValuePair {
    pub key: u64,
    pub value: KvValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KvValue {
    Varint(u64),
    Bytes(Vec<u8>),
}

const MAX_KV_VALUE_BYTES: usize = (1 << 16) - 1;

pub fn encode_key_value_pairs(buf: &mut Vec<u8>, pairs: &[KeyValuePair]) -> Result<()> {
    let mut prev_type: u64 = 0;
    for pair in pairs {
        let delta = pair.key.checked_sub(prev_type).ok_or_else(|| {
            Error::KeyValueFormattingError(format!(
                "key-value types must be non-decreasing: {} after {}",
                pair.key, prev_type
            ))
        })?;
        write_varint(buf, delta)?;
        match &pair.value {
            KvValue::Varint(v) => {
                if pair.key % 2 != 0 {
                    return Err(Error::KeyValueFormattingError(
                        "varint value requires even type".into(),
                    ));
                }
                write_varint(buf, *v)?;
            }
            KvValue::Bytes(b) => {
                if pair.key % 2 != 1 {
                    return Err(Error::KeyValueFormattingError(
                        "bytes value requires odd type".into(),
                    ));
                }
                if b.len() > MAX_KV_VALUE_BYTES {
                    return Err(Error::KeyValueFormattingError(format!(
                        "value length {} exceeds max {}",
                        b.len(),
                        MAX_KV_VALUE_BYTES
                    )));
                }
                write_varint(buf, b.len() as u64)?;
                buf.extend_from_slice(b);
            }
        }
        prev_type = pair.key;
    }
    Ok(())
}

pub fn decode_key_value_pairs(cursor: &mut Cursor<'_>, count: usize) -> Result<Vec<KeyValuePair>> {
    let mut pairs = Vec::with_capacity(count);
    let mut prev_type: u64 = 0;
    for _ in 0..count {
        let delta = cursor.read_varint()?;
        let abs_type = prev_type
            .checked_add(delta)
            .ok_or_else(|| Error::ProtocolViolation("key-value delta type overflow".into()))?;
        let value = if abs_type % 2 == 0 {
            KvValue::Varint(cursor.read_varint()?)
        } else {
            let len = cursor.read_varint()? as usize;
            if len > MAX_KV_VALUE_BYTES {
                return Err(Error::ProtocolViolation(format!(
                    "key-value value length {len} exceeds max {MAX_KV_VALUE_BYTES}"
                )));
            }
            KvValue::Bytes(cursor.read_bytes(len)?.to_vec())
        };
        pairs.push(KeyValuePair {
            key: abs_type,
            value,
        });
        prev_type = abs_type;
    }
    Ok(pairs)
}

pub fn encode_extensions(buf: &mut Vec<u8>, extensions: &[KeyValuePair]) -> Result<()> {
    encode_key_value_pairs(buf, extensions)
}

pub fn decode_extensions_from_remaining(cursor: &mut Cursor<'_>) -> Result<Vec<KeyValuePair>> {
    let mut pairs = Vec::new();
    let mut prev_type: u64 = 0;
    while cursor.remaining() > 0 {
        let delta = cursor.read_varint()?;
        let abs_type = prev_type.checked_add(delta).ok_or_else(|| {
            Error::ProtocolViolation("extension header delta type overflow".into())
        })?;
        let value = if abs_type % 2 == 0 {
            KvValue::Varint(cursor.read_varint()?)
        } else {
            let len = cursor.read_varint()? as usize;
            if len > MAX_KV_VALUE_BYTES {
                return Err(Error::ProtocolViolation(format!(
                    "extension value length {len} exceeds max {MAX_KV_VALUE_BYTES}"
                )));
            }
            KvValue::Bytes(cursor.read_bytes(len)?.to_vec())
        };
        pairs.push(KeyValuePair {
            key: abs_type,
            value,
        });
        prev_type = abs_type;
    }
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_roundtrip() {
        let loc = Location {
            group: 42,
            object: 7,
        };
        let buf = loc.encode_to_vec().unwrap();
        let mut c = Cursor::new(&buf);
        let decoded = Location::decode(&mut c).unwrap();
        assert_eq!(loc, decoded);
        assert_eq!(c.remaining(), 0);
    }

    #[test]
    fn track_namespace_roundtrip() {
        let ns = TrackNamespace::from_strings(&["example.com", "meeting", "room1"]).unwrap();
        let buf = ns.encode_to_vec().unwrap();
        let mut c = Cursor::new(&buf);
        let decoded = TrackNamespace::decode(&mut c).unwrap();
        assert_eq!(ns, decoded);
        assert_eq!(c.remaining(), 0);
    }

    #[test]
    fn track_namespace_prefix_empty() {
        let prefix = TrackNamespacePrefix { fields: vec![] };
        let buf = prefix.encode_to_vec().unwrap();
        let mut c = Cursor::new(&buf);
        let decoded = TrackNamespacePrefix::decode(&mut c).unwrap();
        assert_eq!(prefix, decoded);
    }

    #[test]
    fn reason_phrase_roundtrip() {
        let rp = ReasonPhrase("something went wrong".into());
        let buf = rp.encode_to_vec().unwrap();
        let mut c = Cursor::new(&buf);
        let decoded = ReasonPhrase::decode(&mut c).unwrap();
        assert_eq!(rp, decoded);
    }

    #[test]
    fn subscription_filter_roundtrip() {
        let filters = vec![
            SubscriptionFilter::NextGroupStart,
            SubscriptionFilter::LargestObject,
            SubscriptionFilter::AbsoluteStart {
                start: Location {
                    group: 10,
                    object: 5,
                },
            },
            SubscriptionFilter::AbsoluteRange {
                start: Location {
                    group: 1,
                    object: 0,
                },
                end_group: 100,
            },
        ];
        for filter in filters {
            let buf = filter.encode_to_vec().unwrap();
            let mut c = Cursor::new(&buf);
            let decoded = SubscriptionFilter::decode(&mut c).unwrap();
            assert_eq!(filter, decoded);
            assert_eq!(c.remaining(), 0);
        }
    }

    #[test]
    fn key_value_pairs_roundtrip() {
        let pairs = vec![
            KeyValuePair {
                key: 2,
                value: KvValue::Varint(42),
            },
            KeyValuePair {
                key: 3,
                value: KvValue::Bytes(b"hello".to_vec()),
            },
            KeyValuePair {
                key: 8,
                value: KvValue::Varint(999),
            },
        ];
        let mut buf = Vec::new();
        encode_key_value_pairs(&mut buf, &pairs).unwrap();
        let mut c = Cursor::new(&buf);
        let decoded = decode_key_value_pairs(&mut c, 3).unwrap();
        assert_eq!(pairs, decoded);
        assert_eq!(c.remaining(), 0);
    }

    #[test]
    fn namespace_prefix_match() {
        let ns = TrackNamespace::from_strings(&["example.com", "meeting", "room1"]).unwrap();
        let prefix = TrackNamespace::from_strings(&["example.com", "meeting"]).unwrap();
        assert!(prefix.is_prefix_of(&ns));
        assert!(!ns.is_prefix_of(&prefix));
    }
}
