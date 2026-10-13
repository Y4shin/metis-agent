//! The UI-independent client for the daemon.
//!
//! The wrapper hides the local transport: a caller constructs a [`Client`] and
//! calls a method, and its API names no transport, no `Http2Connection`, and no
//! generated stub type. A later transport changes the construction, not the
//! caller's code. `Client::local` and `Client::with_endpoint` reach the local
//! socket on Unix, and `Client::with_remote_addr` reaches the optional TCP
//! listener, which is off unless the daemon was given a bind address.
//!
//! `skeleton` holds the test-support service the walking-skeleton test and
//! example drive, and the start paths for it. The daemon effort replaces it;
//! the transport and the wrapper stay.

pub mod skeleton;

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use api_contracts::connect::metis::v1::DaemonServiceClient;
use api_contracts::proto::metis::v1::EchoRequest;
use api_contracts::transport;
use connectrpc::ConnectError;
use connectrpc::client::{ClientConfig, SharedHttp2Connection};

/// The environment variable that points the local client at a non-default
/// endpoint. The daemon effort binds the same variable.
const ENDPOINT_ENV: &str = "METIS_ENDPOINT";

/// The platform default endpoint for the local transport.
///
/// The daemon effort binds the same path. The Windows front-end arrives with
/// its own ticket and supplies the named-pipe default.
#[must_use]
pub fn default_endpoint() -> PathBuf {
    PathBuf::from("/tmp/metis-daemon.sock")
}

/// The endpoint the local client connects to: the environment override when it
/// is set and non-empty, otherwise [`default_endpoint`].
///
/// The wrapper and the example resolve through this one function, so the rule
/// cannot drift between them.
#[must_use]
pub fn resolve_endpoint() -> PathBuf {
    match std::env::var_os(ENDPOINT_ENV) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => default_endpoint(),
    }
}

/// A connection to the daemon over the local transport.
///
/// The wrapper owns the generated client and the injected transport, both
/// private, so the transport stays swappable without moving a caller.
pub struct Client {
    inner: DaemonServiceClient<SharedHttp2Connection>,
}

impl Client {
    /// Connect over the local transport to the resolved endpoint.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] when the local transport cannot be built.
    #[cfg(unix)]
    pub async fn local() -> Result<Self, ClientError> {
        Self::with_endpoint(resolve_endpoint()).await
    }

    /// Connect over the local transport to an explicit endpoint.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] when the local transport cannot be built.
    #[cfg(unix)]
    pub async fn with_endpoint(endpoint: impl Into<PathBuf>) -> Result<Self, ClientError> {
        let transport = transport::uds_transport(endpoint).shared(64);
        Ok(Self {
            inner: DaemonServiceClient::new(
                transport,
                ClientConfig::new(transport::local_authority()),
            ),
        })
    }

    /// Connect over the remote transport to an explicit TCP address.
    ///
    /// The wrapper keeps the transport private, so the caller names no TCP type
    /// and no generated stub. This constructor takes the address; it does not
    /// read the server's bind variable, because how a client finds a remote
    /// daemon is the pairing ticket's subject.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] when the remote transport cannot be built.
    pub async fn with_remote_addr(addr: SocketAddr) -> Result<Self, ClientError> {
        let transport = transport::tcp_transport(addr).shared(64);
        Ok(Self {
            inner: DaemonServiceClient::new(
                transport,
                ClientConfig::new(transport::remote_authority(addr)),
            ),
        })
    }

    /// Perform the echo RPC and return the daemon's echoed message.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] when the call fails on the transport or the
    /// daemon answers with a protocol error.
    pub async fn echo(&self, message: &str) -> Result<String, ClientError> {
        let request = EchoRequest {
            message: message.to_owned(),
            ..Default::default()
        };
        Ok(self.inner.echo(request).await?.into_owned().message)
    }
}

/// The wrapper's typed error over the transport and protocol failures.
///
/// The credential-specific variants arrive with the authentication ticket, so
/// the enum is non-exhaustive from outside the crate.
#[derive(Debug)]
#[non_exhaustive]
pub enum ClientError {
    /// The RPC did not complete: the transport failed, or the daemon answered
    /// with a protocol error.
    Call(ConnectError),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Call(err) => write!(f, "echo call failed: {err}"),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Call(err) => Some(err),
        }
    }
}

impl From<ConnectError> for ClientError {
    fn from(err: ConnectError) -> Self {
        Self::Call(err)
    }
}
