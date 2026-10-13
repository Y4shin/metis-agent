//! The daemon's own service: pairing and the served echo.
//!
//! One value carries both what a listener serves and the policy it serves it
//! under. `connectrpc` attaches interceptors to a
//! [`ConnectRpcService`](connectrpc::ConnectRpcService) rather than to a
//! `Router`, so the authorization policy is welded to the service here: a
//! listener cannot serve this service without the interceptor that enforces the
//! policy it was built with.

use std::sync::Arc;

use api_contracts::connect::metis::v1::{DaemonService, DaemonServiceExt};
use api_contracts::identity::{self, IdentityError};
use api_contracts::proto::metis::v1::{EchoRequest, EchoResponse, PairRequest, PairResponse};
use connectrpc::{
    ConnectError, ConnectRpcService, ErrorCode, RequestContext, Router, ServiceRequest,
};

use crate::auth::{self, AuthInterceptor, AuthPolicy, PairError};
use crate::secret::Secret;
use crate::state::Store;

/// The daemon's service: the store it pairs against and the preshared key that
/// gates pairing. The key is optional, because pairing is: a daemon with no key
/// serves every paired surface and refuses the pairing call itself.
pub struct Service {
    store: Store,
    preshared: Option<Secret>,
}

impl Service {
    /// The service over `store`, gated by `preshared` when it is configured.
    #[must_use]
    pub fn new(store: Store, preshared: Option<Secret>) -> Self {
        Self { store, preshared }
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
}

/// The service a listener serves, with the policy it enforces.
///
/// A caller passes the preshared key its configuration resolved, which is `None`
/// when no key is configured: pairing is optional, and a daemon without one still
/// serves its paired surfaces.
///
/// `connectrpc` builds the interceptor chain on the service, so this returns
/// the service rather than the bare router: a caller mounts
/// [`AuthPolicy::LocalAcl`] on the local socket, where the operating system has
/// already authenticated the caller, and [`AuthPolicy::TokenRequired`] on the
/// remote listener, where nothing about the connection says who is calling.
#[must_use]
pub fn service(
    store: Store,
    preshared: Option<Secret>,
    policy: AuthPolicy,
) -> ConnectRpcService<Router> {
    let interceptor = AuthInterceptor::new(store.clone(), policy);
    let router = Arc::new(Service::new(store, preshared)).register(Router::new());
    ConnectRpcService::new(router).with_interceptor(interceptor)
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
