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
use client::{default_endpoint, resolve_endpoint};
use connectrpc::client::{ClientConfig, Http2Connection};
use hyper_util::rt::TokioIo;
use tower::service_fn;

/// The environment variable that overrides the local endpoint. The daemon
/// effort binds the same variable.
const ENDPOINT_ENV: &str = "METIS_ENDPOINT";

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

/// Sets `METIS_ENDPOINT` for the duration of a test and restores the previous
/// value on drop, so a panic cannot leak the override into another test.
struct EndpointOverride {
    previous: Option<std::ffi::OsString>,
}

impl EndpointOverride {
    fn set(value: &Path) -> Self {
        let previous = std::env::var_os(ENDPOINT_ENV);
        // Edition 2024 makes mutating the process environment unsafe because it
        // is not thread-safe. This is the only test in the binary that reads or
        // writes the variable, so nothing races it.
        unsafe { std::env::set_var(ENDPOINT_ENV, value) };
        Self { previous }
    }
}

impl Drop for EndpointOverride {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(ENDPOINT_ENV, value),
                None => std::env::remove_var(ENDPOINT_ENV),
            }
        }
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

/// The platform default is the literal the effort fixes, not whatever the
/// environment happens to hold.
#[test]
fn default_endpoint_is_the_platform_default() {
    assert_eq!(default_endpoint(), PathBuf::from("/tmp/metis-daemon.sock"));
}

/// The environment override redirects the local constructor to its socket, and
/// an empty override means "unset" rather than "the empty path".
#[tokio::test]
async fn environment_override_redirects_the_local_constructor() {
    let socket = TempSocket::new();
    serve_skeleton(socket.path()).await;

    let override_guard = EndpointOverride::set(socket.path());
    assert_eq!(resolve_endpoint(), socket.path().to_path_buf());

    // The local constructor resolves the override and reaches the skeleton. No
    // transport, `Http2Connection`, or generated stub type is named here.
    let caller = Client::local().await.expect("the local client builds");
    let echoed = caller
        .echo("hello through the override")
        .await
        .expect("the override is honoured");
    assert_eq!(echoed, "hello through the override");

    unsafe { std::env::set_var(ENDPOINT_ENV, "") };
    assert_eq!(resolve_endpoint(), default_endpoint());

    drop(override_guard);
}
