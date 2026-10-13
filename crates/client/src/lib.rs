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
//! A surface pairs once: [`Client::pair`] presents the daemon's preshared key
//! and keeps the token it is answered with, and every later call carries that
//! token as `Authorization` metadata. The credential is never named by a caller
//! and never appears in a URL. A daemon that answers with a credential refusal
//! is reported as a typed error, so a caller can tell "pair me" from "your
//! access is gone".
//!
//! [`Client::stream_events`] opens the daemon's event stream and hands back an
//! [`EventStream`] to drain. The events it yields are the daemon's own event
//! type, so a caller never sees a wire message, a buffa view, or the transport.
//! Every stream declares a request identifier of its own, which
//! [`Client::cancel`] stops the stream's work by; a stream the daemon drops or
//! cancels is reported as the typed [`ClientError::StreamDropped`] or
//! [`ClientError::StreamCancelled`], never as a clean end, so a gap is never
//! mistaken for current state.
//!
//! `skeleton` holds the test-support service the walking-skeleton test and
//! example drive, and the start paths for it. The daemon effort replaces it;
//! the transport and the wrapper stay.

pub mod credential;
pub mod skeleton;

mod stream;

pub use stream::EventStream;

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use api_contracts::auth::{self, CredentialRefusal, PairRefusal};
use api_contracts::cancel::CancelOutcome;
use api_contracts::connect::metis::v1::DaemonServiceClient;
use api_contracts::events;
use api_contracts::identity;
use api_contracts::proto::metis::v1::{
    CancelRequest, EchoRequest, PairRequest, StreamEventsRequest,
};
use api_contracts::request::{self, RequestId};
use api_contracts::transport;
use connectrpc::ConnectError;
use connectrpc::client::{CallOptions, ClientConfig, SharedHttp2Connection};
use http::HeaderMap;

