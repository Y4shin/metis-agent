//! The skeleton daemon: test support for the wire path.
//!
//! This is deliberately not product API. It is the minimal `DaemonService`
//! implementation the transport tests and the `echo` example drive, so the
//! served path is proven end to end before the daemon effort replaces it with
//! the real service. It echoes the request and stamps `received_at` with an
//! RFC 3339 UTC time, matching the contract. `serve_uds` and `serve_remote` are
//! the start paths its tests use: the first binds a Unix socket, the second
//! binds the TCP address `METIS_TCP_ADDR` names, if any.
//!
//! `router_recording` is the same service with the identity declaration turned
//! on: it records what each call declared and refuses a declaration it cannot
//! act on. `router` stays declaration-free, because a caller that declares
//! nothing is served by the walking skeleton.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use api_contracts::connect::metis::v1::{DaemonService, DaemonServiceExt};
use api_contracts::identity::{self, ClientIdentity, IdentityError};
use api_contracts::proto::metis::v1::{EchoRequest, EchoResponse, PairRequest, PairResponse};
use api_contracts::transport::{self, ConfigError};
use connectrpc::{ConnectError, ErrorCode, RequestContext, Response, Router, ServiceRequest};

/// The skeleton service: it echoes whatever it receives.
pub struct SkeletonDaemon;

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
}

/// The router the skeleton registers on. The transport loop serves this.
#[must_use]
pub fn router() -> Router {
    Arc::new(SkeletonDaemon).register(Router::new())
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
/// each call declares and refuses a declaration it will not act on.
struct RecordingDaemon {
    recorder: IdentityRecorder,
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
    })
    .register(Router::new())
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

/// Serve the skeleton on a Unix domain socket.
///
/// # Errors
///
/// Returns the bind or accept error from the transport front-end.
#[cfg(unix)]
pub async fn serve_uds(path: &std::path::Path) -> std::io::Result<()> {
    api_contracts::transport::serve_uds(path, router()).await
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

/// The receive time as an RFC 3339 UTC stamp, without a date dependency.
fn received_at() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since_epoch| since_epoch.as_secs())
        .unwrap_or(0);
    format_rfc3339(seconds)
}

/// Format Unix seconds as `YYYY-MM-DDThh:mm:ssZ`.
///
/// The date conversion is Howard Hinnant's `civil_from_days`, the standard
/// days-to-calendar algorithm. The skeleton only needs a valid stamp, so no
/// leap-second or timezone handling is involved: the value is always UTC.
fn format_rfc3339(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3600;
    let minute = (second_of_day % 3600) / 60;
    let second = second_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Days since 1970-01-01 to a proleptic Gregorian `(year, month, day)`.
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153; // [0, 11]
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1; // [1, 31]
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    }; // [1, 12]
    (year + i64::from(month <= 2), month, day)
}
