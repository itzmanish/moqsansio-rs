// 9.  Control Messages
struct ControlMessage {
    message_type: u8,
    message_length: u16,
    message_payload: Vec<u8>,
}

struct TokenParameter {
    alias_type: usize,
    token_alias: Option<usize>,
    token_type: usize,
    token_value: Option<usize>,
}

enum ControlMessages {
    AuthorizationTokenParameter(ControlMessage),
}

// 1.4.1.  Location Structure
struct Location {
    group: u8,
    object: u8,
}

// 1.4.2.  Key-Value Pair Structure
struct KeyValuePair {
    tipe: u8,
    length: Option<u16>, // only present when type is odd
    value: Vec<u8>, // Value: A single varint encoded value when Type is even, otherwise a sequence of Length bytes.
}

// 1.4.3.  Reason Phrase Structure
struct ReasonPhrase {
    length: usize, // The reason phrase length has a maximum length of 1024 bytes.
    value: String, // The reason phrase value is encoded as UTF-8 string
}

pub enum MessageTypes {
    /*
           +======+============================================+
           |   ID | Messages                                   |
           +======+============================================+
           | 0x01 | RESERVED (SETUP for version 00)            |
           +------+--------------------------------------------+
           | 0x40 | RESERVED (CLIENT_SETUP for versions <= 10) |
           +------+--------------------------------------------+
           | 0x41 | RESERVED (SERVER_SETUP for versions <= 10) |
           +------+--------------------------------------------+
           | 0x20 | CLIENT_SETUP (Section 9.3)                 |
           +------+--------------------------------------------+
           | 0x21 | SERVER_SETUP (Section 9.3)                 |
           +------+--------------------------------------------+
           | 0x10 | GOAWAY (Section 9.4)                       |
           +------+--------------------------------------------+
           | 0x15 | MAX_REQUEST_ID (Section 9.5)               |
           +------+--------------------------------------------+
           | 0x1A | REQUESTS_BLOCKED (Section 9.6)             |
           +------+--------------------------------------------+
           |  0x3 | SUBSCRIBE (Section 9.7)                    |
           +------+--------------------------------------------+
           |  0x4 | SUBSCRIBE_OK (Section 9.8)                 |
           +------+--------------------------------------------+
           |  0x5 | SUBSCRIBE_ERROR (Section 9.9)              |
           +------+--------------------------------------------+
           |  0x2 | SUBSCRIBE_UPDATE (Section 9.10)            |
           +------+--------------------------------------------+
           |  0xA | UNSUBSCRIBE (Section 9.11)                 |
           +------+--------------------------------------------+
           |  0xB | PUBLISH_DONE (Section 9.12)                |
           +------+--------------------------------------------+
           | 0x1D | PUBLISH (Section 9.13)                     |
           +------+--------------------------------------------+
           | 0x1E | PUBLISH_OK (Section 9.14)                  |
           +------+--------------------------------------------+
           | 0x1F | PUBLISH_ERROR (Section 9.15)               |
           +------+--------------------------------------------+
           | 0x16 | FETCH (Section 9.16)                       |
           +------+--------------------------------------------+
           | 0x18 | FETCH_OK (Section 9.17)                    |
           +------+--------------------------------------------+
           | 0x19 | FETCH_ERROR (Section 9.18)                 |
           +------+--------------------------------------------+
           | 0x17 | FETCH_CANCEL (Section 9.19)                |
           +------+--------------------------------------------+
           |  0xD | TRACK_STATUS (Section 9.20)                |
           +------+--------------------------------------------+
           |  0xE | TRACK_STATUS_OK (Section 9.21              |
           +------+--------------------------------------------+
           |  0xF | TRACK_STATUS_ERROR (Section 9.22)          |
           +------+--------------------------------------------+
           |  0x6 | PUBLISH_NAMESPACE (Section 9.23)           |
           +------+--------------------------------------------+
           |  0x7 | PUBLISH_NAMESPACE_OK (Section 9.24)        |
           +------+--------------------------------------------+
           |  0x8 | PUBLISH_NAMESPACE_ERROR (Section 9.25)     |
           +------+--------------------------------------------+
           |  0x9 | PUBLISH_NAMESPACE_DONE (Section 9.26)      |
           +------+--------------------------------------------+
           |  0xC | PUBLISH_NAMESPACE_CANCEL (Section 9.27)    |
           +------+--------------------------------------------+
           | 0x11 | SUBSCRIBE_NAMESPACE (Section 9.28)         |
           +------+--------------------------------------------+
           | 0x12 | SUBSCRIBE_NAMESPACE_OK (Section 9.29)      |
           +------+--------------------------------------------+
           | 0x13 | SUBSCRIBE_NAMESPACE_ERROR (Section 9.30    |
           +------+--------------------------------------------+
           | 0x14 | UNSUBSCRIBE_NAMESPACE (Section 9.31)       |
           +------+--------------------------------------------+
    */
    ClientSetup = 0x20,
}
