//! The daemon's own service: pairing and the served echo.
//!
//! One value carries both what a listener serves and the policy it serves it
//! under. `connectrpc` attaches interceptors to a
//! [`ConnectRpcService`](connectrpc::ConnectRpcService) rather than to a
//! `Router`, so the authorization policy is welded to the service here: a
//! listener cannot serve this service without the interceptor that enforces the
//! policy it was built with.

use std::sync::Arc;
use std::time::Duration;

use api_contracts::cancel::Registry;
use api_contracts::connect::metis::v1::{DaemonService, DaemonServiceExt};
use api_contracts::events::{self, CursorError, Ring};
use api_contracts::identity::{self, IdentityError};
use api_contracts::proto::metis::v1::{
    CancelRequest, CancelResponse, EchoRequest, EchoResponse, Event, PairRequest, PairResponse,
    StreamEventsRequest,
};
use api_contracts::proxy::{self, TrustedProxies, WarnOnce};
use api_contracts::request::{self, EchoRequestId, RequestIdError};
use api_contracts::transport::ConfigError;
use connectrpc::{
    ConnectError, ConnectRpcService, ErrorCode, RequestContext, Response, Router, ServiceRequest,
    ServiceResult, ServiceStream,
};

use crate::auth::{self, AuthInterceptor, AuthPolicy, PairError};
use crate::secret::Secret;
use crate::state::Store;

/// How long a stream may carry nothing before it carries a heartbeat.
///
/// A stream is configured with the interval rather than the interval being a
/// constant of the serving loop, so a test sets a short one instead of waiting
/// fifteen seconds. Idle detection is not implemented: the heartbeat is a pace,
/// not a probe.
pub const HEARTBEAT_DEFAULT: Duration = Duration::from_secs(15);

/// How many events the daemon's ring holds before the oldest is dropped.
///
/// The window is what a reconnect can resume from; the map's resync rule is
/// that an expired cursor means a fresh stream, which is why the number is a
/// capacity rather than a retention time.
const EVENT_CAPACITY: usize = 1024;

/// The daemon's service: the store it pairs against, the preshared key that
/// gates pairing, and the events it streams. The key is optional, because
/// pairing is: a daemon with no key serves every paired surface and refuses the
/// pairing call itself.
pub struct Service {
    store: Store,
    preshared: Option<Secret>,
    /// The daemon-wide events this service streams. One ring per service, which
    /// is one ring per daemon: identifiers compare across every channel.
    ring: Ring,
    /// The work this service has in flight, keyed by the surface that started
    /// it and the request identifier it declared. One registry per service, and
    /// the handle is public so the daemon effort registers its own work here.
    cancellations: Registry,
    /// The heartbeat interval of every stream this service opens.
    heartbeat: Duration,
    /// The latch that keeps a per-call plaintext warning from repeating, one per
    /// service: the warning is about this daemon's exposure, so it belongs to
    /// the daemon rather than to a connection.
    warnings: WarnOnce,
}

impl Service {
    /// The service over `store`, gated by `preshared` when it is configured and
    /// heartbeating every `heartbeat`.
    #[must_use]
    pub fn new(store: Store, preshared: Option<Secret>, heartbeat: Duration) -> Self {
        Self {
            store,
            preshared,
            ring: Ring::new(EVENT_CAPACITY),
            cancellations: Registry::new(),
            heartbeat,
            warnings: WarnOnce::new(),
        }
    }

    /// The events this service streams.
    ///
    /// The daemon publishes its real events into this ring, and it is why the
    /// handle is public: a caller that holds the service needs the same handle
    /// to give a stream something to carry.
    #[must_use]
    pub fn ring(&self) -> Ring {
        self.ring.clone()
    }

    /// The work this service has in flight.
    ///
    /// Named after [`Service::ring`] for the same reason: a caller that holds
    /// the service needs the same handle the handlers register into, both to
    /// read what is in flight and to register work of its own.
    #[must_use]
    pub fn cancellations(&self) -> Registry {
        self.cancellations.clone()
    }

    /// The service a listener serves, with the policy it enforces.
    ///
    /// `connectrpc` builds the interceptor chain on the service, so this returns
    /// the service rather than the bare router: a caller mounts
    /// [`AuthPolicy::LocalAcl`] on the local socket, where the operating system
    /// has already authenticated the caller, and [`AuthPolicy::TokenRequired`]
    /// on the remote listener, where nothing about the connection says who is
    /// calling.
    #[must_use]
    pub fn mount(self: Arc<Self>, policy: AuthPolicy) -> ConnectRpcService<Router> {
        let interceptor = AuthInterceptor::new(self.store.clone(), policy);
        let router = self.register(Router::new());
        // The credential check is registered first, so it is the outer layer and
        // a refused call never reaches the echo. The echo is registered for the
        // whole service rather than per handler, so the daemon and the client's
        // in-process service answer a declared identifier identically.
        ConnectRpcService::new(router)
            .with_interceptor(interceptor)
            .with_interceptor(EchoRequestId)
    }
}

