//! The wrapper's event stream, over the skeleton's own socket.
//!
//! The daemon's stream is asserted in `crates/daemon/tests/stream.rs`; this is
//! the in-process half, which is what the walkthrough example drives: the
//! skeleton serves a ring on a temporary socket, the wrapper opens the stream
//! without naming a transport or a generated stub, and the snapshot and the
//! event that follows arrive in order.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use api_contracts::cancel::CancelOutcome;
use api_contracts::events::Ring;
use api_contracts::proto::metis::v1::EventKind;
use client::skeleton;
use client::{Client, ClientError, Surface};

/// A temporary socket directory that removes itself on drop, so the socket
/// directory is cleaned up on the success path and on a panic alike.
///
/// The daemon refuses a socket directory wider than its rule allows, so the
/// test's base is private, exactly as a runtime directory is.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "metis-client-stream-{label}-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the temporary directory");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("restrict the temporary socket directory");
        Self { path }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Serve the skeleton over `ring` on `endpoint`, heartbeating every
/// `heartbeat`, and return once the socket is bound.
async fn serve(endpoint: &Path, ring: Ring, heartbeat: Duration) {
    let served_endpoint = endpoint.to_path_buf();
    tokio::spawn(async move {
        if let Err(err) = skeleton::serve_uds_with(&served_endpoint, ring, heartbeat).await {
            eprintln!("skeleton server error: {err}");
        }
    });
    assert!(
        skeleton::wait_for_socket(endpoint).await,
        "the skeleton binds its socket"
    );
}

/// The stream opens with the snapshot of the selected channel and then carries
/// the event published while it was open.
#[tokio::test]
async fn the_wrapper_drains_the_stream_the_skeleton_serves() {
    let directory = TempDir::new("drain");
    let endpoint = directory.join("daemon.sock");
    let ring = Ring::new(8);
    // The event the stream will open with: it exists before the stream does, so
    // only the snapshot can carry it.
    ring.publish("daemon", EventKind::Event, "{\"head\":true}");
    serve(&endpoint, ring.clone(), skeleton::HEARTBEAT_DEFAULT).await;

    let caller = Client::with_endpoint(&endpoint)
        .await
        .expect("the local transport builds")
        .with_surface(Surface::Test)
        .expect("the stored credential is readable");
    let mut stream = caller
        .stream_events(&["daemon"], 0)
        .await
        .expect("the stream is established");

    let snapshot = stream
        .message()
        .await
        .expect("the stream carries a message")
        .expect("the stream has not ended");
    assert_eq!(snapshot.kind, EventKind::Snapshot);
    assert_eq!(snapshot.channel, "daemon");
    assert_eq!(snapshot.payload, "{\"head\":true}");

    let published = ring.publish("daemon", EventKind::Event, "{\"next\":true}");
    let event = stream
        .message()
        .await
        .expect("the stream carries a message")
        .expect("the stream has not ended");
    assert_eq!(event.kind, EventKind::Event);
    assert_eq!(event.id, published);
    assert!(
        event.id > snapshot.id,
        "the event that follows the snapshot has a greater identifier"
    );
    assert_eq!(event.payload, "{\"next\":true}");
}

