//! The two contract seams `api-contracts` exposes to later tickets.
//!
//! Both are compile-and-construct checks against the committed generated trees:
//! the message types round-trip through the buffa message API, and the
//! generated service trait is implementable and registers its route on a
//! `Router`. Serving the call belongs to the transport ticket, not here.

use std::sync::Arc;

use api_contracts::connect::metis::v1::{
    DAEMON_SERVICE_CANCEL_SPEC, DAEMON_SERVICE_SERVICE_NAME, DAEMON_SERVICE_STREAM_EVENTS_SPEC,
    DaemonService, DaemonServiceExt,
};
use api_contracts::proto::metis::v1::{
    CancelOutcome, CancelRequest, CancelResponse, EchoRequest, EchoResponse, Event, EventKind,
    PairRequest, PairResponse, StreamEventsRequest,
};
use buffa::Message;
use connectrpc::{
    RequestContext, Response, Router, ServiceRequest, ServiceResult, ServiceStream, StreamType,
};

/// The name the contract declares: package `metis.v1`, service `DaemonService`.
const DAEMON_SERVICE_NAME: &str = "metis.v1.DaemonService";

/// The Echo procedure path the the contract declares.
const ECHO_PATH: &str = "metis.v1.DaemonService/Echo";

/// The Pair procedure path the contract declares.
const PAIR_PATH: &str = "metis.v1.DaemonService/Pair";

/// The StreamEvents procedure path the contract declares.
const STREAM_EVENTS_PATH: &str = "metis.v1.DaemonService/StreamEvents";

/// The Cancel procedure path the contract declares.
const CANCEL_PATH: &str = "metis.v1.DaemonService/Cancel";

#[test]
fn echo_request_matches_the_declared_wire_format() {
    let request = EchoRequest {
        message: "hello".to_owned(),
        ..Default::default()
    };

    // Protobuf field 1, wire type 2 (length-delimited): 0x0a, then the length
    // and the bytes. An independent literal from the encoding spec, so a
    // renumbered field cannot pass unnoticed.
    assert_eq!(request.encode_to_vec(), b"\x0a\x05hello");
}

#[test]
fn echo_messages_round_trip_through_the_generated_message_api() {
    let request = EchoRequest {
        message: "hello".to_owned(),
        ..Default::default()
    };
    let decoded = EchoRequest::decode(&mut request.encode_to_vec().as_slice())
        .expect("EchoRequest decodes what it encoded");
    assert_eq!(decoded, request);

    let response = EchoResponse {
        message: "hello".to_owned(),
        received_at: "2026-01-02T03:04:05Z".to_owned(),
        ..Default::default()
    };
    // Field 1 then field 2, both length-delimited, in field-number order: 20
    // bytes of RFC 3339 timestamp are declared field 2, not field 1.
    let mut expected = b"\x0a\x05hello".to_vec();
    expected.extend_from_slice(b"\x12\x14");
    expected.extend_from_slice(b"2026-01-02T03:04:05Z");

    let encoded = response.encode_to_vec();
    assert_eq!(encoded, expected);
    let decoded = EchoResponse::decode(&mut encoded.as_slice())
        .expect("EchoResponse decodes what it encoded");
    assert_eq!(decoded, response);
}

/// The pairing messages keep their declared field numbering, so a renumbered
/// field cannot pass unnoticed. Both carry one length-delimited field 1.
#[test]
fn pair_messages_round_trip_through_the_generated_message_api() {
    let request = PairRequest {
        preshared_key: "key".to_owned(),
        ..Default::default()
    };
    assert_eq!(request.encode_to_vec(), b"\x0a\x03key");

    let decoded = PairRequest::decode(&mut request.encode_to_vec().as_slice())
        .expect("PairRequest decodes what it encoded");
    assert_eq!(decoded, request);

    let response = PairResponse {
        token: "abcd".to_owned(),
        ..Default::default()
    };
    assert_eq!(response.encode_to_vec(), b"\x0a\x04abcd");
    let decoded = PairResponse::decode(&mut response.encode_to_vec().as_slice())
        .expect("PairResponse decodes what it encoded");
    assert_eq!(decoded, response);
}

/// The streaming request keeps its declared field numbering: the selected
/// channels are repeated field 1 and the cursor is field 2, and a `0` cursor
/// is the protobuf default and therefore absent from the wire.
#[test]
fn stream_events_request_carries_the_channels_and_the_cursor() {
    let request = StreamEventsRequest {
        channels: vec!["daemon".to_owned()],
        last_event_id: 0,
        ..Default::default()
    };
    // Field 1, wire type 2 (length-delimited): 0x0a, then the length and the
    // bytes. The absent cursor encodes nothing, because 0 is the default.
    assert_eq!(request.encode_to_vec(), b"\x0a\x06daemon");

    let with_cursor = StreamEventsRequest {
        channels: vec!["daemon".to_owned(), "session:7".to_owned()],
        last_event_id: 3,
        ..Default::default()
    };
    let mut expected = b"\x0a\x06daemon".to_vec();
    expected.extend_from_slice(b"\x0a\x09session:7");
    // Field 2, wire type 0 (varint): 0x10, then the id.
    expected.extend_from_slice(b"\x10\x03");
    assert_eq!(with_cursor.encode_to_vec(), expected);

    let decoded = StreamEventsRequest::decode(&mut with_cursor.encode_to_vec().as_slice())
        .expect("StreamEventsRequest decodes what it encoded");
    assert_eq!(decoded, with_cursor);
}

