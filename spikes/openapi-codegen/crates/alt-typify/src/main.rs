//! Alternative C: generate models only with `typify` and handwrite both HTTP
//! layers.
//!
//! The types in `models.rs` come from `cargo run -p typify-gen`. Everything below
//! is handwritten glue: the server handlers, the client request, and the router.

#[allow(dead_code)]
mod models;

use std::{convert::Infallible, time::Duration};

use axum::{
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use futures_util::{stream, Stream};
use models::{EchoRequest, EchoResponse, EventKind};
use spike_shared::{serve_uds, sse_client, transport};

const LAST_EVENT_ID: u64 = 3;

async fn echo(Json(request): Json<EchoRequest>) -> Json<EchoResponse> {
    Json(EchoResponse {
        message: request.message,
        received_at: chrono::Utc::now(),
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
                "kind": EventKind::Message,
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

async fn post_echo(
    client: &reqwest::Client,
    base: &str,
    request: &EchoRequest,
) -> reqwest::Result<EchoResponse> {
    client
        .post(format!("{base}/v1/echo"))
        .json(request)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
}

#[tokio::main]
async fn main() {
    let socket = std::env::temp_dir().join("metis-openapi-codegen-alt-typify.sock");
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

    let client = transport::unix_client(&socket);
    let echoed = post_echo(
        &client,
        "http://localhost",
        &EchoRequest {
            message: "hello over uds".to_owned(),
        },
    )
    .await
    .expect("echo call");
    println!("[echo] message={:?}", echoed.message);

    let frames = sse_client::read_frames(&client, "http://localhost/v1/events", None, 3)
        .await
        .expect("sse stream");
    let events: Vec<models::Event> = frames.iter().filter_map(|frame| frame.decode()).collect();
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
