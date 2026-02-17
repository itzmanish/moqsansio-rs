use moq_protocol::codec::{write_u16, write_varint, Cursor, Decode, Encode};
use moq_protocol::error::{Error, PublishDoneStatus, RequestErrorCode, SessionError};
use moq_protocol::message::fetch::*;
use moq_protocol::message::publish::*;
use moq_protocol::message::subscribe::*;
use moq_protocol::message::*;
use moq_protocol::params::*;
use moq_protocol::types::*;
use moq_protocol::varint::{decode_varint, MAX_VARINT};

// ---------------------------------------------------------------------------
// A. Error code wire values (spec compliance, IANA Tables 10-12)
// ---------------------------------------------------------------------------

#[test]
fn request_error_code_wire_values() {
    assert_eq!(RequestErrorCode::InternalError.as_u64(), 0x0);
    assert_eq!(RequestErrorCode::Unauthorized.as_u64(), 0x1);
    assert_eq!(RequestErrorCode::Timeout.as_u64(), 0x2);
    assert_eq!(RequestErrorCode::NotSupported.as_u64(), 0x3);
    assert_eq!(RequestErrorCode::MalformedAuthToken.as_u64(), 0x4);
    assert_eq!(RequestErrorCode::ExpiredAuthToken.as_u64(), 0x5);
    assert_eq!(RequestErrorCode::DoesNotExist.as_u64(), 0x10);
    assert_eq!(RequestErrorCode::InvalidRange.as_u64(), 0x11);
    assert_eq!(RequestErrorCode::MalformedTrack.as_u64(), 0x12);
    assert_eq!(RequestErrorCode::DuplicateSubscription.as_u64(), 0x19);
    assert_eq!(RequestErrorCode::Uninterested.as_u64(), 0x20);
    assert_eq!(RequestErrorCode::PrefixOverlap.as_u64(), 0x30);
    assert_eq!(RequestErrorCode::InvalidJoiningRequestId.as_u64(), 0x32);
}

#[test]
fn request_error_code_roundtrip_all() {
    let codes = [
        (0x0, RequestErrorCode::InternalError),
        (0x1, RequestErrorCode::Unauthorized),
        (0x2, RequestErrorCode::Timeout),
        (0x3, RequestErrorCode::NotSupported),
        (0x4, RequestErrorCode::MalformedAuthToken),
        (0x5, RequestErrorCode::ExpiredAuthToken),
        (0x10, RequestErrorCode::DoesNotExist),
        (0x11, RequestErrorCode::InvalidRange),
        (0x12, RequestErrorCode::MalformedTrack),
        (0x19, RequestErrorCode::DuplicateSubscription),
        (0x20, RequestErrorCode::Uninterested),
        (0x30, RequestErrorCode::PrefixOverlap),
        (0x32, RequestErrorCode::InvalidJoiningRequestId),
    ];
    for (wire, expected) in codes {
        assert_eq!(RequestErrorCode::from_u64(wire), Some(expected));
        assert_eq!(expected.as_u64(), wire);
    }
}

#[test]
fn request_error_code_unknown_returns_none() {
    assert_eq!(RequestErrorCode::from_u64(0x06), None);
    assert_eq!(RequestErrorCode::from_u64(0x13), None);
    assert_eq!(RequestErrorCode::from_u64(0x15), None);
    assert_eq!(RequestErrorCode::from_u64(0xFF), None);
}

#[test]
fn publish_done_status_wire_values() {
    let codes = [
        (0x0, PublishDoneStatus::InternalError),
        (0x1, PublishDoneStatus::Unauthorized),
        (0x2, PublishDoneStatus::TrackEnded),
        (0x3, PublishDoneStatus::SubscriptionEnded),
        (0x4, PublishDoneStatus::GoingAway),
        (0x5, PublishDoneStatus::Expired),
        (0x6, PublishDoneStatus::TooFarBehind),
        (0x8, PublishDoneStatus::UpdateFailed),
        (0x12, PublishDoneStatus::MalformedTrack),
    ];
    for (wire, expected) in codes {
        assert_eq!(PublishDoneStatus::from_u64(wire), Some(expected));
        assert_eq!(expected.as_u64(), wire);
    }
}