use crate::credential::Backend;

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
/// private, so the transport stays swappable without moving a caller. It also
/// owns the declared [`Surface`] and the credential that surface keeps, so a
/// caller names neither.
pub struct Client {
    inner: DaemonServiceClient<SharedHttp2Connection>,
    /// The surface this client declares. The credential it carries belongs to
    /// this surface, so pairing and the stored credential agree on it.
    surface: Surface,
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
        let surface = Surface::Unspecified;
        Self::assembled(
            transport,
            ClientConfig::new(transport::local_authority()),
            surface,
        )
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
        let surface = Surface::Unspecified;
        Self::assembled(
            transport,
            ClientConfig::new(transport::remote_authority(addr)),
            surface,
        )
    }

    /// Build a client around a transport, declaring `surface` and carrying
    /// whatever credential that surface has stored.
    fn assembled(
        transport: SharedHttp2Connection,
        base: ClientConfig,
        surface: Surface,
    ) -> Result<Self, ClientError> {
        let config = configured(base, surface)?;
        Ok(Self {
            inner: DaemonServiceClient::new(transport, config),
            surface,
        })
    }

    /// Declare which surface this client speaks for, and carry that surface's
    /// stored credential.
    ///
    /// The surface is request metadata the daemon records, and it is also the
    /// name the credential is filed under, so this is where a client that has
    /// paired before picks its token up. The declaration is rebuilt rather than
    /// appended, so the surface header is replaced instead of duplicated: the
    /// generated client exposes its configuration as a mutable reference, and
    /// `connectrpc` 0.9.1 offers no mutable default-header accessor.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Credential`] when the credential store cannot be
    /// read, which includes the platform keychain refusing to answer.
    pub fn with_surface(self, surface: Surface) -> Result<Self, ClientError> {
        let base = self.inner.config().clone();
        let config = configured(base, surface)?;
        let mut self_ = self;
        *self_.inner.config_mut() = config;
        self_.surface = surface;
        Ok(self_)
    }

    /// The surface this client declares.
    #[must_use]
    pub fn surface(&self) -> Surface {
        self.surface
    }

    /// Pair this surface: present `preshared_key`, keep the token the daemon
    /// answers with, and report where it was kept.
    ///
    /// The token is stored for the declared surface and attached to this client
    /// as well, so the calls that follow need no credential named at the call
    /// site. Pairing is the one call exempt from the credential check, so this
    /// works before the surface has a credential.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Credential`] when the token cannot be kept,
    /// [`ClientError::ProtocolVersion`] when the daemon refuses this client's
    /// declared version, [`ClientError::Pairing`] when the daemon refuses the
    /// pairing call itself (a wrong preshared key, a surface that is already
    /// paired, or a daemon with no key configured), and [`ClientError::Call`]
    /// when the call fails on the transport or the daemon answers with another
    /// protocol error.
    pub async fn pair(&mut self, preshared_key: &str) -> Result<Backend, ClientError> {
        let request = PairRequest {
            preshared_key: preshared_key.to_owned(),
            ..Default::default()
        };
        let response = self.inner.pair(request).await.map_err(refusal)?;
        let token = response.into_owned().token;
        let backend =
            credential::store(self.surface.as_str(), &token).map_err(ClientError::Credential)?;
        self.attach(&token);
        Ok(backend)
    }

    /// Replace this client's configuration with the declaration plus `token`,
    /// so every later call carries it.
    fn attach(&mut self, token: &str) {
        let base = self.inner.config().clone();
        *self.inner.config_mut() = with_credential(declare(base, self.surface), Some(token));
    }

    /// Perform the echo RPC and return the daemon's echoed message.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::CredentialMissing`] when the daemon needs a
    /// credential this client does not have,
    /// [`ClientError::CredentialRejected`] when it refuses the one this client
    /// presented, [`ClientError::ProtocolVersion`] when it refuses this client's
    /// declared protocol version, and [`ClientError::Call`] when the call fails
    /// on the transport or the daemon answers with another protocol error.
    pub async fn echo(&self, message: &str) -> Result<String, ClientError> {
        let request = EchoRequest {
            message: message.to_owned(),
            ..Default::default()
        };
        match self.inner.echo(request).await {
            Ok(response) => Ok(response.into_owned().message),
            Err(err) => Err(refusal(err)),
        }
    }

    /// Open the daemon's event stream for `channels`, after `last_event_id`.
    ///
    /// A stream opened without a cursor starts with the snapshot of the selected
    /// channels (one empty snapshot message when they hold nothing), what follows
    /// are the events published while the stream is open, and an idle stretch is
    /// filled with heartbeats.
    ///
    /// `last_event_id` is the cursor: `0` means no cursor, and the identifiers it
    /// names start at `1`. A non-zero cursor resumes after the highest `EVENT`
    /// identifier the caller has processed: the ring replays what it still holds
    /// after that identifier, in order, and then carries the live events. A
    /// resumed stream carries no snapshot, because the caller already holds the
    /// state a snapshot would restate.
    ///
    /// Every stream declares a request identifier of its own, which the daemon
    /// registers the stream's work under, so [`Client::cancel`] can stop it. The
    /// identifier is on the returned handle as [`EventStream::request_id`]; a
    /// caller never invents one.
    ///
    /// A cursor the ring can no longer serve earns [`ClientError::CursorExpired`]
    /// from this call. The answer is a resync, which is the cursorless form of
    /// the same call: `stream_events(channels, 0)` opens a fresh stream that
    /// begins with the snapshot. Nothing is retried on the caller's behalf.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::CursorExpired`] when the daemon can no longer
    /// serve the cursor, and the same typed errors [`Client::echo`] returns: the
    /// credential check runs when the stream is established, so an unauthorized
    /// caller is refused before any event is served.
    pub async fn stream_events(
        &self,
        channels: &[&str],
        last_event_id: u64,
    ) -> Result<EventStream, ClientError> {
        let request_id = request_id();
        let request = StreamEventsRequest {
            channels: channels
                .iter()
                .map(|channel| (*channel).to_owned())
                .collect(),
            last_event_id,
            ..Default::default()
        };
        // The identifier travels as request metadata on this call and no other,
        // so two streams opened by the same client never share one.
        let options = CallOptions::default()
            .try_with_header(request::REQUEST_ID_HEADER, request_id.as_str())?;
        let stream = self
            .inner
            .stream_events_with_options(request, options)
            .await
            .map_err(refusal)?;
        // A Connect streaming response carries its status inside the stream, so
        // a refusal arrives as a response the stream was established with and an
        // error in the stream itself. Reading the refusal headers here is what
        // makes a stream opened without a credential fail at the call that
        // opened it, exactly as a unary call reports the same refusal, instead
        // of failing at the first event the caller never gets.
        if let Some(refusal) = typed_refusal(stream.headers()) {
            return Err(refusal);
        }
        Ok(EventStream::new(stream, request_id))
    }

    /// Stop the in-flight work the daemon registered under `request_id`, which
    /// is the identifier a stream handle reports as [`EventStream::request_id`].
    ///
    /// The call is idempotent: it reports whether anything was in flight, and
    /// work that finished, work already cancelled, and work never seen are
    /// answered the same way. Work is keyed by the surface this client declares,
    /// so this stops only work this client's own surface started.
    ///
    /// A cancelled stream stops: its next [`EventStream::message`] reports
    /// [`ClientError::StreamCancelled`] rather than an event, so the gap a
    /// cancellation leaves is never presented as current state.
    ///
    /// # Errors
    ///
    /// Returns the same typed errors [`Client::echo`] returns, and
    /// [`ClientError::Call`] when the daemon answers with an outcome this build
    /// cannot act on.
    pub async fn cancel(&self, request_id: &RequestId) -> Result<CancelOutcome, ClientError> {
        let request = CancelRequest {
            request_id: request_id.as_str().to_owned(),
            ..Default::default()
        };
        let response = self.inner.cancel(request).await.map_err(refusal)?;
        response
            .into_owned()
            .outcome
            .as_known()
            .and_then(CancelOutcome::from_proto)
            .ok_or_else(|| {
                ClientError::Call(ConnectError::unknown(
                    "the daemon answered a cancel outcome this build cannot act on",
                ))
            })
    }
}