/// An expired cursor is the wrapper's typed refusal, read from the header the
/// skeleton wrote, and the answer is the resync: the cursorless form of the
/// same call, whose first message is the snapshot.
#[tokio::test]
async fn an_expired_cursor_is_a_typed_refusal_and_the_resync_opens_with_the_snapshot() {
    let directory = TempDir::new("expired");
    let endpoint = directory.join("daemon.sock");
    let ring = Ring::new(8);
    ring.publish("daemon", EventKind::Event, "seen");
    // Move the skeleton's window past identifier 1. A cursor of
    // `earliest_resumable() - 1` is still servable, because the event that
    // opened the window follows it, so the window has to begin at 3 or beyond
    // before identifier 1 is older than the ring.
    while ring.earliest_resumable() <= 2 {
        ring.publish("daemon", EventKind::Event, "{}");
        assert!(
            ring.latest_id() < 10_000,
            "the ring's window must advance past identifier 1"
        );
    }
    serve(&endpoint, ring.clone(), skeleton::HEARTBEAT_DEFAULT).await;

    let caller = Client::with_endpoint(&endpoint)
        .await
        .expect("the local transport builds")
        .with_surface(Surface::Test)
        .expect("the stored credential is readable");

    let refused = caller
        .stream_events(&["daemon"], 1)
        .await
        .expect_err("a cursor older than the window is refused");
    assert!(
        matches!(refused, ClientError::CursorExpired),
        "expected the typed expired-cursor refusal, got {refused}"
    );

    let mut resync = caller
        .stream_events(&["daemon"], 0)
        .await
        .expect("a cursorless stream is served");
    let snapshot = resync
        .message()
        .await
        .expect("the stream carries a message")
        .expect("the stream has not ended");
    assert_eq!(snapshot.kind, EventKind::Snapshot);
    assert_eq!(snapshot.id, ring.latest_id());
    assert_eq!(snapshot.channel, "daemon");
}

/// A stream over a channel nothing was published to still opens with the
/// snapshot, which is the empty one, and then carries heartbeats: ordinary
/// messages carrying the ring's latest identifier.
#[tokio::test]
async fn a_channel_with_no_events_carries_the_skeleton_heartbeat() {
    let directory = TempDir::new("quiet");
    let endpoint = directory.join("daemon.sock");
    let ring = Ring::new(8);
    // Short, so the test waits milliseconds rather than the daemon's interval.
    serve(&endpoint, ring.clone(), Duration::from_millis(25)).await;

    let caller = Client::with_endpoint(&endpoint)
        .await
        .expect("the local transport builds");
    let mut stream = caller
        .stream_events(&["daemon"], 0)
        .await
        .expect("the stream is established");

    let empty = stream
        .message()
        .await
        .expect("the stream carries a message")
        .expect("the stream has not ended");
    assert_eq!(empty.kind, EventKind::Snapshot);
    assert_eq!(empty.id, 0);
    assert!(empty.channel.is_empty());

    // An event on a channel this stream did not select must not reach it, so
    // what arrives is the heartbeat of the idle selected channel.
    ring.publish("session:7", EventKind::Event, "{}");
    let heartbeat = tokio::time::timeout(Duration::from_secs(5), stream.message())
        .await
        .expect("the skeleton heartbeats an idle stream")
        .expect("the stream carries a message")
        .expect("the stream has not ended");
    assert_eq!(heartbeat.kind, EventKind::Heartbeat);
    assert_eq!(
        heartbeat.id, 1,
        "the heartbeat carries the latest identifier"
    );
    assert!(heartbeat.channel.is_empty());
}

/// Publish into `ring` until it holds far less than it has been given, so a
/// subscriber that read nothing has necessarily fallen past the bound between
/// the ring and a reader. Stated in the ring's own observable bounds rather than
/// in a capacity a test does not own, and bounded so a much larger window fails
/// loudly instead of hanging.
///
/// The helper is deliberately duplicated, with the same 10 000 bound, in
/// `crates/daemon/tests/cancel.rs`: the two live in separate test binaries over
/// separate surfaces (the wrapper's drop here, the daemon's refusal there), and
/// sharing one helper would mean a test-support crate both binaries depend on.
fn stall_past_the_bound(ring: &Ring) {
    ring.publish("daemon", EventKind::Event, "{}");
    while ring.since(0, &["daemon"]).len() * 2 > ring.latest_id() as usize {
        ring.publish("daemon", EventKind::Event, "{}");
        assert!(
            ring.latest_id() < 10_000,
            "the ring's window must fill so a stalled reader falls behind"
        );
    }
}

