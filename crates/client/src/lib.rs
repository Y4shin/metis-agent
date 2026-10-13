//! The UI-independent client for the daemon.
//!
//! The wrapper hides the local transport: a caller constructs a [`Client`] and
//! calls a method, and its API names no transport, no `Http2Connection`, and no
//! generated stub type. A later transport changes the construction, not the
//! caller's code. `Client::local` and `Client::with_endpoint` reach the local
//! socket on Unix, and `Client::with_remote_addr` reaches the optional TCP
//! listener, which is off unless the daemon was given a bind address.
//!
//! Every client declares an identity on every call: the protocol version, the
//! [`Surface`] it serves, and its own version, as request metadata the daemon
//! records. The declaration is not a credential, and no call is authenticated
//! by it.
//!
//! `skeleton` holds the test-support service the walking-skeleton test and
//! example drive, and the start paths for it. The daemon effort replaces it;
//! the transport and the wrapper stay.

pub mod credential;
pub mod skeleton;

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use api_contracts::connect::metis::v1::DaemonServiceClient;
use api_contracts::identity;
use api_contracts::proto::metis::v1::EchoRequest;
use api_contracts::transport;
use connectrpc::ConnectError;
use connectrpc::client::{ClientConfig, SharedHttp2Connection};

/// The environment variable that points the local client at a non-default
/// endpoint. The daemon effort binds the same variable.
const ENDPOINT_ENV: &str = "METIS_ENDPOINT";

/// The first private base the default endpoint is derived from. The daemon
/// effort reads the same variable.
const RUNTIME_DIR_ENV: &str = "XDG_RUNTIME_DIR";

/// The second private base. A runtime directory is the right home for a socket
/// and is per-user; a temporary directory is the fallback, and the daemon
/// creates the `metis` directory under it as its own.
const TEMP_DIR_ENV: &str = "TMPDIR";

/// The socket path below the chosen private base.
const ENDPOINT_PATH: &str = "metis/daemon.sock";

/// This build's own version, declared to the daemon on every call.
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The endpoint could not be resolved.
///
/// The enum is non-exhaustive: a later transport adds its own resolution
/// failures without breaking a caller that matches on it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EndpointError {
    /// Neither `XDG_RUNTIME_DIR` nor `TMPDIR` names a private base, so there is
    /// no path the daemon could own. There is deliberately no `/tmp` fallback:
    /// a shared, world-writable directory cannot carry the socket's access
    /// control.
    NoPrivateBase,
}

impl fmt::Display for EndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPrivateBase => write!(
                f,
                "neither {RUNTIME_DIR_ENV} nor {TEMP_DIR_ENV} names a private base for the local socket"
            ),
        }
    }
}

impl std::error::Error for EndpointError {}

/// The platform default endpoint for the local transport.
///
/// The socket lives in a private base: `$XDG_RUNTIME_DIR` when it is set and
/// non-empty, otherwise `$TMPDIR`, and otherwise there is no default and the
/// caller gets [`EndpointError::NoPrivateBase`]. The daemon creates the `metis`
/// directory below that base as its own and restricts it, which a shared path
/// could not carry.
///
/// The daemon effort binds the same path. The Windows front-end arrives with
/// its own ticket and supplies the named-pipe default.
///
/// # Errors
///
/// Returns [`EndpointError::NoPrivateBase`] when neither variable names a
/// private base.
pub fn default_endpoint() -> Result<PathBuf, EndpointError> {
    for variable in [RUNTIME_DIR_ENV, TEMP_DIR_ENV] {
        if let Some(base) = private_base(variable) {
            return Ok(base.join(ENDPOINT_PATH));
        }
    }
    Err(EndpointError::NoPrivateBase)
}

/// The endpoint the local client connects to: the environment override when it
/// is set and non-empty, otherwise [`default_endpoint`].
///
/// The wrapper and the example resolve through this one function, so the rule
/// cannot drift between them. The override is what an operator, a test, or a
/// deployment names its own endpoint with; it is never rejected, because the
/// caller that sets it owns the path's access control.
///
/// # Errors
///
/// Returns [`EndpointError::NoPrivateBase`] when there is no override and
/// neither private base is set.
pub fn resolve_endpoint() -> Result<PathBuf, EndpointError> {
    match std::env::var_os(ENDPOINT_ENV) {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => default_endpoint(),
    }
}

/// A variable that names a private base, or `None` when it is unset or empty.
fn private_base(variable: &str) -> Option<PathBuf> {
    let value = std::env::var_os(variable)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

/// The surface a client speaks for. It travels as request metadata so the
/// daemon's record says which surface did what; it grants nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The command-line client.
    Cli,
    /// The desktop user interface.
    Ui,
    /// A test or another automated caller.
    Test,
    /// A caller that did not name its surface.
    Unspecified,
}