/// A fresh request identifier for one call.
///
/// Sixteen random bytes as lowercase hex: 32 printable bytes, distinct per call,
/// and bounded by the identifier rule, so two calls never share one and a
/// cancelled identifier names the work it was meant for. Fails loudly rather
/// than silently repeating: an identifier that collides would let one call
/// stop another.
fn request_id() -> RequestId {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).expect("the operating system's random source is available");
    let text = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    RequestId::parse(&text).expect("lowercase hex is a printable identifier")
}

/// The configuration a client with a declared surface carries: the declaration
/// every call makes, plus the credential stored for that surface when there is
/// one.
fn configured(base: ClientConfig, surface: Surface) -> Result<ClientConfig, ClientError> {
    let stored = credential::load(surface.as_str()).map_err(ClientError::Credential)?;
    Ok(with_credential(
        declare(base, surface),
        stored.as_ref().map(|stored| stored.token()),
    ))
}

/// The configuration holding the identity declaration each call carries as
/// request metadata, on the base configuration's authority.
///
/// The configuration the wrapper builds holds the authority and the
/// declaration alone, so rebuilding it from the base loses nothing.
fn declare(base: ClientConfig, surface: Surface) -> ClientConfig {
    ClientConfig::new(base.base_uri().clone())
        .with_default_header(
            identity::PROTOCOL_VERSION_HEADER,
            identity::PROTOCOL_VERSION.to_string(),
        )
        .with_default_header(identity::PROTOCOL_SURFACE_HEADER, surface.as_str())
        .with_default_header(identity::PROTOCOL_CLIENT_VERSION_HEADER, CLIENT_VERSION)
}

/// Add the credential as a default header, when there is one.
///
/// The credential travels as metadata on every call, including the event
/// stream, and never in a URL: the URL is built from the authority and the
/// method's procedure path alone.
fn with_credential(config: ClientConfig, token: Option<&str>) -> ClientConfig {
    match token {
        Some(token) => config.with_default_header(auth::AUTHORIZATION_HEADER, auth::bearer(token)),
        None => config,
    }
}

