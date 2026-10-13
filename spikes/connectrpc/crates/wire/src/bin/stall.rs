//! Slow/stalled subscriber probe for the ConnectRPC spike.
//!
//! Answers the task's question 6: what the transport does to a subscriber that
//! stops reading. The handler produces an unbounded stream into a bounded
//! channel; the client reads one message, stops polling, then drops the stream,
//! and the server reports how far the producer got and whether it saw the
//! disconnect.
//!
//! This binary is a diagnostic, not part of the comparable handwritten surface.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use connectrpc::client::ClientConfig;
use connectrpc::{
    ConnectError, RequestContext, Response, Router, ServiceRequest, ServiceResult, ServiceStream,
};
use spike_wire::connect::metis::spike::v1::{SpikeService, SpikeServiceClient, SpikeServiceExt};
use spike_wire::proto::metis::spike::v1::{
    EchoRequest, EchoResponse, Event, EventKind, StreamEventsRequest,
};
use spike_wire::{local_authority, serve_uds, uds_transport};

/// Payload per event, chosen so a few messages fill the HTTP/2 flow window.
const CHUNK: usize = 16 * 1024;
/// Bounded channel between the producer task and the response stream.
const CHANNEL: usize = 8;

#[derive(Clone)]
struct StalledServiceImpl {
    produced: Arc<AtomicU64>,
    disconnected: Arc<AtomicBool>,
}

#[allow(refining_impl_trait)]
impl SpikeService for StalledServiceImpl {
    async fn echo(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, EchoRequest>,
    ) -> ServiceResult<EchoResponse> {
        Response::ok(EchoResponse {
            message: request.message.to_owned(),
            ..Default::default()
        })
    }

    async fn stream_events(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, StreamEventsRequest>,
    ) -> ServiceResult<ServiceStream<Event>> {
        let produced = self.produced.clone();
        let disconnected = self.disconnected.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, ConnectError>>(CHANNEL);
        tokio::spawn(async move {
            let mut id = 0u64;
            loop {
                id += 1;
                let event = Event {
                    id: id.to_string(),
                    kind: EventKind::EVENT_KIND_MESSAGE.into(),
                    payload: "x".repeat(CHUNK),
                    ..Default::default()
                };
                produced.fetch_add(1, Ordering::SeqCst);
                if tx.send(Ok(event)).await.is_err() {
                    // The response stream was dropped: client disconnected.
                    disconnected.store(true, Ordering::SeqCst);
                    break;
                }
            }
        });
        let stream = futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });
        Response::stream_ok(stream)
    }
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let socket = std::env::temp_dir().join("metis-connectrpc-stall.sock");
    let produced = Arc::new(AtomicU64::new(0));
    let disconnected = Arc::new(AtomicBool::new(false));

    let service = StalledServiceImpl {
        produced: Arc::clone(&produced),
        disconnected: Arc::clone(&disconnected),
    };
    let router = Arc::new(service).register(Router::new());
    let server_socket = socket.clone();
    tokio::spawn(async move {
        if let Err(err) = serve_uds(&server_socket, router).await {
            eprintln!("server error: {err}");
        }
    });
    wait_for_socket(&socket).await;

    let transport = uds_transport(&socket).shared(8);
    let client = SpikeServiceClient::new(transport, ClientConfig::new(local_authority()));
    let mut stream = client.stream_events(StreamEventsRequest::default()).await?;

    let first = stream.message().await?;
    let after_first = produced.load(Ordering::SeqCst);
    println!("[stall] first_message_received={}", first.is_some());

    // Stall: keep the stream alive but never poll it again. Sample twice to
    // distinguish a bounded plateau from unbounded buffering.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after_stall_short = produced.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let after_stall_long = produced.load(Ordering::SeqCst);

    // Disconnect: drop the stream, which resets the HTTP/2 stream.
    drop(stream);
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let after_drop = produced.load(Ordering::SeqCst);
    let saw_disconnect = disconnected.load(Ordering::SeqCst);

    println!(
        "[stall] produced: after_first={after_first} after_0.5s_stall={after_stall_short} after_2.5s_stall={after_stall_long} after_drop_1s={after_drop}"
    );
    println!("[stall] server_sender_saw_disconnect={saw_disconnect}");
    let plateaued = after_stall_long == after_stall_short;
    println!(
        "[result] {}",
        if first.is_some() && plateaued && after_drop == after_stall_long && saw_disconnect {
            "PASS"
        } else {
            "FAIL"
        }
    );
    Ok(())
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