#[test]
fn publish_done_status_unknown_returns_none() {
    assert_eq!(PublishDoneStatus::from_u64(0x7), None);
    assert_eq!(PublishDoneStatus::from_u64(0x9), None);
    assert_eq!(PublishDoneStatus::from_u64(0xFF), None);
}

#[test]
fn session_error_wire_values() {
    assert_eq!(SessionError::NoError.as_u64(), 0x0);
    assert_eq!(SessionError::InternalError.as_u64(), 0x1);
    assert_eq!(SessionError::Unauthorized.as_u64(), 0x2);
    assert_eq!(SessionError::ProtocolViolation.as_u64(), 0x3);
    assert_eq!(SessionError::InvalidRequestId.as_u64(), 0x4);
    assert_eq!(SessionError::DuplicateTrackAlias.as_u64(), 0x5);
    assert_eq!(SessionError::KeyValueFormattingError.as_u64(), 0x6);
    assert_eq!(SessionError::TooManyRequests.as_u64(), 0x7);
    assert_eq!(SessionError::InvalidPath.as_u64(), 0x8);
    assert_eq!(SessionError::MalformedPath.as_u64(), 0x9);
    assert_eq!(SessionError::GoawayTimeout.as_u64(), 0x10);
    assert_eq!(SessionError::ControlMessageTimeout.as_u64(), 0x11);
    assert_eq!(SessionError::DataStreamTimeout.as_u64(), 0x12);
    assert_eq!(SessionError::AuthTokenCacheOverflow.as_u64(), 0x13);
    assert_eq!(SessionError::DuplicateAuthTokenAlias.as_u64(), 0x14);
    assert_eq!(SessionError::VersionNegotiationFailed.as_u64(), 0x15);
    assert_eq!(SessionError::MalformedAuthToken.as_u64(), 0x16);
    assert_eq!(SessionError::UnknownAuthTokenAlias.as_u64(), 0x17);
    assert_eq!(SessionError::ExpiredAuthToken.as_u64(), 0x18);
    assert_eq!(SessionError::InvalidAuthority.as_u64(), 0x19);
    assert_eq!(SessionError::MalformedAuthority.as_u64(), 0x1A);
}

// ---------------------------------------------------------------------------
// B. Boundary values
// ---------------------------------------------------------------------------

#[test]
fn location_max_varint_roundtrip() {
    let loc = Location {
        group: MAX_VARINT,
        object: MAX_VARINT,
    };
    let buf = loc.encode_to_vec().unwrap();
    let mut c = Cursor::new(&buf);
    let decoded = Location::decode(&mut c).unwrap();
    assert_eq!(loc, decoded);
    assert_eq!(c.remaining(), 0);
}

#[test]
fn reason_phrase_at_max_1024_bytes() {
    let text = "x".repeat(1024);
    let rp = ReasonPhrase(text.clone());
    let buf = rp.encode_to_vec().unwrap();
    let mut c = Cursor::new(&buf);
    let decoded = ReasonPhrase::decode(&mut c).unwrap();
    assert_eq!(decoded.0, text);
}

#[test]
fn reason_phrase_over_1024_encode_rejected() {
    let text = "x".repeat(1025);
    let rp = ReasonPhrase(text);
    assert!(matches!(
        rp.encode_to_vec(),
        Err(Error::ReasonPhraseTooLong(1025))
    ));
}

#[test]
fn reason_phrase_over_1024_decode_rejected() {
    let mut buf = Vec::new();
    write_varint(&mut buf, 1025).unwrap();
    buf.extend_from_slice(&vec![b'x'; 1025]);
    let mut c = Cursor::new(&buf);
    let result = ReasonPhrase::decode(&mut c);
    assert!(matches!(result, Err(Error::ProtocolViolation(_))));
}

