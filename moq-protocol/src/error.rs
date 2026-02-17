use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("buffer too short")]
    BufferTooShort,

    #[error("varint exceeds maximum value (2^62 - 1)")]
    VarintOverflow,

    #[error("message payload exceeds maximum length (2^16 - 1 bytes)")]
    MessageTooLong,

    #[error("unknown message type: {0:#x}")]
    UnknownMessageType(u64),

    #[error("protocol violation: {0}")]
    ProtocolViolation(String),

    #[error("invalid track namespace: {0}")]
    InvalidTrackNamespace(String),

    #[error("invalid reason phrase: length {0} exceeds maximum 1024")]
    ReasonPhraseTooLong(usize),

    #[error("invalid filter type: {0:#x}")]
    InvalidFilterType(u64),

    #[error("invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("key-value formatting error: {0}")]
    KeyValueFormattingError(String),

    #[error("invalid token alias type: {0:#x}")]
    InvalidAliasType(u64),

    #[error("duplicate parameter type: {0:#x}")]
    DuplicateParameter(u64),

    #[error("invalid value: {0}")]
    InvalidValue(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum SessionError {
    NoError = 0x0,
    InternalError = 0x1,
    Unauthorized = 0x2,
    ProtocolViolation = 0x3,
    InvalidRequestId = 0x4,
    DuplicateTrackAlias = 0x5,
    KeyValueFormattingError = 0x6,
    TooManyRequests = 0x7,
    InvalidPath = 0x8,
    MalformedPath = 0x9,
    GoawayTimeout = 0x10,
    ControlMessageTimeout = 0x11,
    DataStreamTimeout = 0x12,
    AuthTokenCacheOverflow = 0x13,
    DuplicateAuthTokenAlias = 0x14,
    VersionNegotiationFailed = 0x15,
    MalformedAuthToken = 0x16,
    UnknownAuthTokenAlias = 0x17,
    ExpiredAuthToken = 0x18,
    InvalidAuthority = 0x19,
    MalformedAuthority = 0x1A,
}

impl SessionError {
    pub fn as_u64(self) -> u64 {
        self as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum RequestErrorCode {
    InternalError = 0x0,
    Unauthorized = 0x1,
    Timeout = 0x2,
    NotSupported = 0x3,
    MalformedAuthToken = 0x4,
    ExpiredAuthToken = 0x5,
    DoesNotExist = 0x10,
    InvalidRange = 0x11,
    MalformedTrack = 0x12,
    Uninterested = 0x15,
    PrefixOverlap = 0x16,
    InvalidJoiningRequestId = 0x17,
    DuplicateSubscription = 0x19,
}

impl RequestErrorCode {
    pub fn as_u64(self) -> u64 {
        self as u64
    }

    pub fn from_u64(v: u64) -> Option<Self> {
        match v {
            0x0 => Some(Self::InternalError),
            0x1 => Some(Self::Unauthorized),
            0x2 => Some(Self::Timeout),
            0x3 => Some(Self::NotSupported),
            0x4 => Some(Self::MalformedAuthToken),
            0x5 => Some(Self::ExpiredAuthToken),
            0x10 => Some(Self::DoesNotExist),
            0x11 => Some(Self::InvalidRange),
            0x12 => Some(Self::MalformedTrack),
            0x15 => Some(Self::Uninterested),
            0x16 => Some(Self::PrefixOverlap),
            0x17 => Some(Self::InvalidJoiningRequestId),
            0x19 => Some(Self::DuplicateSubscription),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum PublishDoneStatus {
    InternalError = 0x0,
    Unauthorized = 0x1,
    TrackEnded = 0x2,
    SubscriptionEnded = 0x3,
    GoingAway = 0x4,
    Expired = 0x5,
    TooFarBehind = 0x6,
    UpdateFailed = 0x8,
    MalformedTrack = 0x12,
}

impl PublishDoneStatus {
    pub fn as_u64(self) -> u64 {
        self as u64
    }

    pub fn from_u64(v: u64) -> Option<Self> {
        match v {
            0x0 => Some(Self::InternalError),
            0x1 => Some(Self::Unauthorized),
            0x2 => Some(Self::TrackEnded),
            0x3 => Some(Self::SubscriptionEnded),
            0x4 => Some(Self::GoingAway),
            0x5 => Some(Self::Expired),
            0x6 => Some(Self::TooFarBehind),
            0x8 => Some(Self::UpdateFailed),
            0x12 => Some(Self::MalformedTrack),
            _ => None,
        }
    }
}
