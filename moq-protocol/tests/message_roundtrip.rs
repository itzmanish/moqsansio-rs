use moq_protocol::message::fetch::*;
use moq_protocol::message::namespace::*;
use moq_protocol::message::publish::*;
use moq_protocol::message::request::*;
use moq_protocol::message::session_control::*;
use moq_protocol::message::setup::*;
use moq_protocol::message::subscribe::*;
use moq_protocol::message::*;
use moq_protocol::params::*;
use moq_protocol::types::*;

fn roundtrip(msg: ControlMessage) {
    let mut buf = Vec::new();
    encode_control_message(&msg, &mut buf).unwrap();
    let (decoded, consumed) = decode_control_message(&buf).unwrap();
    assert_eq!(consumed, buf.len(), "consumed all bytes");
    assert_eq!(msg, decoded);
}

fn make_namespace() -> TrackNamespace {
    TrackNamespace::from_strings(&["example.com", "meeting", "room1"]).unwrap()
}

fn make_params() -> Parameters {
    let mut p = Parameters::new();
    p.set_forward(true);
    p.set_subscriber_priority(128);
    p
}

#[test]
fn subscribe_roundtrip() {
    let mut params = make_params();
    params
        .set_subscription_filter(&SubscriptionFilter::LargestObject)
        .unwrap();
    roundtrip(ControlMessage::Subscribe(Subscribe {
        request_id: 0,
        track_namespace: make_namespace(),
        track_name: b"video".to_vec(),
        parameters: params,
    }));
}

#[test]
fn subscribe_ok_roundtrip() {
    let mut params = Parameters::new();
    params.set_expires(60000);
    params
        .set_largest_object(&Location {
            group: 10,
            object: 3,
        })
        .unwrap();
    roundtrip(ControlMessage::SubscribeOk(SubscribeOk {
        request_id: 0,
        track_alias: 42,
        parameters: params,
        track_extensions: vec![],
    }));
}

#[test]
fn publish_roundtrip() {
    roundtrip(ControlMessage::Publish(Publish {
        request_id: 2,
        track_namespace: make_namespace(),
        track_name: b"audio".to_vec(),
        track_alias: 99,
        parameters: make_params(),
        track_extensions: vec![],
    }));
}

#[test]
fn publish_ok_roundtrip() {
    roundtrip(ControlMessage::PublishOk(PublishOk {
        request_id: 2,
        parameters: make_params(),
    }));
}

#[test]
fn publish_done_roundtrip() {
    roundtrip(ControlMessage::PublishDone(PublishDone {
        request_id: 2,
        status_code: 0x2,
        stream_count: 5,
        error_reason: ReasonPhrase("track ended".into()),
    }));
}

#[test]
fn publish_namespace_roundtrip() {
    roundtrip(ControlMessage::PublishNamespace(PublishNamespace {
        request_id: 4,
        track_namespace: make_namespace(),
        parameters: Parameters::new(),
    }));
}

#[test]
fn publish_namespace_done_roundtrip() {
    roundtrip(ControlMessage::PublishNamespaceDone(PublishNamespaceDone {
        request_id: 4,
    }));
}

#[test]
fn publish_namespace_cancel_roundtrip() {
    roundtrip(ControlMessage::PublishNamespaceCancel(
        PublishNamespaceCancel {
            request_id: 4,
            error_code: 0x1,
            error_reason: ReasonPhrase("unauthorized".into()),
        },
    ));
}

#[test]
fn subscribe_namespace_roundtrip() {
    roundtrip(ControlMessage::SubscribeNamespace(SubscribeNamespace {
        request_id: 6,
        track_namespace_prefix: TrackNamespacePrefix {
            fields: vec![b"example.com".to_vec(), b"meeting".to_vec()],
        },
        subscribe_options: SubscribeOption::Both,
        parameters: Parameters::new(),
    }));
}

#[test]
fn namespace_roundtrip() {
    roundtrip(ControlMessage::Namespace(Namespace {
        track_namespace_suffix: TrackNamespacePrefix {
            fields: vec![b"participant100".to_vec()],
        },
    }));
}

#[test]
fn namespace_done_roundtrip() {
    roundtrip(ControlMessage::NamespaceDone(NamespaceDone {
        track_namespace_suffix: TrackNamespacePrefix {
            fields: vec![b"participant100".to_vec()],
        },
    }));
}