/// The typed error one failed call earns.
///
/// A refusal the daemon labelled is reported as the typed error it is, and
/// anything else stays the transport error. Every refusal is read from response
/// metadata, never from message text.
pub(crate) fn refusal(err: ConnectError) -> ClientError {
    // A Connect streaming failure carries the daemon's metadata in the
    // end-of-stream envelope rather than in the response headers the stream was
    // established with, so the trailer read is the second step of the chain and
    // the response headers stay first, where the cursor and credential refusals
    // arrive.
    if let Some(refusal) = typed_refusal(err.response_headers()) {
        return refusal;
    }
    if let Some(refusal) = typed_refusal(err.trailers()) {
        return refusal;
    }
    ClientError::Call(err)
}

/// The typed error a refusal's response metadata names, or `None` when it names
/// no refusal this build knows.
///
/// The event stream reports through this as well as a unary call: a Connect
/// streaming response carries its status inside the stream, so its refusal
/// metadata is on the response the stream was established with and in the
/// trailing metadata of a failure that happened after it.
pub(crate) fn typed_refusal(headers: &HeaderMap) -> Option<ClientError> {
    version_refusal(headers)
        .or_else(|| credential_refusal(headers))
        .or_else(|| pair_refusal(headers))
        .or_else(|| cursor_refusal(headers))
        .or_else(|| stream_refusal(headers))
}

/// The typed error for a credential refusal.
///
/// `None` means the failure is not a credential refusal, and the caller keeps
/// the transport error.
fn credential_refusal(headers: &HeaderMap) -> Option<ClientError> {
    let header = headers.get(auth::AUTH_ERROR_HEADER)?;
    let refusal = CredentialRefusal::parse(header.to_str().ok()?)?;
    Some(match refusal {
        CredentialRefusal::Missing => ClientError::CredentialMissing,
        CredentialRefusal::Invalid => ClientError::CredentialRejected {
            cause: CredentialCause::Invalid,
        },
        CredentialRefusal::Revoked => ClientError::CredentialRejected {
            cause: CredentialCause::Revoked,
        },
        // The vocabulary is non-exhaustive: a refusal this build does not know
        // is left as the transport error rather than guessed at.
        _ => return None,
    })
}

/// Why the daemon refused a presented credential.
///
/// The distinction is the one a caller acts on: an invalid credential means the
/// wrong one is being presented, and a revoked one means the surface has no
/// access left and has to pair again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialCause {
    /// The surface has a token, and this is not it.
    Invalid,
    /// The surface has no token: it was never paired, or its token was revoked.
    Revoked,
}

/// The typed error for a pairing refusal.
///
/// The refusal is read from the response header, never from the message text, so
/// a closed pairing, a wrong key and a repeated pairing are told apart by the
/// daemon's own vocabulary rather than by prose. `None` means the failure is not
/// a pairing refusal, and the caller keeps the transport error.
fn pair_refusal(headers: &HeaderMap) -> Option<ClientError> {
    let header = headers.get(auth::PAIR_ERROR_HEADER)?;
    let refusal = PairRefusal::parse(header.to_str().ok()?)?;
    Some(match refusal {
        PairRefusal::Disabled => ClientError::Pairing {
            cause: PairingCause::Disabled,
        },
        PairRefusal::WrongKey => ClientError::Pairing {
            cause: PairingCause::WrongKey,
        },
        PairRefusal::AlreadyPaired => ClientError::Pairing {
            cause: PairingCause::AlreadyPaired,
        },
        // The vocabulary is non-exhaustive: a refusal this build does not know
        // is left as the transport error rather than guessed at.
        _ => return None,
    })
}

