//! Tiny end-to-end binary for the ConnectRPC spike.
//!
//! Serves the generated `SpikeService` over a Unix domain socket through a
//! hand-written hyper accept loop and drives both RPCs with the generated
//! client over `Http2Connection::lazy_with_connector`, then prints `PASS`.
//!
//! It mirrors `spikes/openapi-codegen/crates/wire/src/bin/spike.rs`: one
//! ordinary call and one server-streaming call with a resumable cursor. The
//! stream here is a real Connect server-streaming RPC rather than SSE.

use std::sync::Arc;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use connectrpc::ConnectError;
#[cfg(unix)]
use connectrpc::client::ClientConfig;
use connectrpc::{RequestContext, Response, Router, ServiceRequest, ServiceResult, ServiceStream};
#[cfg(unix)]
use spike_wire::connect::metis::spike::v1::{SPIKE_SERVICE_SERVICE_NAME, SpikeServiceClient};
use spike_wire::connect::metis::spike::v1::{SpikeService, SpikeServiceExt};
use spike_wire::proto::metis::spike::v1::{
    EchoRequest, EchoResponse, Event, EventKind, StreamEventsRequest,
};
#[cfg(unix)]
use spike_wire::{local_authority, serve_uds, uds_transport};

/// How many message events the stream carries before it completes.
const MESSAGE_EVENTS: u32 = 3;

struct SpikeServiceImpl;

// The generated trait uses an explicit RPITIT signature; an `async fn` impl
// refines it, which the compiler flags with `refining_impl_trait`.
#[allow(refining_impl_trait)]
impl SpikeService for SpikeServiceImpl {
    async fn echo(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, EchoRequest>,
    ) -> ServiceResult<EchoResponse> {
        Response::ok(EchoResponse {
            message: request.message.to_owned(),
            received_at: now_millis(),
            ..Default::default()
        })
    }

    async fn stream_events(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, StreamEventsRequest>,
    ) -> ServiceResult<ServiceStream<Event>> {
        // `last_event_id` is the resumable cursor: message events with an id at
        // or below it are skipped, heartbeats keep flowing.
        let after: u32 = request.last_event_id.parse().unwrap_or(0);
        let mut events = Vec::new();
        for id in 1..=MESSAGE_EVENTS {
            events.push(Ok(heartbeat_event(id)));
            if id > after {
                events.push(Ok(message_event(id)));
            }
        }
        Response::stream_ok(futures::stream::iter(events))
    }
}

/// A heartbeat is an ordinary stream message; Connect has no SSE comment frame.
fn heartbeat_event(id: u32) -> Event {
    Event {
        id: format!("h{id}"),
        kind: EventKind::EVENT_KIND_HEARTBEAT.into(),
        payload: String::new(),
        ..Default::default()
    }
}

fn message_event(id: u32) -> Event {
    Event {
        id: id.to_string(),
        kind: EventKind::EVENT_KIND_MESSAGE.into(),
        payload: format!("event {id}"),
        ..Default::default()
    }
}

fn now_millis() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default()
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let socket = std::env::temp_dir().join("metis-connectrpc-spike.sock");

    let router = Arc::new(SpikeServiceImpl).register(Router::new());
    let server_socket = socket.clone();
    tokio::spawn(async move {
        if let Err(err) = serve_uds(&server_socket, router).await {
            eprintln!("server error: {err}");
        }
    });
    wait_for_socket(&socket).await;

    let transport = uds_transport(&socket).shared(64);
    let client = SpikeServiceClient::new(transport, ClientConfig::new(local_authority()));

    // Ordinary RPC through the generated client.
    let echoed = client
        .echo(EchoRequest {
            message: "hello over uds".to_owned(),
            ..Default::default()
        })
        .await
        .map_err(|e| -> BoxError { format!("echo call: {e}").into() })?
        .into_owned();
    println!(
        "[echo] service={SPIKE_SERVICE_SERVICE_NAME} message={:?} received_at={}",
        echoed.message, echoed.received_at
    );

    // Server-streaming RPC, from the start.
    let (messages, heartbeats) = collect(&client, "").await?;
    println!("[stream] messages={messages:?} heartbeats={heartbeats}");

    // Resume: ask for everything after message 2, so the first message is 3.
    let (resumed, _) = collect(&client, "2").await?;
    println!(
        "[stream] resume_after=2 first_message={:?}",
        resumed.first()
    );

    let ok = echoed.message == "hello over uds"
        && messages == ["1", "2", "3"]
        && resumed == ["3"]
        && heartbeats == MESSAGE_EVENTS as usize;
    println!("[result] {}", if ok { "PASS" } else { "FAIL" });
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

/// Drain a StreamEvents call and split message ids from heartbeat count.
#[cfg(unix)]
async fn collect(
    client: &SpikeServiceClient<connectrpc::client::SharedHttp2Connection>,
    cursor: &str,
) -> Result<(Vec<String>, usize), ConnectError> {
    let mut stream = client
        .stream_events(StreamEventsRequest {
            last_event_id: cursor.to_owned(),
            ..Default::default()
        })
        .await?;
    let mut messages = Vec::new();
    let mut heartbeats = 0usize;
    while let Some(msg) = stream.message().await? {
        if msg.view().kind == EventKind::EVENT_KIND_MESSAGE {
            messages.push(msg.view().id.to_owned());
        } else {
            heartbeats += 1;
        }
    }
    Ok((messages, heartbeats))
}

#[cfg(unix)]
async fn wait_for_socket(socket: &std::path::Path) {
    for _ in 0..100 {
        if socket.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Windows counterpart, compile-only on this host.
///
/// It wires the same service, a named-pipe server and a named-pipe client
/// transport. It is not run here: the Unix path above is the executed proof,
/// and this main exists so `cargo check --target x86_64-pc-windows-msvc` type
/// checks both ends of the pipe.
#[cfg(windows)]
#[tokio::main]
async fn main() -> Result<(), BoxError> {
    const PIPE: &str = r"\\.\pipe\metis-connectrpc-spike";
    let router = Arc::new(SpikeServiceImpl).register(Router::new());
    let server = spike_wire::serve_named_pipe(PIPE, router);
    let transport = spike_wire::named_pipe_transport(PIPE);
    let _ = (server, transport);
    Ok(())
}