// The generated trait uses an explicit RPITIT signature; an `async fn` impl
// refines it, which the compiler flags with `refining_impl_trait`.
#[allow(refining_impl_trait)]
impl DaemonService for Service {
    /// Echo a message back with the time it arrived, which is the contract's
    /// example call and the whole exposed surface today.
    async fn echo(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, EchoRequest>,
    ) -> Result<connectrpc::Response<EchoResponse>, ConnectError> {
        connectrpc::Response::ok(EchoResponse {
            message: request.message.to_owned(),
            received_at: api_contracts::time::rfc3339_now(),
            ..Default::default()
        })
    }

    /// Pair the declaring surface: check the preshared key, mint the surface's
    /// token, and answer with it once.
    async fn pair(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PairRequest>,
    ) -> Result<connectrpc::Response<PairResponse>, ConnectError> {
        // The leg's exposure is a property of the connection rather than of the
        // request, so it is judged before the call is refused for any other
        // reason: a plaintext leg that carried the preshared key is what the
        // operator is told about, once per process. A named proxy that says it
        // terminated TLS is believed, because the transport has no way to see
        // the leg in front of it and the proxy rule is the only account of it.
        let trusted = TrustedProxies::resolve().map_err(refuse_trusted_proxies)?;
        let peer = ctx.peer_addr().map(|addr| addr.ip());
        let scheme = proxy::client_scheme(peer, &trusted, ctx.headers());
        if let Some(warning) = proxy::secret_warning(scheme, peer) {
            self.warnings.warn(&warning);
        }

        let declared = identity::from_headers(ctx.headers()).map_err(refuse_identity)?;
        // A daemon with no configured key has nothing to compare a presented key
        // with, so pairing is closed rather than open to any key. The rest of the
        // surface still serves: closing pairing is not revocation.
        let Some(preshared) = self.preshared.as_ref() else {
            return Err(refuse_pair(PairError::PairingDisabled));
        };
        // The body is copied out before the await: the request borrows the
        // connection's buffer, and the pairing work outlives that borrow's
        // usefulness rather than its validity.
        let presented = request.preshared_key.to_owned();
        let token = auth::pair(&self.store, preshared, declared.surface(), &presented)
            .await
            .map_err(refuse_pair)?;
        connectrpc::Response::ok(PairResponse {
            token: token.as_str().to_owned(),
            ..Default::default()
        })
    }

    /// Stream the daemon's events. A stream opened without a cursor begins with
    /// the snapshot of the selected channels (one empty snapshot message when
    /// they hold nothing); a stream opened with a cursor begins with the events
    /// that follow it, in identifier order, and carries no snapshot. Either way
    /// the live events follow, with heartbeats as ordinary messages.
    ///
    /// A cursor the ring can no longer serve is refused before the stream opens,
    /// through the `metis-cursor-error` header: the caller's answer is a
    /// cursorless stream, which is the resync.
    ///
    /// A request that declared an identifier is registered as work in flight for
    /// as long as the stream runs, so a `Cancel` naming that identifier stops it.
    /// A caller that declared none is served exactly as before and is not
    /// cancellable. A client that goes away leaves the stream, which drops the
    /// registration: a disconnect cancels nothing.
    ///
    /// The declaration is checked here because the recording is per handler: a
    /// streaming handler that skipped the call would leave a stream unrecorded
    /// rather than refused.
    async fn stream_events(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StreamEventsRequest>,
    ) -> ServiceResult<ServiceStream<Event>> {
        let declared = identity::from_headers(ctx.headers()).map_err(refuse_identity)?;
        let id = request::RequestId::from_headers(ctx.headers()).map_err(refuse_request_id)?;
        // The channels are borrowed views of the request body, which the stream
        // outlives, so they are collected into owned borrows on this frame
        // before the stream is built.
        let channels = request.channels.iter().copied().collect::<Vec<_>>();
        let stream = self
            .ring
            .stream(&channels, request.last_event_id, self.heartbeat)
            .map_err(refuse_cursor)?;
        // Registered after the cursor is served, so a refused stream leaves
        // nothing in flight. The guard travels inside the stream, so the work is
        // in flight exactly as long as the stream is served.
        let guard = id.map(|id| self.cancellations.register(declared.surface(), id));
        Ok(Response::stream(events::cancellable(stream, guard)))
    }

