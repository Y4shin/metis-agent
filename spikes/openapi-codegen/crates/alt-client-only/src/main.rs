//! Alternative B: generate the client only and handwrite the axum server.
//!
//! The server below is the whole handwritten glue for this alternative: request
//! and response types, the JSON handler, the SSE handler and the router. The
//! generated client is used unchanged against it.

use std::{convert::Infallible, time::Duration};

use axum::{
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use futures_util::{stream, Stream};
use metis_spike_client::apis::{configuration::Configuration, default_api};
use metis_spike_client::models as client_models;
use serde::{Deserialize, Serialize};
use spike_shared::{serve_uds, sse_client, transport};

const LAST_EVENT_ID: u64 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EchoRequest {
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EchoResponse {
    message: String,
    #[serde(rename = "receivedAt")]
    received_at: String,
}

async fn echo(Json(request): Json<EchoRequest>) -> Json<EchoResponse> {
    Json(EchoResponse {
        message: request.message,
        received_at: chrono::Utc::now().to_rfc3339(),
    })
}

async fn events(headers: HeaderMap) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let resume_from = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let frames = ((resume_from + 1)..=LAST_EVENT_ID)
        .flat_map(|id| {
            let data = serde_json::json!({
                "id": id.to_string(),
                "kind": "message",
                "payload": format!("payload-{id}"),
            })
            .to_string();
            [
                Ok(Event::default().comment("heartbeat")),
                Ok(Event::default()
                    .id(id.to_string())
                    .event("message")
                    .data(data)),
            ]
        })
        .collect::<Vec<_>>();
    Sse::new(stream::iter(frames)).keep_alive(KeepAlive::default())
}

#[tokio::main]
async fn main() {
    let socket = std::env::temp_dir().join("metis-openapi-codegen-alt-client-only.sock");
    let app = Router::new()
        .route("/v1/echo", post(echo))
        .route("/v1/events", get(events));
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
    let echoed = default_api::echo(
        &configuration,
        client_models::EchoRequest::new("hello over uds".to_owned()),
    )
    .await
    .expect("echo call");
    println!("[echo] message={:?}", echoed.message);

    let frames =
        sse_client::read_frames(&configuration.client, "http://localhost/v1/events", None, 3)
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

    let ok = echoed.message == "hello over uds" && events.len() == 3 && heartbeats == 3;
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
