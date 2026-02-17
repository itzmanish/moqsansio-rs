use crate::codec::{write_varint, Cursor, Decode, Encode};
use crate::error::Result;
use crate::types::{
    decode_key_value_pairs, encode_key_value_pairs, KeyValuePair, KvValue, Location,
    SubscriptionFilter,
};

pub const PARAM_PATH: u64 = 0x01;
pub const PARAM_DELIVERY_TIMEOUT: u64 = 0x02;
pub const PARAM_AUTHORIZATION_TOKEN: u64 = 0x03;
pub const PARAM_MAX_AUTH_TOKEN_CACHE_SIZE: u64 = 0x04;
pub const PARAM_AUTHORITY: u64 = 0x05;
pub const PARAM_MOQT_IMPLEMENTATION: u64 = 0x07;
pub const PARAM_EXPIRES: u64 = 0x08;
pub const PARAM_LARGEST_OBJECT: u64 = 0x09;
pub const PARAM_MAX_REQUEST_ID: u64 = 0x02;
pub const PARAM_FORWARD: u64 = 0x10;
pub const PARAM_SUBSCRIBER_PRIORITY: u64 = 0x20;
pub const PARAM_SUBSCRIPTION_FILTER: u64 = 0x21;
pub const PARAM_GROUP_ORDER: u64 = 0x22;
pub const PARAM_NEW_GROUP_REQUEST: u64 = 0x32;

#[derive(Debug, Clone, Eq, Default)]
pub struct Parameters(pub Vec<KeyValuePair>);

impl PartialEq for Parameters {
    fn eq(&self, other: &Self) -> bool {
        self.sorted() == other.sorted()
    }
}

impl Parameters {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn add_varint(&mut self, key: u64, value: u64) {
        debug_assert!(key % 2 == 0, "varint parameters must have even key");
        self.0.push(KeyValuePair {
            key,
            value: KvValue::Varint(value),
        });
    }

    pub fn add_bytes(&mut self, key: u64, value: Vec<u8>) {
        debug_assert!(key % 2 == 1, "bytes parameters must have odd key");
        self.0.push(KeyValuePair {
            key,
            value: KvValue::Bytes(value),
        });
    }

    pub fn get_varint(&self, key: u64) -> Option<u64> {
        self.0.iter().find_map(|p| {
            if p.key == key {
                match &p.value {
                    KvValue::Varint(v) => Some(*v),
                    _ => None,
                }
            } else {
                None
            }
        })
    }

    pub fn get_bytes(&self, key: u64) -> Option<&[u8]> {
        self.0.iter().find_map(|p| {
            if p.key == key {
                match &p.value {
                    KvValue::Bytes(v) => Some(v.as_slice()),
                    _ => None,
                }
            } else {
                None
            }
        })
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    fn sorted(&self) -> Vec<KeyValuePair> {
        let mut sorted = self.0.clone();
        sorted.sort_by_key(|p| p.key);
        sorted
    }
}

impl Encode for Parameters {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        let sorted = self.sorted();
        write_varint(buf, sorted.len() as u64)?;
        encode_key_value_pairs(buf, &sorted)?;
        Ok(())
    }
}

impl Decode for Parameters {
    fn decode(cursor: &mut Cursor<'_>) -> Result<Self> {
        let count = cursor.read_varint()? as usize;
        let pairs = decode_key_value_pairs(cursor, count)?;
        Ok(Self(pairs))
    }
}

impl Parameters {
    pub fn get_subscription_filter(&self) -> Result<Option<SubscriptionFilter>> {
        match self.get_bytes(PARAM_SUBSCRIPTION_FILTER) {
            Some(bytes) => {
                let mut c = Cursor::new(bytes);
                let filter = SubscriptionFilter::decode(&mut c)?;
                Ok(Some(filter))
            }
            None => Ok(None),
        }
    }

