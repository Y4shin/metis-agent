//! Handwritten server side of the `text/event-stream` endpoint.
//!
//! The `rust-axum` generator models a `text/event-stream` response as a plain
//! `String` response variant (`StreamEventsResponse::Status200_AStreamOfEvents`)
//! and buffers it into `Body::from(String)`. It cannot stream. Everything below
//! is therefore handwritten.

use std::convert::Infallible;

use axum::{
    extract::Request,
    http::Method,
    middleware::Next,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
};
use futures_util::{stream, Stream};

const FIRST_EVENT_ID: u64 = 1;
const LAST_EVENT_ID: u64 = 3;

/// Build one message event plus one heartbeat comment per cursor position.
fn event_frames(start_after: u64) -> Vec<Result<Event, Infallible>> {
    let mut frames = Vec::new();
    for id in (start_after + 1)..=LAST_EVENT_ID {
        let data = serde_json::json!({
            "id": id.to_string(),
            "kind": "message",
            "payload": format!("payload-{id}"),
        })
        .to_string();
        frames.push(Ok(Event::default().comment("heartbeat")));
        frames.push(Ok(Event::default()
            .id(id.to_string())
            .event("message")
            .data(data)));
    }
    frames
}

/// The real SSE handler, mounted by [`crate::sse_server::sse_override`].
pub async fn sse_stream(
    last_event_id: Option<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let resume_from = last_event_id
        .as_deref()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(FIRST_EVENT_ID - 1);
    Sse::new(stream::iter(event_frames(resume_from))).keep_alive(KeepAlive::default())
}

/// Intercept `GET /v1/events` before the generated (buffered) handler sees it.
///
/// The generated router registers this path with a handler that returns
/// `Body::from(String)`, so it cannot stream. axum panics when the same route is
/// registered twice, so the only way to keep the generated router *and* stream
/// is to short-circuit in a layer wrapped around it.
pub async fn sse_override(request: Request, next: Next) -> Response {
    if request.method() == Method::GET && request.uri().path() == "/v1/events" {
        let last_event_id = request
            .headers()
            .get("last-event-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        return sse_stream(last_event_id).await.into_response();
    }
    next.run(request).await
}
