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
use std::net::SocketAddr;
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

/// Serve a Connect router over a TCP socket.
///
/// The listener is bound here, so a requested port of `0` is resolved by the
/// operating system. Each accepted stream is handed to [`serve_stream`] on its
/// own task, with a clone of the one router. The transport is plaintext: TLS
/// and the proxy trust rule belong to their own tickets.
///
/// # Errors
///
/// Returns the bind or accept error. Accepting runs until the task is dropped.
pub async fn serve_tcp(addr: SocketAddr, router: Router) -> io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    serve_tcp_listener(listener, router).await
}

/// Serve a Connect router over an already bound TCP listener.
///
/// This is the same accept loop [`serve_tcp`] runs, not a second one. The split
/// exists for a caller that must report the address the operating system
/// assigned to a port of `0`: it binds first, reads `local_addr`, and hands the
/// listener on. `client::skeleton::serve_remote` is that caller.
///
/// # Errors
///
/// Returns the accept error. Accepting runs until the task is dropped.
pub async fn serve_tcp_listener(
    listener: tokio::net::TcpListener,
    router: Router,
) -> io::Result<()> {
    let router = Arc::new(router);
    loop {
        let (stream, _) = listener.accept().await?;
        let router = Arc::clone(&router);
        tokio::spawn(async move {
            if let Err(err) = serve_stream(stream, router).await {
                eprintln!("remote connection ended with error: {err}");
            }
        });
    }
}

/// Build an h2c client connection over TCP, lazily on first use.
///
/// The connector is the shape [`uds_transport`] uses: a plain
/// `tower::service_fn` that opens the socket. The authority is `http://<addr>`.
pub fn tcp_transport(addr: SocketAddr) -> connectrpc::client::Http2Connection {
    let connector = tower::service_fn(move |_uri: http::Uri| async move {
        let stream = tokio::net::TcpStream::connect(addr).await?;
        Ok::<_, io::Error>(TokioIo::new(stream))
    });
    connectrpc::client::Http2Connection::lazy_with_connector(connector, remote_authority(addr))
}

