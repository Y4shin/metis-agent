//! The skeleton daemon: test support for the wire path.
//!
//! This is deliberately not product API. It is the minimal `DaemonService`
//! implementation the transport tests and the walkthrough examples drive, so the
//! served path is proven end to end before the daemon effort replaces it with
//! the real service. It echoes the request, stamps `received_at` with an
//! RFC 3339 UTC time, and streams whatever its event ring holds. `serve_uds` and
//! `serve_remote` are the start paths its tests use: the first binds a Unix
//! socket, the second binds the TCP address `METIS_TCP_ADDR` names, if any.
//!
//! `router_recording` is the same service with the identity declaration turned
//! on: it records what each call declared and refuses a declaration it cannot
//! act on. `router` stays declaration-free, because a caller that declares
//! nothing is served by the walking skeleton.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use api_contracts::connect::metis::v1::{DaemonService, DaemonServiceExt};
use api_contracts::events::{CursorError, Ring};
use api_contracts::identity::{self, ClientIdentity, IdentityError};
use api_contracts::proto::metis::v1::{
    EchoRequest, EchoResponse, Event, PairRequest, PairResponse, StreamEventsRequest,
};
use api_contracts::transport::{self, ConfigError};
use connectrpc::{
    ConnectError, ErrorCode, RequestContext, Response, Router, ServiceRequest, ServiceResult,
    ServiceStream,
};

/// How long the skeleton's streams may carry nothing before they carry a
/// heartbeat. The daemon's own default is the same interval, and it is public
/// for the walkthrough example and for a test that serves its own ring.
pub const HEARTBEAT_DEFAULT: Duration = Duration::from_secs(15);

/// How many events the skeleton's ring holds before the oldest is dropped.
///
/// Smaller than the daemon's window on purpose: the skeleton is test support and
/// a smaller window makes a drop reachable from a test.
const EVENT_CAPACITY: usize = 64;

/// The skeleton service: it echoes whatever it receives, and it streams the
/// events its ring holds.
pub struct SkeletonDaemon {
    ring: Ring,
    heartbeat: Duration,
}

impl SkeletonDaemon {
    /// The skeleton over an empty ring, heartbeating on the daemon's own
    /// interval.
    #[must_use]
    pub fn new() -> Self {
        Self::with_ring(Ring::new(EVENT_CAPACITY), HEARTBEAT_DEFAULT)
    }

    /// The skeleton over `ring`, heartbeating every `heartbeat`.
    ///
    /// The caller keeps its own handle to `ring`, which is how a test or an
    /// example publishes the events a stream carries.
    #[must_use]
    pub fn with_ring(ring: Ring, heartbeat: Duration) -> Self {
        Self { ring, heartbeat }
    }
}

impl Default for SkeletonDaemon {
    fn default() -> Self {
        Self::new()
    }
}

// The generated trait uses an explicit RPITIT signature; an `async fn` impl
// refines it, which the compiler flags with `refining_impl_trait`.
#[allow(refining_impl_trait)]
impl DaemonService for SkeletonDaemon {
    async fn echo(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, EchoRequest>,
    ) -> Result<Response<EchoResponse>, ConnectError> {
        Response::ok(echo_body(request.message))
    }

    /// The skeleton does not pair: pairing belongs to the daemon's own service,
    /// which is the one this ticket lands. A call that reaches here is answered
    /// as unimplemented rather than with a token nothing issued.
    async fn pair(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, PairRequest>,
    ) -> Result<Response<PairResponse>, ConnectError> {
        Err(unpaired())
    }

    /// Stream the skeleton's ring: the snapshot for the selected channels, then
    /// the events that follow.
    async fn stream_events(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, StreamEventsRequest>,
    ) -> ServiceResult<ServiceStream<Event>> {
        // The channels are borrowed views of the request body, so they are
        // copied onto the iterator's own stack slot rather than converted.
        let channels = request.channels.iter().copied().collect::<Vec<_>>();
        events(&self.ring, self.heartbeat, &channels, request.last_event_id)
    }
}