/// The event message keeps its declared field numbering: the identifier, the
/// kind, the channel, and the structured payload, in that order. The kinds are
/// the contract's first enum, and their wire values are part of the contract.
#[test]
fn event_messages_round_trip_through_the_generated_message_api() {
    let event = Event {
        id: 1,
        kind: EventKind::Event.into(),
        channel: "daemon".to_owned(),
        payload: "{}".to_owned(),
        ..Default::default()
    };
    // Field 1 varint 1, field 2 varint 3 (`EVENT`), field 3 length-delimited
    // `daemon`, field 4 length-delimited `{}`.
    let mut expected = b"\x08\x01".to_vec();
    expected.extend_from_slice(b"\x10\x03");
    expected.extend_from_slice(b"\x1a\x06daemon");
    expected.extend_from_slice(b"\x22\x02{}");
    assert_eq!(event.encode_to_vec(), expected);

    let decoded = Event::decode(&mut event.encode_to_vec().as_slice())
        .expect("Event decodes what it encoded");
    assert_eq!(decoded, event);

    assert_eq!(EventKind::Event as i32, 3);
    assert_eq!(EventKind::Snapshot as i32, 1);
    assert_eq!(EventKind::Heartbeat as i32, 2);
    assert_eq!(EventKind::Unspecified as i32, 0);
}

/// The streaming RPC is declared as server streaming: one request in, a stream
/// of events out, on the service's own procedure path.
#[test]
fn stream_events_is_declared_as_a_server_stream() {
    assert_eq!(
        DAEMON_SERVICE_STREAM_EVENTS_SPEC.procedure,
        "/metis.v1.DaemonService/StreamEvents"
    );
    assert_eq!(
        DAEMON_SERVICE_STREAM_EVENTS_SPEC.stream_type,
        StreamType::ServerStream
    );
}

/// The cancel call is an ordinary unary RPC on the service's own procedure
/// path, and its two messages carry the identifier in and the outcome out.
#[test]
fn cancel_is_declared_as_a_unary_call_with_an_outcome() {
    assert_eq!(
        DAEMON_SERVICE_CANCEL_SPEC.procedure,
        "/metis.v1.DaemonService/Cancel"
    );
    assert_eq!(DAEMON_SERVICE_CANCEL_SPEC.stream_type, StreamType::Unary);

    let request = CancelRequest {
        request_id: "cli-7".to_owned(),
        ..Default::default()
    };
    // Field 1, wire type 2 (length-delimited): 0x0a, then the length and bytes.
    assert_eq!(request.encode_to_vec(), b"\x0a\x05cli-7");

    let response = CancelResponse {
        outcome: CancelOutcome::Cancelled.into(),
        ..Default::default()
    };
    // Field 1, wire type 0 (varint): 0x08, then the outcome's number.
    assert_eq!(response.encode_to_vec(), b"\x08\x01");
    assert_eq!(CancelOutcome::Cancelled as i32, 1, "a cancelled request");
    assert_eq!(
        CancelOutcome::AlreadyFinished as i32,
        2,
        "the idempotent answer"
    );
    assert_eq!(CancelOutcome::Unspecified as i32, 0, "not set");

    let decoded = CancelRequest::decode(&mut request.encode_to_vec().as_slice())
        .expect("CancelRequest decodes what it encoded");
    assert_eq!(decoded, request);
}

/// A stand-in service implementation. Nothing serves it here; this proves the
/// generated trait is implementable from outside the crate.
struct DummyDaemon;

// The generated trait uses an explicit RPITIT signature; an `async fn` impl
// refines it, which the compiler flags with `refining_impl_trait`.
#[allow(refining_impl_trait)]
impl DaemonService for DummyDaemon {
    async fn echo(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, EchoRequest>,
    ) -> ServiceResult<EchoResponse> {
        Response::ok(EchoResponse {
            message: request.message.to_owned(),
            received_at: String::new(),
            ..Default::default()
        })
    }

    async fn pair(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, PairRequest>,
    ) -> ServiceResult<PairResponse> {
        Response::ok(PairResponse {
            token: String::new(),
            ..Default::default()
        })
    }

    /// One event and a clean end: enough to prove the streaming method is
    /// implementable from outside the crate.
    async fn stream_events(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, StreamEventsRequest>,
    ) -> ServiceResult<ServiceStream<Event>> {
        Response::stream_ok(futures_util::stream::iter([Ok(Event {
            id: 1,
            kind: EventKind::Event.into(),
            channel: "daemon".to_owned(),
            payload: "{}".to_owned(),
            ..Default::default()
        })]))
    }

    /// Nothing in flight: enough to prove the cancel method is implementable
    /// from outside the crate.
    async fn cancel(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, CancelRequest>,
    ) -> ServiceResult<CancelResponse> {
        Response::ok(CancelResponse {
            outcome: CancelOutcome::AlreadyFinished.into(),
            ..Default::default()
        })
    }
}

#[test]
fn generated_service_trait_registers_its_route_on_a_router() {
    assert_eq!(DAEMON_SERVICE_SERVICE_NAME, DAEMON_SERVICE_NAME);

    // The router starts empty, so the routes below come from registration.
    assert!(!Router::new().has_method(ECHO_PATH));
    assert!(!Router::new().has_method(PAIR_PATH));
    assert!(!Router::new().has_method(STREAM_EVENTS_PATH));
    assert!(!Router::new().has_method(CANCEL_PATH));

    let router = Arc::new(DummyDaemon).register(Router::new());

    assert!(router.has_method(ECHO_PATH));
    assert!(router.has_method(PAIR_PATH));
    assert!(router.has_method(STREAM_EVENTS_PATH));
    assert!(router.has_method(CANCEL_PATH));
    let mut methods = router.methods().collect::<Vec<_>>();
    methods.sort_unstable();
    assert_eq!(
        methods,
        vec![CANCEL_PATH, ECHO_PATH, PAIR_PATH, STREAM_EVENTS_PATH]
    );
}
