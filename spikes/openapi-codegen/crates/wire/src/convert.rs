//! Conversion between the generated server DTOs and the generated client DTOs.
//!
//! This is the "two generated crates, two DTO sets" boundary. The two crates are
//! independent, so neither the server DTOs nor the client DTOs are local to this
//! crate: the orphan rule forbids `impl From<ServerX> for ClientX`. Conversions
//! therefore have to be free functions (or a local extension trait).

use metis_spike_client::models as client;
use metis_spike_server::models as server;

pub fn echo_request_to_client(value: &server::EchoRequest) -> client::EchoRequest {
    client::EchoRequest::new(value.message.clone())
}

pub fn echo_request_to_server(value: &client::EchoRequest) -> server::EchoRequest {
    server::EchoRequest::new(value.message.clone())
}

pub fn echo_response_to_client(value: &server::EchoResponse) -> client::EchoResponse {
    client::EchoResponse::new(value.message.clone(), value.received_at.to_rfc3339())
}

pub fn echo_response_to_server(value: &client::EchoResponse) -> server::EchoResponse {
    server::EchoResponse::new(
        value.message.clone(),
        chrono::DateTime::parse_from_rfc3339(&value.received_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_default(),
    )
}

pub fn event_to_client(value: &server::Event) -> client::Event {
    client::Event::new(
        value.id.clone(),
        kind_to_client(&value.kind),
        value.payload.clone(),
    )
}

pub fn event_to_server(value: &client::Event) -> server::Event {
    server::Event::new(
        value.id.clone(),
        kind_to_server(value.kind),
        value.payload.clone(),
    )
}

pub fn api_error_to_client(value: &server::ApiError) -> client::ApiError {
    client::ApiError::new(value.code.clone(), value.message.clone())
}

pub fn api_error_to_server(value: &client::ApiError) -> server::ApiError {
    server::ApiError::new(value.code.clone(), value.message.clone())
}

/// The server generator renders inline enums as `String`; the client generator
/// renders them as a real enum. That asymmetry has to be bridged by hand.
pub fn kind_to_client(kind: &str) -> client::event::Kind {
    match kind {
        "heartbeat" => client::event::Kind::Heartbeat,
        _ => client::event::Kind::Message,
    }
}

pub fn kind_to_server(kind: client::event::Kind) -> String {
    match kind {
        client::event::Kind::Heartbeat => "heartbeat".to_owned(),
        client::event::Kind::Message => "message".to_owned(),
    }
}
