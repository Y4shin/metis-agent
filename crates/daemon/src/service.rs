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

use api_contracts::connect::metis::v1::{DaemonService, DaemonServiceExt};
use api_contracts::events::Ring;
use api_contracts::identity::{self, IdentityError};
use api_contracts::proto::metis::v1::{
    EchoRequest, EchoResponse, Event, PairRequest, PairResponse, StreamEventsRequest,
};
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
    /// The heartbeat interval of every stream this service opens.
    heartbeat: Duration,
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
            heartbeat,
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
        ConnectRpcService::new(router).with_interceptor(interceptor)
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

    /// Stream the daemon's events: the snapshot for the selected channels first,
    /// then the events that follow, with heartbeats as ordinary messages.
    ///
    /// The declaration is checked here because the recording is per handler: a
    /// streaming handler that skipped the call would leave a stream unrecorded
    /// rather than refused.
    async fn stream_events(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StreamEventsRequest>,
    ) -> ServiceResult<ServiceStream<Event>> {
        identity::from_headers(ctx.headers()).map_err(refuse_identity)?;
        // The selection is copied out of the request before the stream is built:
        // the request borrows the connection's buffer, and the stream outlives
        // that borrow.
        // The channels are borrowed views of the request body, so they are
        // copied onto the iterator's own stack slot rather than converted.
        let channels = request.channels.iter().copied().collect::<Vec<_>>();
        Ok(Response::stream(self.ring.stream(
            &channels,
            request.last_event_id,
            self.heartbeat,
        )))
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
/// Pairing is exempt from the interceptor, so its handler makes the check
/// itself, with the same typed refusal and the same header the handlers behind
/// the interceptor produce.
fn refuse_identity(error: IdentityError) -> ConnectError {
    let headers = identity::refusal_headers(&error);
    ConnectError::new(ErrorCode::InvalidArgument, error.to_string()).with_headers(headers)
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