/// The body of a stream over `ring`: the snapshot of `channels` first when the
/// stream carries no cursor, the events that follow a cursor when it does, and
/// the live events after either, with heartbeats on an idle stretch.
///
/// Both skeleton services serve their stream this way, so the skeleton and the
/// daemon agree on the stream's shape by sharing the ring's own assembly rather
/// than by repeating it, and both refuse a cursor the ring can no longer serve
/// through [`refuse_cursor`], exactly as the daemon's own handler does.
fn events(
    ring: &Ring,
    heartbeat: Duration,
    channels: &[&str],
    cursor: u64,
) -> ServiceResult<ServiceStream<Event>> {
    let stream = ring
        .stream(channels, cursor, heartbeat)
        .map_err(refuse_cursor)?;
    Ok(Response::stream(stream))
}

/// The router the skeleton registers on. The transport loop serves this.
#[must_use]
pub fn router() -> Router {
    router_with(Ring::new(EVENT_CAPACITY), HEARTBEAT_DEFAULT)
}

/// The router the skeleton registers on, streaming `ring`.
///
/// The caller keeps a handle to `ring` to publish the events the stream carries.
#[must_use]
pub fn router_with(ring: Ring, heartbeat: Duration) -> Router {
    Arc::new(SkeletonDaemon::with_ring(ring, heartbeat)).register(Router::new())
}

/// The identity declarations the daemon has recorded.
///
/// The handle is clonable, so the daemon keeps one and the caller that reads
/// what happened keeps another. This is the seam a test reads and the daemon
/// effort logs from; no logging framework is involved.
#[derive(Clone, Default)]
pub struct IdentityRecorder {
    last: Arc<Mutex<Option<ClientIdentity>>>,
}

impl IdentityRecorder {
    /// A recorder with nothing recorded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The most recently declared identity, or `None` when no call carrying a
    /// declaration has been served.
    #[must_use]
    pub fn last(&self) -> Option<ClientIdentity> {
        self.last
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, identity: ClientIdentity) {
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = Some(identity);
    }
}

/// The skeleton service with the declaration turned on: it records the identity
/// each call declares and refuses a declaration it will not act on. It streams
/// an empty ring, because the recording is the subject here and a stream's
/// contents are not.
struct RecordingDaemon {
    recorder: IdentityRecorder,
    ring: Ring,
    heartbeat: Duration,
}

// The generated trait uses an explicit RPITIT signature; an `async fn` impl
// refines it, which the compiler flags with `refining_impl_trait`.
#[allow(refining_impl_trait)]
impl DaemonService for RecordingDaemon {
    async fn echo(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, EchoRequest>,
    ) -> Result<Response<EchoResponse>, ConnectError> {
        let declared = identity::from_headers(ctx.headers()).map_err(refuse)?;
        self.recorder.record(declared);
        Response::ok(echo_body(request.message))
    }

    /// Pairing is the daemon's own service; the recording skeleton answers as
    /// unimplemented, so a test cannot read a token out of it.
    async fn pair(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, PairRequest>,
    ) -> Result<Response<PairResponse>, ConnectError> {
        Err(unpaired())
    }

    /// Stream the recording skeleton's ring. The declaration is checked and
    /// recorded here for the same reason it is checked for echo: a handler that
    /// skipped the call would leave a stream unrecorded rather than refused.
    async fn stream_events(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StreamEventsRequest>,
    ) -> ServiceResult<ServiceStream<Event>> {
        let declared = identity::from_headers(ctx.headers()).map_err(refuse)?;
        self.recorder.record(declared);
        // The channels are borrowed views of the request body, so they are
        // copied onto the iterator's own stack slot rather than converted.
        let channels = request.channels.iter().copied().collect::<Vec<_>>();
        events(&self.ring, self.heartbeat, &channels, request.last_event_id)
    }
}

/// The refusal both skeleton services answer a pairing call with.
fn unpaired() -> ConnectError {
    ConnectError::new(
        ErrorCode::Unimplemented,
        "the skeleton does not pair; the daemon's own service does",
    )
}