/// The `:authority` value used for a TCP transport to `addr`.
///
/// The connector and the generated client both name it, so a request carries
/// the address the connection was opened to.
///
/// # Panics
///
/// Never in practice: a socket address literal is a valid authority.
#[must_use]
pub fn remote_authority(addr: SocketAddr) -> http::Uri {
    format!("http://{addr}")
        .parse()
        .expect("a socket address is a valid authority")
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

/// The environment variable that names the remote bind address.
///
/// Naming an `IP:port` literal enables the remote listener; an absent or empty
/// value leaves it off. There is no default address, so remote access cannot be
/// enabled by accident.
pub const REMOTE_ADDR_ENV: &str = "METIS_TCP_ADDR";

/// The configuration could not be applied.
///
/// The first configuration error in the repository. It is non-exhaustive
/// because the transport tickets that follow add their own settings.
#[derive(Debug)]
#[non_exhaustive]
pub enum ConfigError {
    /// The variable names something that is not an `IP:port` literal.
    InvalidRemoteAddr {
        /// The variable that was read.
        variable: &'static str,
        /// The value it held.
        value: String,
        /// The address parse failure.
        source: std::net::AddrParseError,
    },
    /// The configured remote bind address could not be bound.
    RemoteBind {
        /// The address that was refused.
        addr: SocketAddr,
        /// The bind failure.
        source: io::Error,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRemoteAddr {
                variable, value, ..
            } => write!(f, "{variable} is not an `IP:port` literal: {value:?}"),
            Self::RemoteBind { addr, source } => {
                write!(f, "cannot bind the remote listener to {addr}: {source}")
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidRemoteAddr { source, .. } => Some(source),
            Self::RemoteBind { source, .. } => Some(source),
        }
    }
}

/// Resolve the remote bind address from the environment.
///
/// An absent or empty [`REMOTE_ADDR_ENV`] is `None`, which is the whole of off
/// by default. An `IP:port` literal is `Some`; anything else, a bare hostname
/// included, is a [`ConfigError`] naming the variable and the value, never a
/// silent disable. The literal is parsed here and bound later, so resolution
/// happens at bind time.
///
/// # Errors
///
/// Returns [`ConfigError::InvalidRemoteAddr`] when the variable holds anything
/// that is not an `IP:port` literal.
pub fn resolve_remote_addr() -> Result<Option<SocketAddr>, ConfigError> {
    let Some(value) = std::env::var_os(REMOTE_ADDR_ENV) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    let value = value.to_string_lossy().into_owned();
    value
        .parse::<SocketAddr>()
        .map(Some)
        .map_err(|source| ConfigError::InvalidRemoteAddr {
            variable: REMOTE_ADDR_ENV,
            value,
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::{REMOTE_ADDR_ENV, resolve_remote_addr};
    use std::ffi::OsString;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// Serialises the environment in this test binary: `std::env` is process
    /// wide and the harness runs the tests on threads.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Holds one value of `METIS_TCP_ADDR` for the duration of a test and
    /// restores the previous value on drop, so a panic cannot leak the value
    /// into another test. The lock it carries serialises every environment
    /// test in this binary.
    struct RemoteAddrEnv {
        previous: Option<OsString>,
        _lock: MutexGuard<'static, ()>,
    }

    impl RemoteAddrEnv {
        fn set(value: &str) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            let previous = std::env::var_os(REMOTE_ADDR_ENV);
            // Edition 2024 makes mutating the process environment unsafe
            // because it is not thread-safe. Every test that touches the
            // variable holds `ENV_LOCK`, so nothing races this write.
            unsafe { std::env::set_var(REMOTE_ADDR_ENV, value) };
            Self {
                previous,
                _lock: lock,
            }
        }

        fn unset() -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            let previous = std::env::var_os(REMOTE_ADDR_ENV);
            // SAFETY: see `set`.
            unsafe { std::env::remove_var(REMOTE_ADDR_ENV) };
            Self {
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for RemoteAddrEnv {
        fn drop(&mut self) {
            // SAFETY: see `set`; the lock is held until this guard is dropped.
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var(REMOTE_ADDR_ENV, value),
                    None => std::env::remove_var(REMOTE_ADDR_ENV),
                }
            }
        }
    }

    /// An absent variable and an empty one both mean "no remote listener",
    /// which is the whole of off by default: there is no address to fall back
    /// to.
    #[test]
    fn an_absent_or_empty_remote_addr_disables_the_listener() {
        let absent = RemoteAddrEnv::unset();
        assert_eq!(
            resolve_remote_addr().expect("an absent variable is not an error"),
            None
        );
        drop(absent);

        let empty = RemoteAddrEnv::set("");
        assert_eq!(
            resolve_remote_addr().expect("an empty value is treated as unset"),
            None
        );
        drop(empty);
    }

    /// Naming an `IP:port` literal is the enablement, for both address
    /// families, and a port of `0` is allowed so the operating system can pick
    /// one at bind time.
    #[test]
    fn an_ip_literal_with_a_port_is_the_bind_address() {
        let env = RemoteAddrEnv::set("127.0.0.1:0");
        assert_eq!(
            resolve_remote_addr().expect("an IPv4 literal with a port parses"),
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        );

        // SAFETY: `env` holds `ENV_LOCK` for the rest of this test.
        unsafe { std::env::set_var(REMOTE_ADDR_ENV, "127.0.0.1:8443") };
        assert_eq!(
            resolve_remote_addr().expect("an IPv4 literal parses"),
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8443))
        );

        // SAFETY: `env` holds `ENV_LOCK` for the rest of this test.
        unsafe { std::env::set_var(REMOTE_ADDR_ENV, "[::1]:8443") };
        assert_eq!(
            resolve_remote_addr().expect("an IPv6 literal parses"),
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 8443))
        );

        drop(env);
    }

    /// Anything that is not an `IP:port` literal is a typed error naming the
    /// variable and the value. Never a silent disable: a misspelled address
    /// must not look like an intentional one.
    #[test]
    fn a_value_that_is_not_an_ip_port_literal_is_a_typed_error() {
        for value in ["127.0.0.1", "example.com:80", "not an address", "127.0.0.1:port"] {
            let env = RemoteAddrEnv::set(value);
            let error = resolve_remote_addr().expect_err("a non-literal value is refused");
            let message = error.to_string();
            assert!(
                message.contains(REMOTE_ADDR_ENV),
                "the error names the variable: {message}"
            );
            assert!(
                message.contains(value),
                "the error names the value: {message}"
            );
            drop(env);
        }
    }
}
