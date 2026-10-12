//! Tiny end-to-end binary for the spike.
//!
//! It serves the generated `rust-axum` router over a Unix domain socket through
//! hyper, calls the generated Rust client over that same socket, and consumes the
//! SSE endpoint with the handwritten client.

use std::sync::Arc;
use std::time::Duration;

use axum::middleware;
use metis_spike_client::apis::{configuration::Configuration, default_api};
use metis_spike_client::models as client_models;
use metis_spike_server::apis;
use metis_spike_server::models as server_models;
use metis_spike_server::server;
use spike_wire::{convert, serve_uds, sse_client, sse_server, transport};

struct ApiImpl;

#[async_trait::async_trait]
impl apis::default::Default for ApiImpl {
    async fn echo(
        &self,
        _method: axum::http::Method,
        _host: axum::extract::Host,
        _cookies: axum_extra::extract::CookieJar,
        body: server_models::EchoRequest,
    ) -> Result<apis::default::EchoResponse, ()> {
        Ok(apis::default::EchoResponse::Status200_EchoedMessage(
            server_models::EchoResponse {
                message: body.message,
                received_at: chrono::Utc::now(),
            },
        ))
    }

    async fn stream_events(
        &self,
        _method: axum::http::Method,
        _host: axum::extract::Host,
        _cookies: axum_extra::extract::CookieJar,
        _header_params: server_models::StreamEventsHeaderParams,
    ) -> Result<apis::default::StreamEventsResponse, ()> {
        // Never reached: the SSE path is short-circuited by the layer below,
        // because the generated variant can only carry a buffered `String`.
        Ok(apis::default::StreamEventsResponse::Status200_AStreamOfEvents(String::new()))
    }
}

#[tokio::main]
async fn main() {
    let socket = std::env::temp_dir().join("metis-openapi-codegen-spike.sock");
    let app = server::new(Arc::new(ApiImpl)).layer(middleware::from_fn(sse_server::sse_override));

    let server_socket = socket.clone();
    tokio::spawn(async move {
        if let Err(err) = serve_uds(&server_socket, app).await {
            eprintln!("server error: {err}");
        }
    });
    wait_for_socket(&socket).await;

    let configuration = Configuration {
        base_path: "http://localhost".to_owned(),
        client: transport::unix_client(&socket),
        ..Configuration::default()
    };

    // Ordinary request through the generated client.
    let echoed = default_api::echo(
        &configuration,
        client_models::EchoRequest::new("hello over uds".to_owned()),
    )
    .await
    .expect("echo call");
    let echoed_as_server = convert::echo_response_to_server(&echoed);
    println!(
        "[echo] status=200 message={:?} received_at={}",
        echoed_as_server.message, echoed_as_server.received_at
    );

    // SSE through the handwritten client, using the generated client's transport.
    let url = "http://localhost/v1/events";
    let frames = sse_client::read_frames(&configuration.client, url, None, 3)
        .await
        .expect("sse stream");
    let events: Vec<client_models::Event> =
        frames.iter().filter_map(|frame| frame.decode()).collect();
    let heartbeats = frames
        .iter()
        .filter(|frame| !frame.is_message() && !frame.comments.is_empty())
        .count();
    println!(
        "[sse] frames={} messages={} heartbeats={}",
        frames.len(),
        events.len(),
        heartbeats
    );

    // Last-Event-ID resume: ask for everything after the second event we saw, so
    // the server must start again at id 3 instead of replaying the stream.
    let last_id = frames.iter().filter_map(|frame| frame.id.clone()).nth(1);
    let resumed = sse_client::read_frames(&configuration.client, url, last_id.as_deref(), 1)
        .await
        .expect("sse resume");
    let resumed_id = resumed.iter().find_map(|frame| frame.id.clone());
    println!("[sse] resume_after={last_id:?} first_frame_id={resumed_id:?}");

    let ok = echoed.message == "hello over uds"
        && events.len() == 3
        && heartbeats == 3
        && resumed_id.as_deref() == Some("3");
    println!("[result] {}", if ok { "PASS" } else { "FAIL" });
    if !ok {
        std::process::exit(1);
    }
}

async fn wait_for_socket(socket: &std::path::Path) {
    for _ in 0..100 {
        if socket.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