/// The typed error for a cursor refusal.
///
/// The refusal is read from the response header, never from the message text, so
/// the caller learns that the ring cannot serve the cursor from the daemon's own
/// vocabulary. `None` means the failure is not a cursor refusal, and the caller
/// keeps the transport error. An expired cursor is answered with a cursorless
/// stream rather than a retry, so the caller is told rather than looped.
fn cursor_refusal(headers: &HeaderMap) -> Option<ClientError> {
    let header = headers.get(events::CURSOR_ERROR_HEADER)?;
    let refusal = events::CursorRefusal::parse(header.to_str().ok()?)?;
    Some(match refusal {
        events::CursorRefusal::Expired => ClientError::CursorExpired,
        // The vocabulary is non-exhaustive: a refusal this build does not know
        // is left as the transport error rather than guessed at.
        _ => return None,
    })
}

/// Why the daemon refused a pairing call.
///
/// The distinction is the one a caller acts on: a closed pairing is the
/// operator's configuration rather than the caller's mistake, the wrong key is
/// the operator's to correct, and a surface that already paired has to be
/// revoked before it can pair again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingCause {
    /// The daemon has no preshared key configured, so pairing is closed.
    Disabled,
    /// The preshared key presented is not the one the daemon was configured
    /// with.
    WrongKey,
    /// The surface already has a token.
    AlreadyPaired,
}

/// The typed error for a stream the daemon ended in the middle.
///
/// The refusal is read from the trailing metadata, never from the message text,
/// so the caller learns what happened from the daemon's own vocabulary. A stream
/// that ends this way carries no further event, and an error this build does not
/// know is left as the transport error rather than guessed at.
fn stream_refusal(headers: &HeaderMap) -> Option<ClientError> {
    let header = headers.get(events::STREAM_ERROR_HEADER)?;
    let refusal = events::StreamRefusal::parse(header.to_str().ok()?)?;
    Some(match refusal {
        events::StreamRefusal::SlowConsumer => ClientError::StreamDropped,
        events::StreamRefusal::Cancelled => ClientError::StreamCancelled,
        // The vocabulary is non-exhaustive: a refusal this build does not know
        // is left as the transport error rather than guessed at.
        _ => return None,
    })
}

/// The typed error for a refusal that lists supported protocol versions.
///
/// The list is read from the refusal's response header, never from its message
/// text, and `found` is the version this client declared. `None` means the
/// failure is not a version refusal, and the caller keeps the transport error.
fn version_refusal(headers: &HeaderMap) -> Option<ClientError> {
    let header = headers.get(identity::SUPPORTED_VERSIONS_HEADER)?;
    let supported = identity::parse_supported_versions(header.to_str().ok()?);
    if supported.is_empty() {
        return None;
    }
    Some(ClientError::ProtocolVersion {
        found: identity::PROTOCOL_VERSION,
        supported,
    })
}

