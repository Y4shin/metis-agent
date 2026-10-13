//! The local transport: one serve loop and one connector shape, shared by every
//! front-end.
//!
//! `serve_stream` is the accept-loop body. It is generic over any asynchronous
//! bidirectional stream, so a Unix socket, a named pipe, a TCP socket, or an
//! in-memory duplex pair all reach the same Connect service without a second
//! serve loop. `serve_uds` and `uds_transport` are the Unix front-ends: the
//! later transport tickets add their own listener and connector against the
//! same two shapes.
//!
//! This module is the only place that knows how to serve and connect a local
//! stream. Generated code stays mounted and untouched.

use std::io;
use std::sync::Arc;

use connectrpc::{ConnectRpcService, Router};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite};

/// Serve a Connect router over any asynchronous bidirectional stream.
///
/// The stream is wrapped for hyper, the router is built into a
/// [`ConnectRpcService`] once, and a `hyper_util` auto connection drives
/// HTTP/1.1 or HTTP/2 until the stream closes. The caller owns the listener and
/// the accept loop; this function owns one connection.
///
/// The router is taken through `Into<Arc<Router>>` because `Router` is not
/// `Clone` in connectrpc 0.9.1: a listener builds the router once, wraps it in
/// an `Arc`, and hands each accepted stream a clone of that `Arc`. A one-shot
/// caller passes the router directly.
///
/// # Errors
///
/// Returns the connection's terminal error as an [`io::Error`]. A client that
/// disconnects ends the connection without an error on the server side.
pub async fn serve_stream<S>(stream: S, router: impl Into<Arc<Router>>) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let service = ConnectRpcService::from_arc(router.into());
    let builder = AutoBuilder::new(TokioExecutor::new());
    builder
        .serve_connection(TokioIo::new(stream), TowerToHyperService::new(service))
        .await
        .map_err(io::Error::other)
}

/// Serve a Connect router over a Unix domain socket.
///
/// A stale socket file is removed first, so a crashed listener does not block
/// the next start. Each accepted stream is handed to [`serve_stream`] on its
/// own task.
///
/// # Errors
///
/// Returns the bind or accept error. Accepting runs until the task is dropped.
#[cfg(unix)]
pub async fn serve_uds(path: &std::path::Path, router: Router) -> io::Result<()> {
    let _ = std::fs::remove_file(path);
    let listener = tokio::net::UnixListener::bind(path)?;
    let router = Arc::new(router);
    loop {
        let (stream, _) = listener.accept().await?;
        let router = Arc::clone(&router);
        tokio::spawn(async move {
            if let Err(err) = serve_stream(stream, router).await {
                eprintln!("local connection ended with error: {err}");
            }
        });
    }
}

/// Build an h2c client connection over a Unix domain socket, lazily on first
/// use.
///
/// The connector is a plain `tower::service_fn` that opens the socket, so a
/// later transport supplies its own connector against the same generated client
/// shape. The returned connection is an `Http2Connection`; wrap it with
/// `shared` to obtain a `ClientTransport` for the generated client.
#[cfg(unix)]
pub fn uds_transport(path: impl Into<std::path::PathBuf>) -> connectrpc::client::Http2Connection {
    let path = path.into();
    let connector = tower::service_fn(move |_uri: http::Uri| {
        let path = path.clone();
        async move {
            let stream = tokio::net::UnixStream::connect(path).await?;
            Ok::<_, io::Error>(TokioIo::new(stream))
        }
    });
    connectrpc::client::Http2Connection::lazy_with_connector(connector, local_authority())
}

/// The `:authority` value used for the local IPC transport. The server does not
/// validate it.
///
/// # Panics
///
/// Never in practice: the literal parses.
#[must_use]
pub fn local_authority() -> http::Uri {
    "http://localhost".parse().expect("static authority")
}