#[test]
fn track_namespace_32_fields_succeeds() {
    let fields: Vec<Vec<u8>> = (0..32).map(|i| format!("f{i}").into_bytes()).collect();
    let ns = TrackNamespace::new(fields).unwrap();
    let buf = ns.encode_to_vec().unwrap();
    let mut c = Cursor::new(&buf);
    let decoded = TrackNamespace::decode(&mut c).unwrap();
    assert_eq!(ns, decoded);
}

#[test]
fn track_namespace_33_fields_rejected() {
    let fields: Vec<Vec<u8>> = (0..33).map(|i| format!("f{i}").into_bytes()).collect();
    assert!(TrackNamespace::new(fields).is_err());
}

#[test]
fn track_namespace_empty_rejected() {
    let fields: Vec<Vec<u8>> = vec![];
    assert!(TrackNamespace::new(fields).is_err());
}

#[test]
fn publish_done_max_stream_count() {
    let msg = ControlMessage::PublishDone(PublishDone {
        request_id: 0,
        status_code: 0x2,
        stream_count: MAX_VARINT,
        error_reason: ReasonPhrase(String::new()),
    });
    let mut buf = Vec::new();
    encode_control_message(&msg, &mut buf).unwrap();
    let (decoded, consumed) = decode_control_message(&buf).unwrap();
    assert_eq!(consumed, buf.len());
    assert_eq!(msg, decoded);
}