#[test]
fn request_ok_roundtrip() {
    roundtrip(ControlMessage::RequestOk(RequestOk {
        request_id: 0,
        parameters: Parameters::new(),
    }));
}

#[test]
fn request_error_roundtrip() {
    roundtrip(ControlMessage::RequestError(RequestErrorMsg {
        request_id: 0,
        error_code: 0x10,
        retry_interval: 5000,
        error_reason: ReasonPhrase("does not exist".into()),
    }));
}

#[test]
fn request_update_roundtrip() {
    let mut params = Parameters::new();
    params.set_forward(false);
    roundtrip(ControlMessage::RequestUpdate(RequestUpdate {
        request_id: 8,
        existing_request_id: 0,
        parameters: params,
    }));
}

#[test]
fn unsubscribe_roundtrip() {
    roundtrip(ControlMessage::Unsubscribe(Unsubscribe { request_id: 0 }));
}

#[test]
fn goaway_roundtrip() {
    roundtrip(ControlMessage::Goaway(Goaway {
        new_session_uri: b"moqt://newserver.example.com/path".to_vec(),
    }));
}

#[test]
fn goaway_empty_uri_roundtrip() {
    roundtrip(ControlMessage::Goaway(Goaway {
        new_session_uri: vec![],
    }));
}

#[test]
fn max_request_id_roundtrip() {
    roundtrip(ControlMessage::MaxRequestId(MaxRequestId {
        max_request_id: 100,
    }));
}

#[test]
fn requests_blocked_roundtrip() {
    roundtrip(ControlMessage::RequestsBlocked(RequestsBlocked {
        maximum_request_id: 50,
    }));
}

#[test]
fn client_setup_roundtrip() {
    let mut params = Parameters::new();
    params.add_varint(PARAM_MAX_REQUEST_ID, 100);
    roundtrip(ControlMessage::ClientSetup(ClientSetup {
        parameters: params,
    }));
}

#[test]
fn server_setup_roundtrip() {
    let params = Parameters::new();
    roundtrip(ControlMessage::ServerSetup(ServerSetup {
        parameters: params,
    }));
}

#[test]
fn fetch_standalone_roundtrip() {
    roundtrip(ControlMessage::Fetch(Fetch {
        request_id: 10,
        fetch_type: FetchType::Standalone {
            track_namespace: make_namespace(),
            track_name: b"video".to_vec(),
            start: Location {
                group: 5,
                object: 0,
            },
            end: Location {
                group: 10,
                object: 0,
            },
        },
        parameters: Parameters::new(),
    }));
}

#[test]
fn fetch_ok_roundtrip() {
    roundtrip(ControlMessage::FetchOk(FetchOk {
        request_id: 10,
        end_of_track: false,
        end_location: Location {
            group: 9,
            object: 15,
        },
        parameters: Parameters::new(),
        track_extensions: vec![],
    }));
}

#[test]
fn fetch_cancel_roundtrip() {
    roundtrip(ControlMessage::FetchCancel(FetchCancel { request_id: 10 }));
}

#[test]
fn track_status_roundtrip() {
    roundtrip(ControlMessage::TrackStatus(TrackStatus {
        request_id: 12,
        track_namespace: make_namespace(),
        track_name: b"video".to_vec(),
        parameters: Parameters::new(),
    }));
}

#[test]
fn subscribe_with_absolute_range_filter() {
    let mut params = Parameters::new();
    params
        .set_subscription_filter(&SubscriptionFilter::AbsoluteRange {
            start: Location {
                group: 1,
                object: 0,
            },
            end_group: 100,
        })
        .unwrap();
    params.set_delivery_timeout(5000);
    params.set_subscriber_priority(64);
    roundtrip(ControlMessage::Subscribe(Subscribe {
        request_id: 14,
        track_namespace: make_namespace(),
        track_name: b"video-hd".to_vec(),
        parameters: params,
    }));
}

#[test]
fn publish_with_extensions() {
    roundtrip(ControlMessage::Publish(Publish {
        request_id: 16,
        track_namespace: make_namespace(),
        track_name: b"chat".to_vec(),
        track_alias: 7,
        parameters: Parameters::new(),
        track_extensions: vec![KeyValuePair {
            key: 2,
            value: KvValue::Varint(30000),
        }],
    }));
}
