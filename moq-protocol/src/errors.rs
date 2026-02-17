use thiserror::Error;

#[derive(Error, Debug)]
pub enum MoqError {
    #[error("PROTOCOL_VIOLATION")]
    ProtocolViolation,

    #[error("DUPLICATE_AUTH_TOKEN_ALIAS")]
    DuplicateAuthTokenAlias,

    #[error("UNKNOWN_AUTH_TOKEN_ALIAS")]
    UnknownAuthTokenAlias,

    #[error("MALFORMED_AUTH_TOKEN")]
    MalformedAuthToken,

    #[error("TOO_FAR_BEHIND")]
    TooFarBehind,

    #[error("KEY_VALUE_FORMATTING_ERROR")]
    KeyValueFormattingError,
}

// 3.4.  Termination
pub enum TerminationErrorCodes {
    NoError = 0x0,
    InternalError,
    Unauthorized,
    ProtocolViolation,
    InvalidRequestId,
    DuplicateTrackAlias,
    KeyValueFormattingError,
    TooManyRequests,
    InValidPath,
    MalformedPath,
    GoawayTimeout,
    ControlMessageTimeout,
    DataStreamTimeout,
    AuthTokenCacheOverflow,
    DuplicateAuthTokenAlias,
    VersionNegotiationFailed,
    MalformedAuthToken,
    UnknownAuthTokenAlias,
    ExpiredAuthToken,
    InvalidAuthority,
    MalformedAuthority,
}
