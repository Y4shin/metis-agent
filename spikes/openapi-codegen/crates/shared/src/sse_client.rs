//! Handwritten client side of the `text/event-stream` endpoint.
//!
//! The generated Rust client's `stream_events` calls `resp.text().await` and
//! `serde_json::from_str` into a single `models::Event`, so it buffers the whole
//! stream and cannot consume SSE at all. Reconnection, `Last-Event-ID` resume,
//! framing and heartbeat handling are all handwritten below.

use futures_util::StreamExt;
use serde::de::DeserializeOwned;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct SseFrame {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: String,
    pub comments: Vec<String>,
}

impl SseFrame {
    fn is_empty(&self) -> bool {
        self.id.is_none()
            && self.event.is_none()
            && self.data.is_empty()
            && self.comments.is_empty()
    }

    pub fn is_message(&self) -> bool {
        !self.data.is_empty()
    }

    /// The generated client DTOs are still usable: the SSE `data:` payload is the
    /// same JSON the OpenAPI document describes.
    pub fn decode<T: DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_str(&self.data).ok()
    }
}

/// Read SSE frames from `url`, optionally resuming after `last_event_id`, and
/// stop once `stop_after_messages` frames carrying data have arrived.
pub async fn read_frames(
    client: &reqwest::Client,
    url: &str,
    last_event_id: Option<&str>,
    stop_after_messages: usize,
) -> Result<Vec<SseFrame>, reqwest::Error> {
    let mut request = client.get(url);
    if let Some(id) = last_event_id {
        request = request.header("Last-Event-ID", id);
    }
    let response = request.send().await?.error_for_status()?;
    let mut body = response.bytes_stream();
    let mut buffer = String::new();
    let mut frame = SseFrame::default();
    let mut frames = Vec::new();
    let mut messages = 0usize;

    while let Some(chunk) = body.next().await {
        buffer.push_str(&String::from_utf8_lossy(&chunk?));
        while let Some(newline) = buffer.find('\n') {
            let line = buffer[..newline].trim_end_matches('\r').to_owned();
            buffer.drain(..=newline);
            if line.is_empty() {
                if !frame.is_empty() {
                    let is_message = frame.is_message();
                    frames.push(std::mem::take(&mut frame));
                    if is_message {
                        messages += 1;
                        if messages >= stop_after_messages {
                            return Ok(frames);
                        }
                    }
                }
            } else if let Some(comment) = line.strip_prefix(':') {
                frame.comments.push(comment.trim_start().to_owned());
            } else if let Some((field, value)) = line.split_once(':') {
                let value = value.strip_prefix(' ').unwrap_or(value);
                match field {
                    "id" => frame.id = Some(value.to_owned()),
                    "event" => frame.event = Some(value.to_owned()),
                    "data" => {
                        if !frame.data.is_empty() {
                            frame.data.push('\n');
                        }
                        frame.data.push_str(value);
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(frames)
}