#[test]
fn fetch_ok_end_of_track_true_roundtrip() {
    let msg = ControlMessage::FetchOk(FetchOk {
        request_id: 0,
        end_of_track: true,
        end_location: Location {
            group: 100,
            object: 50,
        },
        parameters: Parameters::new(),
        track_extensions: vec![],
    });
    let mut buf = Vec::new();
    encode_control_message(&msg, &mut buf).unwrap();
    let (decoded, _) = decode_control_message(&buf).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn empty_parameters_roundtrip() {
    let msg = ControlMessage::Subscribe(Subscribe {
        request_id: 0,
        track_namespace: TrackNamespace::from_strings(&["ns"]).unwrap(),
        track_name: b"t".to_vec(),
        parameters: Parameters::new(),
    });
    let mut buf = Vec::new();
    encode_control_message(&msg, &mut buf).unwrap();
    let (decoded, consumed) = decode_control_message(&buf).unwrap();
    assert_eq!(consumed, buf.len());
    assert_eq!(msg, decoded);
}

// ---------------------------------------------------------------------------
// C. Malformed input rejection
// ---------------------------------------------------------------------------

#[test]
fn decode_empty_buffer() {
    assert!(matches!(
        decode_control_message(&[]),
        Err(Error::BufferTooShort)
    ));
}

#[test]
fn decode_unknown_message_type() {
    let mut buf = Vec::new();
    write_varint(&mut buf, 0xFF).unwrap();
    write_u16(&mut buf, 0);
    let result = decode_control_message(&buf);
    assert!(matches!(result, Err(Error::UnknownMessageType(0xFF))));
}

#[test]
fn invalid_filter_type_rejected() {
    for bad in [0x0, 0x5, 0xFF] {
        let mut buf = Vec::new();
        write_varint(&mut buf, bad).unwrap();
        let mut c = Cursor::new(&buf);
        let result = SubscriptionFilter::decode(&mut c);
        assert!(
            matches!(result, Err(Error::InvalidFilterType(_))),
            "filter type {bad:#x}"
        );
    }
}

#[test]
fn invalid_fetch_type_rejected() {
    let mut payload = Vec::new();
    write_varint(&mut payload, 0).unwrap(); // request_id
    write_varint(&mut payload, 0x99).unwrap(); // bad fetch type

    let mut buf = Vec::new();
    write_varint(&mut buf, MSG_FETCH).unwrap();
    write_u16(&mut buf, payload.len() as u16);
    buf.extend_from_slice(&payload);

    let result = decode_control_message(&buf);
    assert!(matches!(result, Err(Error::ProtocolViolation(_))));
}

#[test]
fn track_namespace_zero_fields_decode_rejected() {
    let mut buf = Vec::new();
    write_varint(&mut buf, 0).unwrap();
    let mut c = Cursor::new(&buf);
    let result = TrackNamespace::decode(&mut c);
    assert!(matches!(result, Err(Error::ProtocolViolation(_))));
}

#[test]
fn track_namespace_33_fields_decode_rejected() {
    let mut buf = Vec::new();
    write_varint(&mut buf, 33).unwrap();
    let mut c = Cursor::new(&buf);
    let result = TrackNamespace::decode(&mut c);
    assert!(matches!(result, Err(Error::ProtocolViolation(_))));
}

#[test]
fn track_namespace_empty_field_value_decode_rejected() {
    let mut buf = Vec::new();
    write_varint(&mut buf, 2).unwrap();
    write_varint(&mut buf, 3).unwrap();
    buf.extend_from_slice(b"abc");
    write_varint(&mut buf, 0).unwrap(); // empty field
    let mut c = Cursor::new(&buf);
    let result = TrackNamespace::decode(&mut c);
    assert!(matches!(result, Err(Error::ProtocolViolation(_))));
}

#[test]
fn track_namespace_prefix_empty_field_value_decode_rejected() {
    let mut buf = Vec::new();
    write_varint(&mut buf, 1).unwrap();
    write_varint(&mut buf, 0).unwrap(); // empty field
    let mut c = Cursor::new(&buf);
    let result = TrackNamespacePrefix::decode(&mut c);
    assert!(matches!(result, Err(Error::ProtocolViolation(_))));
}

#[test]
fn subscribe_option_invalid_rejected() {
    assert!(SubscribeOption::from_u64(0x3).is_err());
    assert!(SubscribeOption::from_u64(0xFF).is_err());
}

#[test]
fn group_order_invalid_rejected() {
    assert!(GroupOrder::from_u64(0x0).is_err());
    assert!(GroupOrder::from_u64(0x3).is_err());
}

#[test]
fn kvp_decreasing_types_rejected() {
    let pairs = vec![
        KeyValuePair {
            key: 4,
            value: KvValue::Varint(1),
        },
        KeyValuePair {
            key: 2,
            value: KvValue::Varint(2),
        },
    ];
    let mut buf = Vec::new();
    let result = encode_key_value_pairs(&mut buf, &pairs);
    assert!(matches!(result, Err(Error::KeyValueFormattingError(_))));
}

#[test]
fn kvp_varint_on_odd_type_rejected() {
    let pairs = vec![KeyValuePair {
        key: 3,
        value: KvValue::Varint(42),
    }];
    let mut buf = Vec::new();
    let result = encode_key_value_pairs(&mut buf, &pairs);
    assert!(matches!(result, Err(Error::KeyValueFormattingError(_))));
}

#[test]
fn kvp_bytes_on_even_type_rejected() {
    let pairs = vec![KeyValuePair {
        key: 2,
        value: KvValue::Bytes(b"hi".to_vec()),
    }];
    let mut buf = Vec::new();
    let result = encode_key_value_pairs(&mut buf, &pairs);
    assert!(matches!(result, Err(Error::KeyValueFormattingError(_))));
}

#[test]
fn fetch_ok_end_of_track_invalid_value_rejected() {
    let mut payload = Vec::new();
    write_varint(&mut payload, 0).unwrap(); // request_id
    payload.push(2u8); // invalid end_of_track (must be 0 or 1)
    write_varint(&mut payload, 1).unwrap(); // end_location.group
    write_varint(&mut payload, 0).unwrap(); // end_location.object
    write_varint(&mut payload, 0).unwrap(); // param count

    let mut buf = Vec::new();
    write_varint(&mut buf, MSG_FETCH_OK).unwrap();
    write_u16(&mut buf, payload.len() as u16);
    buf.extend_from_slice(&payload);

    let result = decode_control_message(&buf);
    assert!(matches!(result, Err(Error::ProtocolViolation(_))));
}

// ---------------------------------------------------------------------------
// D. Trailing bytes / payload consumption
// ---------------------------------------------------------------------------

#[test]
fn trailing_bytes_in_unsubscribe_rejected() {
    let mut payload = Vec::new();
    write_varint(&mut payload, 0).unwrap(); // request_id
    payload.push(0xFF); // trailing garbage

    let mut buf = Vec::new();
    write_varint(&mut buf, MSG_UNSUBSCRIBE).unwrap();
    write_u16(&mut buf, payload.len() as u16);
    buf.extend_from_slice(&payload);

    let result = decode_control_message(&buf);
    assert!(matches!(result, Err(Error::ProtocolViolation(_))));
}

#[test]
fn trailing_bytes_in_max_request_id_rejected() {
    let mut payload = Vec::new();
    write_varint(&mut payload, 100).unwrap();
    payload.extend_from_slice(&[0x00, 0x00]); // trailing garbage

    let mut buf = Vec::new();
    write_varint(&mut buf, MSG_MAX_REQUEST_ID).unwrap();
    write_u16(&mut buf, payload.len() as u16);
    buf.extend_from_slice(&payload);

    let result = decode_control_message(&buf);
    assert!(matches!(result, Err(Error::ProtocolViolation(_))));
}

#[test]
fn trailing_bytes_in_goaway_rejected() {
    let mut payload = Vec::new();
    write_varint(&mut payload, 0).unwrap(); // URI length = 0 (empty)
    payload.push(0xAB); // trailing garbage

    let mut buf = Vec::new();
    write_varint(&mut buf, MSG_GOAWAY).unwrap();
    write_u16(&mut buf, payload.len() as u16);
    buf.extend_from_slice(&payload);

    let result = decode_control_message(&buf);
    assert!(matches!(result, Err(Error::ProtocolViolation(_))));
}

#[test]
fn trailing_bytes_in_fetch_cancel_rejected() {
    let mut payload = Vec::new();
    write_varint(&mut payload, 10).unwrap(); // request_id
    payload.push(0x01); // trailing garbage

    let mut buf = Vec::new();
    write_varint(&mut buf, MSG_FETCH_CANCEL).unwrap();
    write_u16(&mut buf, payload.len() as u16);
    buf.extend_from_slice(&payload);

    let result = decode_control_message(&buf);
    assert!(matches!(result, Err(Error::ProtocolViolation(_))));
}

// ---------------------------------------------------------------------------
// E. Partial reads / buffer underflow
// ---------------------------------------------------------------------------

#[test]
fn control_message_type_only_no_length() {
    let mut buf = Vec::new();
    write_varint(&mut buf, MSG_SUBSCRIBE).unwrap();
    let result = decode_control_message(&buf);
    assert!(matches!(result, Err(Error::BufferTooShort)));
}

#[test]
fn control_message_length_exceeds_available() {
    let mut buf = Vec::new();
    write_varint(&mut buf, MSG_SUBSCRIBE).unwrap();
    write_u16(&mut buf, 100); // claim 100 bytes of payload
    buf.push(0x00); // only 1 byte available
    let result = decode_control_message(&buf);
    assert!(matches!(result, Err(Error::BufferTooShort)));
}

#[test]
fn varint_2byte_truncated() {
    assert!(matches!(decode_varint(&[0x40]), Err(Error::BufferTooShort)));
}

#[test]
fn varint_4byte_truncated() {
    assert!(matches!(
        decode_varint(&[0x80, 0x01]),
        Err(Error::BufferTooShort)
    ));
}

#[test]
fn varint_8byte_truncated() {
    assert!(matches!(
        decode_varint(&[0xC0, 0x00, 0x00, 0x00]),
        Err(Error::BufferTooShort)
    ));
}

#[test]
fn subscribe_truncated_after_request_id() {
    let mut payload = Vec::new();
    write_varint(&mut payload, 0).unwrap(); // request_id only, no namespace

    let mut buf = Vec::new();
    write_varint(&mut buf, MSG_SUBSCRIBE).unwrap();
    write_u16(&mut buf, payload.len() as u16);
    buf.extend_from_slice(&payload);

    let result = decode_control_message(&buf);
    assert!(matches!(result, Err(Error::BufferTooShort)));
}

// ---------------------------------------------------------------------------
// F. Message type constant pin tests
// ---------------------------------------------------------------------------

#[test]
fn message_type_constants_match_spec() {
    assert_eq!(MSG_CLIENT_SETUP, 0x20);
    assert_eq!(MSG_SERVER_SETUP, 0x21);
    assert_eq!(MSG_GOAWAY, 0x10);
    assert_eq!(MSG_MAX_REQUEST_ID, 0x15);
    assert_eq!(MSG_REQUESTS_BLOCKED, 0x1A);
    assert_eq!(MSG_REQUEST_OK, 0x07);
    assert_eq!(MSG_REQUEST_ERROR, 0x05);
    assert_eq!(MSG_SUBSCRIBE, 0x03);
    assert_eq!(MSG_SUBSCRIBE_OK, 0x04);
    assert_eq!(MSG_REQUEST_UPDATE, 0x02);
    assert_eq!(MSG_UNSUBSCRIBE, 0x0A);
    assert_eq!(MSG_PUBLISH, 0x1D);
    assert_eq!(MSG_PUBLISH_OK, 0x1E);
    assert_eq!(MSG_PUBLISH_DONE, 0x0B);
    assert_eq!(MSG_FETCH, 0x16);
    assert_eq!(MSG_FETCH_OK, 0x18);
    assert_eq!(MSG_FETCH_CANCEL, 0x17);
    assert_eq!(MSG_TRACK_STATUS, 0x0D);
    assert_eq!(MSG_PUBLISH_NAMESPACE, 0x06);
    assert_eq!(MSG_NAMESPACE, 0x08);
    assert_eq!(MSG_PUBLISH_NAMESPACE_DONE, 0x09);
    assert_eq!(MSG_NAMESPACE_DONE, 0x0E);
    assert_eq!(MSG_PUBLISH_NAMESPACE_CANCEL, 0x0C);
    assert_eq!(MSG_SUBSCRIBE_NAMESPACE, 0x11);
}

#[test]
fn parameter_type_constants_match_spec() {
    assert_eq!(PARAM_PATH, 0x01);
    assert_eq!(PARAM_DELIVERY_TIMEOUT, 0x02);
    assert_eq!(PARAM_AUTHORIZATION_TOKEN, 0x03);
    assert_eq!(PARAM_MAX_AUTH_TOKEN_CACHE_SIZE, 0x04);
    assert_eq!(PARAM_AUTHORITY, 0x05);
    assert_eq!(PARAM_MOQT_IMPLEMENTATION, 0x07);
    assert_eq!(PARAM_EXPIRES, 0x08);
    assert_eq!(PARAM_LARGEST_OBJECT, 0x09);
    assert_eq!(PARAM_MAX_REQUEST_ID, 0x02);
    assert_eq!(PARAM_FORWARD, 0x10);
    assert_eq!(PARAM_SUBSCRIBER_PRIORITY, 0x20);
    assert_eq!(PARAM_SUBSCRIPTION_FILTER, 0x21);
    assert_eq!(PARAM_GROUP_ORDER, 0x22);
    assert_eq!(PARAM_NEW_GROUP_REQUEST, 0x32);
}