/// The router the recording service registers on. The transport loop serves it.
///
/// The recording is the daemon's own; nothing about the call is authenticated
/// by the declaration, and a refused call is not recorded.
#[must_use]
pub fn router_recording(recorder: &IdentityRecorder) -> Router {
    Arc::new(RecordingDaemon {
        recorder: recorder.clone(),
        ring: Ring::new(EVENT_CAPACITY),
        heartbeat: HEARTBEAT_DEFAULT,
    })
    .register(Router::new())
}

/// Refuse a cursor the ring can no longer serve, in the shape the daemon's own
/// handler uses, so the skeleton and the daemon refuse a cursor identically.
fn refuse_cursor(error: CursorError) -> ConnectError {
    ConnectError::new(ErrorCode::FailedPrecondition, error.to_string())
}

/// Refuse a declaration this daemon cannot act on.
///
/// The message is for a person reading a log; what the refusal means
/// mechanically is the response header `identity::refusal_headers` attaches: a
/// version refusal names the versions this daemon speaks, so the client gets a
/// typed error instead of parsing prose.
fn refuse(error: IdentityError) -> ConnectError {
    let headers = identity::refusal_headers(&error);
    ConnectError::new(ErrorCode::InvalidArgument, error.to_string()).with_headers(headers)
}

/// The echoed response both services return.
fn echo_body(message: &str) -> EchoResponse {
    EchoResponse {
        message: message.to_owned(),
        received_at: received_at(),
        ..Default::default()
    }
}

/// Serve the skeleton on a Unix domain socket, streaming an empty ring.
///
/// # Errors
///
/// Returns the bind or accept error from the transport front-end.
#[cfg(unix)]
pub async fn serve_uds(path: &std::path::Path) -> std::io::Result<()> {
    api_contracts::transport::serve_uds(path, router()).await
}

/// Serve the skeleton on a Unix domain socket, streaming `ring`.
///
/// This is the start path a caller drives a stream through: it keeps a handle to
/// `ring` and publishes into it. The interval is the one the caller names, so a
/// walkthrough does not wait out the daemon's fifteen seconds.
///
/// # Errors
///
/// Returns the bind or accept error from the transport front-end.
#[cfg(unix)]
pub async fn serve_uds_with(
    path: &std::path::Path,
    ring: Ring,
    heartbeat: Duration,
) -> std::io::Result<()> {
    api_contracts::transport::serve_uds(path, router_with(ring, heartbeat)).await
}

/// Wait, bounded, for a listener to create its socket file.
///
/// The listener binds before it accepts, so the file appearing means a connect
/// can be queued. Returns whether the file appeared within the bound.
#[cfg(unix)]
pub async fn wait_for_socket(path: &std::path::Path) -> bool {
    for _ in 0..100 {
        if path.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// Start the remote listener when `METIS_TCP_ADDR` names a bind address.
///
/// Resolution is `transport::resolve_remote_addr`: an absent or empty variable
/// returns `Ok(None)` having bound nothing, which is the whole of off by
/// default. A named address is bound before this function returns and accepted
/// on its own task, and the address the operating system assigned is returned,
/// so a caller connects to the real port instead of guessing one.
///
/// This is test support, like [`serve_uds`]: until the daemon effort wires the
/// listener into `main`, this is the start path the remote test drives.
///
/// # Errors
///
/// Returns [`ConfigError::InvalidRemoteAddr`] when the variable names something
/// that is not an `IP:port` literal, and [`ConfigError::RemoteBind`] when the
/// named address cannot be bound.
pub async fn serve_remote() -> Result<Option<SocketAddr>, ConfigError> {
    let Some(addr) = transport::resolve_remote_addr()? else {
        return Ok(None);
    };
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| ConfigError::RemoteBind { addr, source })?;
    let bound = listener
        .local_addr()
        .map_err(|source| ConfigError::RemoteBind { addr, source })?;
    tokio::spawn(async move {
        if let Err(err) = transport::serve_tcp_listener(listener, router()).await {
            eprintln!("remote listener error: {err}");
        }
    });
    Ok(Some(bound))
}

/// The receive time as the RFC 3339 UTC stamp the contract asks for.
fn received_at() -> String {
    api_contracts::time::rfc3339_now()
}