    /// Stop the in-flight work the declaring surface started under the
    /// identifier in the request.
    ///
    /// The call is idempotent: it answers whether anything was in flight, and
    /// work that finished, work already cancelled, and work never seen read the
    /// same, so a repeated call is answered rather than refused and no surface
    /// can learn whether another surface's identifier exists.
    async fn cancel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CancelRequest>,
    ) -> Result<connectrpc::Response<CancelResponse>, ConnectError> {
        let declared = identity::from_headers(ctx.headers()).map_err(refuse_identity)?;
        let id = request::RequestId::parse(request.request_id).map_err(refuse_request_id)?;
        let outcome = self.cancellations.cancel(declared.surface(), &id);
        connectrpc::Response::ok(CancelResponse {
            outcome: outcome.to_proto().into(),
            ..Default::default()
        })
    }
}

/// The service a listener serves, with the policy it enforces.
///
/// A caller passes the preshared key its configuration resolved, which is `None`
/// when no key is configured: pairing is optional, and a daemon without one still
/// serves its paired surfaces. The stream heartbeats on [`HEARTBEAT_DEFAULT`];
/// a caller that needs another interval builds the [`Service`] and mounts it
/// itself.
#[must_use]
pub fn service(
    store: Store,
    preshared: Option<Secret>,
    policy: AuthPolicy,
) -> ConnectRpcService<Router> {
    Arc::new(Service::new(store, preshared, HEARTBEAT_DEFAULT)).mount(policy)
}

/// Refuse a declaration this daemon cannot act on.
///
/// Pairing is exempt from the interceptor, and the stream's handler makes the
/// check itself because the recording is per handler, so both handlers produce
/// the same typed refusal and the same header the handlers behind the
/// interceptor produce.
fn refuse_identity(error: IdentityError) -> ConnectError {
    let headers = identity::refusal_headers(&error);
    ConnectError::new(ErrorCode::InvalidArgument, error.to_string()).with_headers(headers)
}

/// Refuse a cursor the ring can no longer serve.
///
/// The cursor's answer is a fresh, cursorless stream rather than a gap or a
/// retry, so the refusal is raised before the stream opens and its kind travels
/// as a response header instead of as message text.
fn refuse_cursor(error: CursorError) -> ConnectError {
    let headers = events::cursor_refusal_headers(&error);
    ConnectError::new(ErrorCode::FailedPrecondition, error.to_string()).with_headers(headers)
}

/// Refuse a request identifier this daemon cannot act on.
///
/// The refusal carries no header of its own, unlike a cursor or a credential:
/// there is no answer a client can act on differently, because an identifier a
/// daemon cannot read names no work either way. It is refused rather than
/// answered as already finished, because a caller that believes its work is
/// cancellable has lost the guarantee it asked for.
fn refuse_request_id(error: RequestIdError) -> ConnectError {
    ConnectError::new(ErrorCode::InvalidArgument, error.to_string())
}

/// Refuse a pairing call this daemon cannot judge the exposure of.
///
/// The trusted-proxy set is read where the proxy rule is applied, so a value
/// this build cannot read is an internal failure rather than an exposure
/// decision made on configuration nobody could parse: a daemon that cannot say
/// whether the key travelled in plaintext must not answer as if it could.
fn refuse_trusted_proxies(error: ConfigError) -> ConnectError {
    ConnectError::new(ErrorCode::Internal, error.to_string())
}

/// Map a pairing refusal onto the wire.
///
/// The refusal's meaning travels as [`auth::PAIR_ERROR_HEADER`], so the client
/// maps it to a typed error instead of reading the message text. A refusal
/// without a wire value is an internal failure, and keeps the status code it
/// was given.
fn refuse_pair(error: PairError) -> ConnectError {
    let headers = auth::pair_refusal_headers(&error);
    let code = match &error {
        PairError::PairingDisabled => ErrorCode::FailedPrecondition,
        PairError::WrongPresharedKey => ErrorCode::PermissionDenied,
        PairError::AlreadyPaired => ErrorCode::AlreadyExists,
        PairError::Store(_) => ErrorCode::Internal,
    };
    let refusal = ConnectError::new(code, error.to_string());
    match headers {
        Some(headers) => refusal.with_headers(headers),
        None => refusal,
    }
}
