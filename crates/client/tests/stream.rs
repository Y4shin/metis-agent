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

use api_contracts::events::Ring;
use api_contracts::proto::metis::v1::EventKind;
use client::skeleton;
use client::{Client, Surface};

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