    pub fn set_subscription_filter(&mut self, filter: &SubscriptionFilter) -> Result<()> {
        let bytes = filter.encode_to_vec()?;
        self.0.retain(|p| p.key != PARAM_SUBSCRIPTION_FILTER);
        self.add_bytes(PARAM_SUBSCRIPTION_FILTER, bytes);
        Ok(())
    }

    pub fn get_largest_object(&self) -> Result<Option<Location>> {
        match self.get_bytes(PARAM_LARGEST_OBJECT) {
            Some(bytes) => {
                let mut c = Cursor::new(bytes);
                let loc = Location::decode(&mut c)?;
                Ok(Some(loc))
            }
            None => Ok(None),
        }
    }

    pub fn set_largest_object(&mut self, loc: &Location) -> Result<()> {
        let bytes = loc.encode_to_vec()?;
        self.0.retain(|p| p.key != PARAM_LARGEST_OBJECT);
        self.add_bytes(PARAM_LARGEST_OBJECT, bytes);
        Ok(())
    }

    pub fn get_forward(&self) -> Option<bool> {
        self.get_varint(PARAM_FORWARD).map(|v| v != 0)
    }

    pub fn set_forward(&mut self, forward: bool) {
        self.0.retain(|p| p.key != PARAM_FORWARD);
        self.add_varint(PARAM_FORWARD, if forward { 1 } else { 0 });
    }

    pub fn get_subscriber_priority(&self) -> Option<u8> {
        self.get_varint(PARAM_SUBSCRIBER_PRIORITY).map(|v| v as u8)
    }

    pub fn set_subscriber_priority(&mut self, priority: u8) {
        self.0.retain(|p| p.key != PARAM_SUBSCRIBER_PRIORITY);
        self.add_varint(PARAM_SUBSCRIBER_PRIORITY, priority as u64);
    }

    pub fn get_delivery_timeout(&self) -> Option<u64> {
        self.get_varint(PARAM_DELIVERY_TIMEOUT)
    }

    pub fn set_delivery_timeout(&mut self, timeout_ms: u64) {
        self.0.retain(|p| p.key != PARAM_DELIVERY_TIMEOUT);
        self.add_varint(PARAM_DELIVERY_TIMEOUT, timeout_ms);
    }

    pub fn get_expires(&self) -> Option<u64> {
        self.get_varint(PARAM_EXPIRES)
    }

    pub fn set_expires(&mut self, expires_ms: u64) {
        self.0.retain(|p| p.key != PARAM_EXPIRES);
        self.add_varint(PARAM_EXPIRES, expires_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_roundtrip() {
        let mut params = Parameters::new();
        params.add_varint(PARAM_DELIVERY_TIMEOUT, 5000);
        params.add_varint(PARAM_FORWARD, 1);
        params.add_varint(PARAM_SUBSCRIBER_PRIORITY, 128);

        let buf = params.encode_to_vec().unwrap();
        let mut c = Cursor::new(&buf);
        let decoded = Parameters::decode(&mut c).unwrap();
        assert_eq!(c.remaining(), 0);

        assert_eq!(decoded.get_varint(PARAM_DELIVERY_TIMEOUT), Some(5000));
        assert_eq!(decoded.get_varint(PARAM_FORWARD), Some(1));
        assert_eq!(decoded.get_varint(PARAM_SUBSCRIBER_PRIORITY), Some(128));
    }

    #[test]
    fn parameters_with_filter() {
        let mut params = Parameters::new();
        let filter = SubscriptionFilter::AbsoluteStart {
            start: Location {
                group: 5,
                object: 0,
            },
        };
        params.set_subscription_filter(&filter).unwrap();

        let buf = params.encode_to_vec().unwrap();
        let mut c = Cursor::new(&buf);
        let decoded = Parameters::decode(&mut c).unwrap();
        assert_eq!(decoded.get_subscription_filter().unwrap(), Some(filter));
    }

    #[test]
    fn parameters_empty() {
        let params = Parameters::new();
        let buf = params.encode_to_vec().unwrap();
        let mut c = Cursor::new(&buf);
        let decoded = Parameters::decode(&mut c).unwrap();
        assert!(decoded.is_empty());
    }
}