/// A reader that stops reading is dropped, the wrapper reports the drop as the
/// typed error rather than as a clean end or a transport failure, and the stream
/// carries nothing after it. The answer is the resync, which is the cursorless
/// form of the same call: a fresh stream whose first message is the snapshot of
/// the channels' current heads, replacing the state the caller held rather than
/// appending to it.
#[tokio::test]
async fn a_dropped_stream_is_resync_required_and_the_resync_opens_with_the_snapshot() {
    let directory = TempDir::new("dropped");
    let endpoint = directory.join("daemon.sock");
    // A small window, so the bound is reachable from a test rather than from a
    // production amount of traffic.
    let ring = Ring::new(4);
    serve(&endpoint, ring.clone(), skeleton::HEARTBEAT_DEFAULT).await;

    let caller = Client::with_endpoint(&endpoint)
        .await
        .expect("the local transport builds");
    let mut stream = caller
        .stream_events(&["daemon"], 0)
        .await
        .expect("the stream is established");

    let opening = stream
        .message()
        .await
        .expect("the stream carries a message")
        .expect("the stream has not ended");
    assert_eq!(opening.kind, EventKind::Snapshot);

    // The reader stops reading while the ring fills past its window.
    stall_past_the_bound(&ring);

    let dropped = stream
        .message()
        .await
        .expect_err("a stalled reader is dropped rather than carried across a gap");
    assert!(
        matches!(dropped, ClientError::StreamDropped),
        "expected the typed drop, got {dropped}"
    );

    // Nothing follows it, and the stream does not read as a clean end either:
    // the gap is never presented as current state.
    let again = stream.message().await;
    assert!(
        matches!(again, Err(ClientError::StreamDropped)),
        "the drop is terminal and sticky, got {again:?}"
    );

    // The resync: a fresh cursorless stream, which opens with the snapshot. Its
    // head is the current state, not the events that were missed.
    let head = ring.publish("daemon", EventKind::Event, "{\"head\":true}");
    let mut resync = caller
        .stream_events(&["daemon"], 0)
        .await
        .expect("a cursorless stream is served");
    let snapshot = resync
        .message()
        .await
        .expect("the stream carries a message")
        .expect("the stream has not ended");
    assert_eq!(snapshot.kind, EventKind::Snapshot);
    assert_eq!(snapshot.id, head, "the head is the current state");
    assert_eq!(snapshot.channel, "daemon");
    assert_eq!(snapshot.payload, "{\"head\":true}");
}

/// A cancelled stream stops, and the wrapper reports it as the typed cancelled
/// refusal rather than as an event or a clean end. The identifier the handle
/// carries is the one the daemon registered the work under, so a caller never
/// invents one to cancel by.
#[tokio::test]
async fn a_cancelled_stream_stops_as_the_typed_refusal() {
    let directory = TempDir::new("cancelled");
    let endpoint = directory.join("daemon.sock");
    let ring = Ring::new(8);
    serve(&endpoint, ring.clone(), skeleton::HEARTBEAT_DEFAULT).await;

    let caller = Client::with_endpoint(&endpoint)
        .await
        .expect("the local transport builds");
    let mut stream = caller
        .stream_events(&["daemon"], 0)
        .await
        .expect("the stream is established");
    let opening = stream
        .message()
        .await
        .expect("the stream carries a message")
        .expect("the stream has not ended");
    assert_eq!(opening.kind, EventKind::Snapshot);
    let id = stream.request_id().clone();

    assert_eq!(
        caller
            .cancel(&id)
            .await
            .expect("the cancel reaches the daemon"),
        CancelOutcome::Cancelled
    );

    let stopped = stream
        .message()
        .await
        .expect_err("a cancelled stream is stopped");
    assert!(
        matches!(stopped, ClientError::StreamCancelled),
        "expected the typed cancelled refusal, got {stopped}"
    );
    assert!(
        stream.message().await.is_err(),
        "a stopped stream carries nothing after it"
    );
    assert_eq!(
        caller
            .cancel(&id)
            .await
            .expect("a repeated cancel is answered"),
        CancelOutcome::AlreadyFinished
    );
}
