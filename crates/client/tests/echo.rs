//! The local transport and the client wrapper, exercised end to end.
//!
//! The seams are the transport loop, the served path over a real Unix socket,
//! the endpoint rule, and the wrapper call that names no transport. The
//! skeleton service and a temporary socket directory are the shared setup.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use api_contracts::connect::metis::v1::DaemonServiceClient;
use api_contracts::proto::metis::v1::EchoRequest;
use api_contracts::transport::{local_authority, serve_stream};
use client::Client;
use client::skeleton;
use connectrpc::client::{ClientConfig, Http2Connection};
use hyper_util::rt::TokioIo;
use tower::service_fn;

/// A temporary socket directory that removes itself on drop, so the socket
/// directory is cleaned up on the success path and on a panic alike.
struct TempSocket {
    dir: PathBuf,
    path: PathBuf,
}

impl TempSocket {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("metis-echo-{}-{sequence}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the temporary socket directory");
        let path = dir.join("daemon.sock");
        Self { dir, path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Start the skeleton on `path` and wait until the listener has bound it.
async fn serve_skeleton(path: &Path) {
    let server_path = path.to_path_buf();
    tokio::spawn(async move {
        if let Err(err) = skeleton::serve_uds(&server_path).await {
            eprintln!("skeleton server error: {err}");
        }
    });
    assert!(
        skeleton::wait_for_socket(path).await,
        "the listener binds its socket"
    );
}

/// Serve one router over an in-memory duplex pair and drive it with the
/// generated client. Nothing here knows about a Unix socket, which is the
/// point: the loop is generic over any asynchronous bidirectional stream.
#[tokio::test]
async fn serve_stream_serves_a_router_over_a_duplex_stream() {
    let (client_io, server_io) = tokio::io::duplex(4096);
    tokio::spawn(serve_stream(server_io, skeleton::router()));

    // `lazy_with_connector` may call the connector once; the single duplex
    // half is handed over on that call.
    let mut client_io = Some(client_io);
    let connector = service_fn(move |_uri: http::Uri| {
        let client_io = client_io.take().expect("one connection attempt");
        async move { Ok::<_, std::io::Error>(TokioIo::new(client_io)) }
    });

    let transport = Http2Connection::lazy_with_connector(connector, local_authority());
    let daemon =
        DaemonServiceClient::new(transport.shared(64), ClientConfig::new(local_authority()));

    let response = daemon
        .echo(EchoRequest {
            message: "hello over a duplex".to_owned(),
            ..Default::default()
        })
        .await
        .expect("the duplex stream carries the call");

    assert_eq!(response.view().message, "hello over a duplex");
    // The contract says `received_at` is an RFC 3339 UTC string.
    assert!(
        response.view().received_at.ends_with('Z'),
        "received_at is an RFC 3339 UTC stamp, got {:?}",
        response.view().received_at
    );

    // Keep the test bounded: `#[tokio::test]` uses a current-thread runtime,
    // and the server task above completes when the connection closes.
    tokio::time::sleep(Duration::from_millis(10)).await;
}

/// The wrapper reaches the skeleton over a real socket and returns the echoed
/// message. The temporary socket directory is removed by the guard on drop,
/// which is the success path and the failure path alike.
#[tokio::test]
async fn wrapper_echoes_over_a_temporary_socket() {
    let socket = TempSocket::new();
    let directory = socket.dir.clone();
    serve_skeleton(socket.path()).await;

    let caller = Client::with_endpoint(socket.path())
        .await
        .expect("the local transport builds");
    let echoed = caller
        .echo("hello over the socket")
        .await
        .expect("the wrapper performs the echo RPC");

    assert_eq!(echoed, "hello over the socket");

    drop(caller);
    drop(socket);
    assert!(
        !directory.exists(),
        "the temporary socket directory is removed"
    );
}
