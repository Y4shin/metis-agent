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
/// gates pairing.
pub struct Service {
    store: Store,
    preshared: Secret,
}

impl Service {
    /// The service over `store`, gated by `preshared`.
    #[must_use]
    pub fn new(store: Store, preshared: Secret) -> Self {
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
        // The body is copied out before the await: the request borrows the
        // connection's buffer, and the pairing work outlives that borrow's
        // usefulness rather than its validity.
        let presented = request.preshared_key.to_owned();
        let token = auth::pair(&self.store, &self.preshared, declared.surface(), &presented)
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
/// `connectrpc` builds the interceptor chain on the service, so this returns
/// the service rather than the bare router: a caller mounts
/// [`AuthPolicy::LocalAcl`] on the local socket, where the operating system has
/// already authenticated the caller, and [`AuthPolicy::TokenRequired`] on the
/// remote listener, where nothing about the connection says who is calling.
#[must_use]
pub fn service(store: Store, preshared: Secret, policy: AuthPolicy) -> ConnectRpcService<Router> {
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
fn refuse_pair(error: PairError) -> ConnectError {
    let code = match &error {
        PairError::WrongPresharedKey => ErrorCode::PermissionDenied,
        PairError::AlreadyPaired => ErrorCode::AlreadyExists,
        PairError::Store(_) => ErrorCode::Internal,
    };
    ConnectError::new(code, error.to_string())
}