impl Surface {
    /// The name the surface declares on the wire.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Ui => "ui",
            Self::Test => "test",
            Self::Unspecified => "unspecified",
        }
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
    /// Returns [`ClientError::Endpoint`] when there is no endpoint to resolve,
    /// and any other [`ClientError`] when the local transport cannot be built.
    #[cfg(unix)]
    pub async fn local() -> Result<Self, ClientError> {
        Self::with_endpoint(resolve_endpoint()?).await
    }

    /// Connect over the local transport to an explicit endpoint.
    ///
    /// The declared identity is [`Surface::Unspecified`]; a caller that knows
    /// which surface it serves names it with [`Client::with_surface`].
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
                declare(
                    ClientConfig::new(transport::local_authority()),
                    Surface::Unspecified,
                ),
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
                declare(
                    ClientConfig::new(transport::remote_authority(addr)),
                    Surface::Unspecified,
                ),
            ),
        })
    }

    /// Declare which surface this client speaks for.
    ///
    /// The surface is request metadata the daemon records; it authenticates
    /// nothing. The declaration is rebuilt rather than appended, so the
    /// surface header is replaced instead of duplicated: the generated client
    /// exposes its configuration as a mutable reference, and `connectrpc`
    /// 0.9.1 offers no mutable default-header accessor.
    #[must_use]
    pub fn with_surface(mut self, surface: Surface) -> Self {
        let authority = self.inner.config().base_uri().clone();
        *self.inner.config_mut() = declare(ClientConfig::new(authority), surface);
        self
    }

    /// Perform the echo RPC and return the daemon's echoed message.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::ProtocolVersion`] when the daemon refuses this
    /// client's declared protocol version, and any other [`ClientError`] when
    /// the call fails on the transport or the daemon answers with a protocol
    /// error.
    pub async fn echo(&self, message: &str) -> Result<String, ClientError> {
        let request = EchoRequest {
            message: message.to_owned(),
            ..Default::default()
        };
        match self.inner.echo(request).await {
            Ok(response) => Ok(response.into_owned().message),
            Err(err) => Err(version_refusal(&err).unwrap_or(ClientError::Call(err))),
        }
    }
}

/// The configuration every constructor builds: the given configuration plus
/// the identity declaration each call carries as request metadata.
fn declare(config: ClientConfig, surface: Surface) -> ClientConfig {
    config
        .with_default_header(
            identity::PROTOCOL_VERSION_HEADER,
            identity::PROTOCOL_VERSION.to_string(),
        )
        .with_default_header(identity::PROTOCOL_SURFACE_HEADER, surface.as_str())
        .with_default_header(identity::PROTOCOL_CLIENT_VERSION_HEADER, CLIENT_VERSION)
}

/// The typed error for a refusal that lists supported protocol versions.
///
/// The list is read from the refusal's response header, never from its message
/// text, and `found` is the version this client declared. `None` means the
/// failure is not a version refusal, and the caller keeps the transport error.
fn version_refusal(err: &ConnectError) -> Option<ClientError> {
    let header = err
        .response_headers()
        .get(identity::SUPPORTED_VERSIONS_HEADER)?;
    let supported = identity::parse_supported_versions(header.to_str().ok()?);
    if supported.is_empty() {
        return None;
    }
    Some(ClientError::ProtocolVersion {
        found: identity::PROTOCOL_VERSION,
        supported,
    })
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
    /// The local endpoint could not be resolved.
    Endpoint(EndpointError),
    /// The daemon refused the protocol version this client declared, and said
    /// which versions it supports.
    ProtocolVersion {
        /// The version this client declared.
        found: u32,
        /// The versions the daemon supports.
        supported: Vec<u32>,
    },
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Call(err) => write!(f, "echo call failed: {err}"),
            Self::Endpoint(err) => write!(f, "the local endpoint could not be resolved: {err}"),
            Self::ProtocolVersion { found, supported } => {
                let supported = supported
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    f,
                    "the daemon refuses protocol version {found}; it supports: {supported}"
                )
            }
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Call(err) => Some(err),
            Self::Endpoint(err) => Some(err),
            Self::ProtocolVersion { .. } => None,
        }
    }
}

impl From<ConnectError> for ClientError {
    fn from(err: ConnectError) -> Self {
        Self::Call(err)
    }
}

impl From<EndpointError> for ClientError {
    fn from(err: EndpointError) -> Self {
        Self::Endpoint(err)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ENDPOINT_ENV, EndpointError, RUNTIME_DIR_ENV, TEMP_DIR_ENV, default_endpoint,
        resolve_endpoint,
    };
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// Serialises the environment in this test binary: `std::env` is process
    /// wide and the harness runs the tests on threads.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Holds the endpoint-rule variables for the duration of a test and
    /// restores every previous value on drop, so a panic cannot leak one into
    /// another test. The lock it carries serialises every environment test in
    /// this binary.
    struct EndpointEnv {
        previous: Vec<(&'static str, Option<OsString>)>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EndpointEnv {
        fn new() -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            let previous = [ENDPOINT_ENV, RUNTIME_DIR_ENV, TEMP_DIR_ENV]
                .into_iter()
                .map(|name| (name, std::env::var_os(name)))
                .collect();
            let this = Self {
                previous,
                _lock: lock,
            };
            this.unset(ENDPOINT_ENV);
            this.unset(RUNTIME_DIR_ENV);
            this.unset(TEMP_DIR_ENV);
            this
        }

