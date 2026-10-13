//! The identity a client declares on every call, and the version gate.
//!
//! The declaration rides request metadata, not a handshake and not connection
//! state, and it is not a credential: a call that declares nothing but its
//! version is refused for the version, not for the absence of a token. The
//! daemon records what was declared, and refuses a protocol version it does not
//! speak with a typed error that names the ones it does.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use api_contracts::connect::metis::v1::DaemonServiceClient;
use api_contracts::identity::{
    self, PROTOCOL_CLIENT_VERSION_HEADER, PROTOCOL_SURFACE_HEADER, PROTOCOL_VERSION,
    PROTOCOL_VERSION_HEADER, SUPPORTED_VERSIONS_HEADER,
};
use api_contracts::proto::metis::v1::EchoRequest;
use api_contracts::transport::{local_authority, serve_uds, uds_transport};
use client::skeleton::{self, IdentityRecorder};
use client::{Client, Surface};
use connectrpc::ErrorCode;
use connectrpc::client::ClientConfig;

/// A temporary socket directory that removes itself on drop, so cleanup happens
/// on the success path and on a panic alike. It is created private, the way a
/// runtime directory is: the daemon refuses a socket directory that is wider
/// than its rule allows.
struct TempSocket {
    dir: PathBuf,
    path: PathBuf,
}

impl TempSocket {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("metis-identity-{}-{sequence}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the temporary socket directory");
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("restrict the temporary socket directory");
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

/// Serve the recording skeleton on `path` and wait until it is bound.
async fn serve_recording(path: &Path, recorder: &IdentityRecorder) {
    let router = skeleton::router_recording(recorder);
    let server_path = path.to_path_buf();
    tokio::spawn(async move {
        if let Err(err) = serve_uds(&server_path, router).await {
            eprintln!("skeleton server error: {err}");
        }
    });
    assert!(
        skeleton::wait_for_socket(path).await,
        "the listener binds its socket"
    );
}

/// A declaration as a client would send it, for a caller that drives the
/// generated client itself.
fn declared(version: Option<u32>) -> ClientConfig {
    let config = ClientConfig::new(local_authority());
    match version {
        Some(version) => config
            .with_default_header(PROTOCOL_VERSION_HEADER, version.to_string())
            .with_default_header(PROTOCOL_SURFACE_HEADER, "test")
            .with_default_header(PROTOCOL_CLIENT_VERSION_HEADER, "0.0.0-test"),
        None => config,
    }
}

/// The wrapper declares the three parts of its identity on every call, the
/// daemon records them, and the recorder names the surface rather than
/// authenticating it.
#[tokio::test]
async fn the_daemon_records_the_declared_identity() {
    let socket = TempSocket::new();
    let recorder = IdentityRecorder::new();
    serve_recording(socket.path(), &recorder).await;

    let caller = Client::with_endpoint(socket.path())
        .await
        .expect("the local transport builds")
        .with_surface(Surface::Cli);
    let echoed = caller
        .echo("hello with a declaration")
        .await
        .expect("the wrapper performs the echo RPC without any credential");
    assert_eq!(echoed, "hello with a declaration");

    let identity = recorder
        .last()
        .expect("the daemon recorded the declaration");
    assert_eq!(identity.protocol_version(), PROTOCOL_VERSION);
    assert_eq!(identity.surface(), "cli");
    assert_eq!(identity.client_version(), env!("CARGO_PKG_VERSION"));
}

/// A caller that names no surface is recorded as unspecified rather than left
/// unrecorded, and the declaration covers every call the client makes.
#[tokio::test]
async fn the_declared_surface_defaults_to_unspecified() {
    let socket = TempSocket::new();
    let recorder = IdentityRecorder::new();
    serve_recording(socket.path(), &recorder).await;

    let caller = Client::with_endpoint(socket.path())
        .await
        .expect("the local transport builds");
    caller.echo("first").await.expect("the first call lands");

    let identity = recorder.last().expect("the first call was recorded");
    assert_eq!(identity.surface(), Surface::Unspecified.as_str());

    // The surface is replaced, not appended: a second declaration on the same
    // client is the one the daemon reads.
    let caller = caller.with_surface(Surface::Ui);
    caller.echo("second").await.expect("the second call lands");
    let identity = recorder.last().expect("the second call was recorded");
    assert_eq!(identity.surface(), "ui");
    assert_eq!(identity.protocol_version(), PROTOCOL_VERSION);
}

/// A client that declares a protocol version this daemon does not speak is
/// refused with an `InvalidArgument` error, and the refusal lists the versions
/// it does speak as a response header.
#[tokio::test]
async fn a_client_declaring_another_version_is_refused() {
    let socket = TempSocket::new();
    let recorder = IdentityRecorder::new();
    serve_recording(socket.path(), &recorder).await;

    let transport = uds_transport(socket.path().to_path_buf()).shared(64);
    let daemon = DaemonServiceClient::new(transport, declared(Some(PROTOCOL_VERSION + 1)));
    let err = daemon
        .echo(EchoRequest {
            message: "anyone there".to_owned(),
            ..Default::default()
        })
        .await
        .expect_err("version 2 is not supported");

    assert_eq!(err.code, ErrorCode::InvalidArgument);
    assert_eq!(
        err.response_headers().get(SUPPORTED_VERSIONS_HEADER),
        Some(
            &identity::supported_versions_value()
                .parse()
                .expect("a valid header value")
        ),
        "the refusal names what is supported, as a header"
    );
    assert!(
        err.to_string().contains('1'),
        "the message names it too: {err}"
    );
    assert!(
        recorder.last().is_none(),
        "a refused call is not a recorded identity"
    );
}

/// A client that declares no version at all is refused the same way: the
/// declaration is required, and the refusal is typed rather than a message.
#[tokio::test]
async fn a_client_declaring_no_version_is_refused() {
    let socket = TempSocket::new();
    let recorder = IdentityRecorder::new();
    serve_recording(socket.path(), &recorder).await;

    let transport = uds_transport(socket.path().to_path_buf()).shared(64);
    let daemon = DaemonServiceClient::new(transport, declared(None));
    let err = daemon
        .echo(EchoRequest {
            message: "anyone there".to_owned(),
            ..Default::default()
        })
        .await
        .expect_err("a call with no declaration is refused");

    assert_eq!(err.code, ErrorCode::InvalidArgument);
    assert_eq!(
        err.response_headers().get(SUPPORTED_VERSIONS_HEADER),
        Some(
            &identity::supported_versions_value()
                .parse()
                .expect("a valid header value")
        ),
        "a client that declares nothing is still told what is supported"
    );
    assert!(recorder.last().is_none());
}

/// The plain router the walking skeleton lands stays declaration-free, so the
/// earlier tests and the example keep serving a caller that declares nothing.
#[tokio::test]
async fn the_plain_router_serves_a_call_with_no_declaration() {
    let socket = TempSocket::new();
    let server_path = socket.path().to_path_buf();
    tokio::spawn(async move {
        if let Err(err) = skeleton::serve_uds(&server_path).await {
            eprintln!("skeleton server error: {err}");
        }
    });
    assert!(skeleton::wait_for_socket(socket.path()).await);

    let caller = Client::with_endpoint(socket.path())
        .await
        .expect("the local transport builds");
    let echoed = caller
        .echo("no declaration needed here")
        .await
        .expect("the plain router needs no declaration");
    assert_eq!(echoed, "no declaration needed here");
}
