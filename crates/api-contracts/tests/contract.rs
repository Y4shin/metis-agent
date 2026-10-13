//! The two contract seams `api-contracts` exposes to later tickets.
//!
//! Both are compile-and-construct checks against the committed generated trees:
//! the message types round-trip through the buffa message API, and the
//! generated service trait is implementable and registers its route on a
//! `Router`. Serving the call belongs to the transport ticket, not here.

use std::sync::Arc;

use api_contracts::connect::metis::v1::{
    DAEMON_SERVICE_SERVICE_NAME, DaemonService, DaemonServiceExt,
};
use api_contracts::proto::metis::v1::{EchoRequest, EchoResponse, PairRequest, PairResponse};
use buffa::Message;
use connectrpc::{RequestContext, Response, Router, ServiceRequest, ServiceResult};

/// The name the contract declares: package `metis.v1`, service `DaemonService`.
const DAEMON_SERVICE_NAME: &str = "metis.v1.DaemonService";

/// The Echo procedure path the the contract declares.
const ECHO_PATH: &str = "metis.v1.DaemonService/Echo";

/// The Pair procedure path the contract declares.
const PAIR_PATH: &str = "metis.v1.DaemonService/Pair";

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
}

#[test]
fn generated_service_trait_registers_its_route_on_a_router() {
    assert_eq!(DAEMON_SERVICE_SERVICE_NAME, DAEMON_SERVICE_NAME);

    // The router starts empty, so the routes below come from registration.
    assert!(!Router::new().has_method(ECHO_PATH));
    assert!(!Router::new().has_method(PAIR_PATH));

    let router = Arc::new(DummyDaemon).register(Router::new());

    assert!(router.has_method(ECHO_PATH));
    assert!(router.has_method(PAIR_PATH));
    let mut methods = router.methods().collect::<Vec<_>>();
    methods.sort_unstable();
    assert_eq!(methods, vec![ECHO_PATH, PAIR_PATH]);
}