        /// Edition 2024 makes mutating the process environment unsafe because
        /// it is not thread-safe. `_lock` serialises every environment test in
        /// this binary, so nothing races these writes.
        fn set(&self, name: &'static str, value: &str) {
            // SAFETY: the guard holds `ENV_LOCK` for its whole lifetime.
            unsafe { std::env::set_var(name, value) };
        }

        fn unset(&self, name: &'static str) {
            // SAFETY: the guard holds `ENV_LOCK` for its whole lifetime.
            unsafe { std::env::remove_var(name) };
        }
    }

    impl Drop for EndpointEnv {
        fn drop(&mut self) {
            for (name, value) in &self.previous {
                // SAFETY: the guard holds `ENV_LOCK` until it is dropped.
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
    }

    /// The runtime directory wins when it is set, and the socket sits under a
    /// `metis` directory the daemon can own.
    #[test]
    fn the_runtime_directory_is_the_first_private_base() {
        let env = EndpointEnv::new();
        env.set(RUNTIME_DIR_ENV, "/run/user/1000");
        env.set(TEMP_DIR_ENV, "/var/tmp");
        assert_eq!(
            default_endpoint().expect("the runtime directory is a private base"),
            PathBuf::from("/run/user/1000/metis/daemon.sock")
        );
        drop(env);
    }

    /// An empty runtime directory is unset, and the next private base is used.
    #[test]
    fn the_temporary_directory_is_the_second_private_base() {
        let env = EndpointEnv::new();
        env.set(RUNTIME_DIR_ENV, "");
        env.set(TEMP_DIR_ENV, "/var/tmp");
        assert_eq!(
            default_endpoint().expect("the temporary directory is a private base"),
            PathBuf::from("/var/tmp/metis/daemon.sock")
        );
        drop(env);
    }

    /// With neither variable set there is no private base, and the rule says so
    /// instead of falling back to a shared path the ACL cannot carry.
    #[test]
    fn no_private_base_is_a_typed_error() {
        let env = EndpointEnv::new();
        let error = default_endpoint().expect_err("there is no private base to use");
        assert_eq!(error, EndpointError::NoPrivateBase);
        assert!(
            error.to_string().contains(RUNTIME_DIR_ENV),
            "the error names the first variable: {error}"
        );
        assert!(
            error.to_string().contains(TEMP_DIR_ENV),
            "the error names the second variable: {error}"
        );
        drop(env);
    }

    /// A refusal that lists supported versions becomes the typed error. The
    /// list comes from the response header, never from the message text: the
    /// version the message mentions is not the one the error reports.
    #[test]
    fn a_version_refusal_is_a_typed_error() {
        use super::{ClientError, version_refusal};
        use api_contracts::identity::{PROTOCOL_VERSION, SUPPORTED_VERSIONS_HEADER};
        use connectrpc::{ConnectError, ErrorCode};
        use http::{HeaderMap, HeaderValue};

        let mut headers = HeaderMap::new();
        headers.insert(SUPPORTED_VERSIONS_HEADER, HeaderValue::from_static("1, 2"));
        let refusal = ConnectError::new(
            ErrorCode::InvalidArgument,
            "protocol version 9 is not supported",
        )
        .with_headers(headers);
        match version_refusal(&refusal) {
            Some(ClientError::ProtocolVersion { found, supported }) => {
                assert_eq!(found, PROTOCOL_VERSION, "the version this client declared");
                assert_eq!(supported, vec![1, 2]);
            }
            other => panic!("expected the typed version error, got {other:?}"),
        }

        // The message names the supported versions too, for a person reading it.
        let display = ClientError::ProtocolVersion {
            found: PROTOCOL_VERSION,
            supported: vec![1, 2],
        }
        .to_string();
        assert!(
            display.contains("1, 2"),
            "the message lists them: {display}"
        );

        // A failure with no such header stays the transport error it was.
        let other = ConnectError::new(ErrorCode::Internal, "boom");
        assert!(
            version_refusal(&other).is_none(),
            "an ordinary failure is not a version refusal"
        );
    }

    /// The explicit override wins over both bases, so an operator or a test
    /// that names an endpoint is never second-guessed.
    #[test]
    fn the_explicit_override_wins() {
        let env = EndpointEnv::new();
        env.set(RUNTIME_DIR_ENV, "/run/user/1000");
        env.set(TEMP_DIR_ENV, "/var/tmp");
        env.set(ENDPOINT_ENV, "/tmp/a-test-endpoint.sock");
        assert_eq!(
            resolve_endpoint().expect("an explicit endpoint needs no private base"),
            PathBuf::from("/tmp/a-test-endpoint.sock")
        );

        env.set(ENDPOINT_ENV, "");
        assert_eq!(
            resolve_endpoint().expect("an empty override is unset"),
            PathBuf::from("/run/user/1000/metis/daemon.sock")
        );
        drop(env);
    }
}