/// The wrapper's typed error over the transport, protocol, and credential
/// failures.
///
/// The enum is non-exhaustive from outside the crate.
#[derive(Debug)]
#[non_exhaustive]
pub enum ClientError {
    /// The RPC did not complete: the transport failed, or the daemon answered
    /// with a protocol error.
    Call(ConnectError),
    /// The local endpoint could not be resolved.
    Endpoint(EndpointError),
    /// The daemon needs a credential this client does not have, so the surface
    /// has to pair.
    CredentialMissing,
    /// The daemon refused the credential this client presented.
    CredentialRejected {
        /// Why it was refused.
        cause: CredentialCause,
    },
    /// The credential store could not be read or written.
    Credential(credential::CredentialError),
    /// The daemon refused the pairing call this client made.
    Pairing {
        /// Why it was refused.
        cause: PairingCause,
    },
    /// The daemon refused the protocol version this client declared, and said
    /// which versions it supports.
    ProtocolVersion {
        /// The version this client declared.
        found: u32,
        /// The versions the daemon supports.
        supported: Vec<u32>,
    },
    /// The daemon can no longer serve the cursor this client presented: the
    /// ring's window moved past it, or the daemon restarted. The answer is a
    /// resync, which is [`Client::stream_events`] with a cursor of `0`: a fresh
    /// stream that begins with the snapshot.
    CursorExpired,
    /// The daemon dropped this stream because the client did not read it fast
    /// enough, so events were lost before they could be carried. The answer is
    /// the same resync an expired cursor earns, a fresh cursorless stream: the
    /// events that were dropped are gone, and the state the caller holds is
    /// stale rather than merely behind.
    StreamDropped,
    /// The request that owned this stream was cancelled, so the stream stopped.
    /// What it already published stays published; what had not happened yet did
    /// not.
    StreamCancelled,
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Call(err) => write!(f, "the call failed: {err}"),
            Self::Endpoint(err) => write!(f, "the local endpoint could not be resolved: {err}"),
            Self::CredentialMissing => write!(
                f,
                "the daemon needs a credential for this surface; pair it first"
            ),
            Self::CredentialRejected { cause } => match cause {
                CredentialCause::Invalid => {
                    write!(f, "the daemon refused this surface's credential as invalid")
                }
                CredentialCause::Revoked => write!(
                    f,
                    "the daemon has no token for this surface: it was never paired, or it was revoked"
                ),
            },
            Self::Credential(err) => write!(f, "the credential could not be kept: {err}"),
            Self::Pairing { cause } => match cause {
                PairingCause::Disabled => write!(
                    f,
                    "the daemon refused to pair this surface: it has no preshared key configured, so pairing is closed"
                ),
                PairingCause::WrongKey => write!(
                    f,
                    "the daemon refused to pair this surface: that is not its preshared key"
                ),
                PairingCause::AlreadyPaired => write!(
                    f,
                    "the daemon refused to pair this surface: it already has a token, so revoke it before pairing again"
                ),
            },
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
            Self::CursorExpired => write!(
                f,
                "the daemon can no longer resume from this cursor: open a cursorless stream to resync"
            ),
            Self::StreamDropped => write!(
                f,
                "the daemon dropped this stream because it could not keep up with it: events were lost, so open a cursorless stream to resync"
            ),
            Self::StreamCancelled => write!(
                f,
                "the request that owned this stream was cancelled, so the stream stopped"
            ),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Call(err) => Some(err),
            Self::Endpoint(err) => Some(err),
            Self::Credential(err) => Some(err),
            Self::ProtocolVersion { .. } | Self::CredentialMissing => None,
            Self::CredentialRejected { .. } => None,
            Self::Pairing { .. } => None,
            Self::CursorExpired | Self::StreamDropped | Self::StreamCancelled => None,
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

impl From<credential::CredentialError> for ClientError {
    fn from(err: credential::CredentialError) -> Self {
        Self::Credential(err)
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
        match version_refusal(refusal.response_headers()) {
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
            version_refusal(other.response_headers()).is_none(),
            "an ordinary failure is not a version refusal"
        );
    }

    /// An expired cursor is the typed error, read from the response header and
    /// never from message text; a value this build does not know, and a
    /// failure with no such header, stay the transport error.
    #[test]
    fn a_cursor_refusal_is_a_typed_error() {
        use super::{ClientError, cursor_refusal};
        use api_contracts::events::{CURSOR_ERROR_HEADER, CursorRefusal};
        use connectrpc::{ConnectError, ErrorCode};
        use http::{HeaderMap, HeaderValue};

        assert_eq!(CursorRefusal::Expired.to_string(), "expired");

        let mut headers = HeaderMap::new();
        headers.insert(CURSOR_ERROR_HEADER, HeaderValue::from_static("expired"));
        let refusal = ConnectError::new(ErrorCode::FailedPrecondition, "the ring moved on")
            .with_headers(headers);
        assert!(
            matches!(
                cursor_refusal(refusal.response_headers()),
                Some(ClientError::CursorExpired)
            ),
            "the header is the refusal, not the message text"
        );

        // A refusal this build does not know is not guessed at.
        let mut unknown = HeaderMap::new();
        unknown.insert(
            CURSOR_ERROR_HEADER,
            HeaderValue::from_static("something-else"),
        );
        assert!(cursor_refusal(&unknown).is_none());

        // An ordinary failure is not a cursor refusal.
        let other = ConnectError::new(ErrorCode::Internal, "boom");
        assert!(cursor_refusal(other.response_headers()).is_none());
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
