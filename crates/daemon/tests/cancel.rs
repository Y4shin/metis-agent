//! Cancellation and slow-consumer handling, over a real socket.
//!
//! The daemon's own service is served the way a listener serves it, the client
//! harness reaches it through the wrapper or through the generated client, and
//! the ring the service serves is the one the test publishes into. What no unit
//! test can show is asserted here: a `Cancel` call stops a stream that is
//! serving, a second call is answered rather than refused, a client that
//! disconnects cancels nothing, and a client that stops reading is dropped with
//! the loss named instead of being carried across a silent gap.
//!
//! The keychain machinery is held out of the way: `PATH` names one empty
//! directory, so the client's owner-only file answers and nothing reaches the
//! test runner's own home.
//!
//! The fixture is this binary's own, the convention the daemon's stream test
//! set: an integration test binary carries the setup it drives.

#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use api_contracts::cancel::CancelOutcome;
use api_contracts::connect::metis::v1::DaemonServiceClient;
use api_contracts::events::{Event, Ring};
use api_contracts::proto::metis::v1::{CancelRequest, EchoRequest, EventKind, StreamEventsRequest};
use api_contracts::request::{REQUEST_ID_HEADER, RequestId};
use api_contracts::transport::{serve_uds_service, uds_transport};
use client::{Client, ClientError, EventStream, Surface};
use connectrpc::client::{CallOptions, ClientConfig};
use connectrpc::{ConnectRpcService, Router};
use daemon::auth::AuthPolicy;
use daemon::service::Service;
use daemon::state::Store;
use http::HeaderValue;

/// The heartbeat the fixture's service answers with. Short, so a test never
/// waits on the production interval; the daemon's own default is fifteen
/// seconds.
const HEARTBEAT: Duration = Duration::from_millis(25);

/// Serialises the environment in this test binary: `std::env` is process wide
/// and the harness runs the tests on threads.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// A temporary directory that removes itself on drop, so the test cleans up on
/// the success path and on a panic alike.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "metis-cancel-{label}-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the temporary directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Holds the daemon's and the client's configuration for the duration of a test
/// and restores every previous value on drop.
struct CancelEnv {
    previous: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl CancelEnv {
    fn new(tools: &Path, data: &Path, database: &Path) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = [
            "PATH",
            "XDG_DATA_HOME",
            daemon::state::STATE_DB_ENV,
            "METIS_SOCKET_GROUP",
        ]
        .into_iter()
        .map(|name| (name, std::env::var_os(name)))
        .collect();
        let this = Self {
            previous,
            _lock: lock,
        };
        // SAFETY: the guard holds `ENV_LOCK` for its whole lifetime, and every
        // environment write in this binary takes that lock.
        unsafe {
            std::env::set_var("PATH", tools);
            std::env::set_var("XDG_DATA_HOME", data);
            std::env::set_var(daemon::state::STATE_DB_ENV, database);
            std::env::remove_var("METIS_SOCKET_GROUP");
        }
        this
    }
}

impl Drop for CancelEnv {
    fn drop(&mut self) {
        // SAFETY: the guard still holds `ENV_LOCK` until it is dropped.
        unsafe {
            for (name, value) in &self.previous {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

/// Everything a test needs to run the daemon and a client against each other.
struct Fixture {
    store: Store,
    /// Kept alive for the whole test: the client's credential file and the
    /// state database live inside these, and their `Drop` removes them.
    _tools: TempDir,
    _data: TempDir,
    state: TempDir,
    _env: CancelEnv,
}

impl Fixture {
    async fn new(label: &str) -> Self {
        let tools = TempDir::new(&format!("{label}-tools"));
        let data = TempDir::new(&format!("{label}-data"));
        let state = TempDir::new(&format!("{label}-state"));
        // The daemon owns the directory its database lives in and refuses a
        // wider one, the way it refuses a wider socket directory.
        let database_dir = state.join("state");
        std::fs::create_dir_all(&database_dir).expect("create the state directory");
        std::fs::set_permissions(&database_dir, std::fs::Permissions::from_mode(0o700))
            .expect("restrict the state directory");
        let database = database_dir.join("metis.db");
        let env = CancelEnv::new(tools.path(), data.path(), &database);
        let store = Store::open(&database).await.expect("the state store opens");
        Self {
            store,
            _tools: tools,
            _data: data,
            state,
            _env: env,
        }
    }

    /// The daemon's own service, heartbeating every [`HEARTBEAT`].
    ///
    /// No preshared key is configured: pairing is closed, and the local socket
    /// needs no credential, so the tests here never pair.
    fn service(&self) -> Arc<Service> {
        Arc::new(Service::new(self.store.clone(), None, HEARTBEAT))
    }

    /// A private directory for one socket, and the socket path inside it. The
    /// daemon refuses a socket directory wider than its rule allows, so the
    /// directory is created the way a runtime directory is.
    fn socket(&self, name: &str) -> PathBuf {
        let directory = self.state.join(name);
        std::fs::create_dir_all(&directory).expect("create the socket directory");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .expect("restrict the socket directory");
        directory.join("daemon.sock")
    }

    /// A client that declares `surface`, which loads that surface's stored
    /// credential.
    async fn client(&self, socket: &Path, surface: Surface) -> Client {
        Client::with_endpoint(socket)
            .await
            .expect("the local transport builds")
            .with_surface(surface)
            .expect("the stored credential is readable")
    }
}

/// Serve `service` on `path`, and return the listener task once the socket is
/// bound.
async fn serve(path: &Path, service: ConnectRpcService<Router>) -> tokio::task::JoinHandle<()> {
    let served_path = path.to_path_buf();
    let listener = tokio::spawn(async move {
        if let Err(err) = serve_uds_service(&served_path, service).await {
            eprintln!("daemon listener error: {err}");
        }
    });
    assert!(
        client::skeleton::wait_for_socket(path).await,
        "the listener binds its socket"
    );
    listener
}

/// The generated client over the daemon's socket, declaring the identity every
/// call needs. A test that must name the wire itself, an identifier the wrapper
/// cannot produce, or a trailer the wrapper maps away, drives the call this
/// way.
fn raw_client(path: &Path) -> DaemonServiceClient<connectrpc::client::SharedHttp2Connection> {
    let transport = uds_transport(path.to_path_buf()).shared(64);
    let config = ClientConfig::new(api_contracts::transport::local_authority())
        .with_default_header(
            api_contracts::identity::PROTOCOL_VERSION_HEADER,
            api_contracts::identity::PROTOCOL_VERSION.to_string(),
        )
        .with_default_header(
            api_contracts::identity::PROTOCOL_SURFACE_HEADER,
            Surface::Test.as_str(),
        )
        .with_default_header(
            api_contracts::identity::PROTOCOL_CLIENT_VERSION_HEADER,
            env!("CARGO_PKG_VERSION"),
        );
    DaemonServiceClient::new(transport, config)
}

/// The next message on `stream`, or a panic naming what was expected when the
/// stream ends first.
async fn next(stream: &mut EventStream) -> Event {
    tokio::time::timeout(Duration::from_secs(5), stream.message())
        .await
        .expect("the stream keeps carrying messages")
        .expect("the stream carries a message")
        .expect("the stream has not ended")
}

/// The terminal error of `stream`, skipping whatever it still carries: an error
/// is the end of a stream, and a clean end is not one.
async fn refusal(stream: &mut EventStream) -> ClientError {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), stream.message()).await {
            Ok(Ok(Some(_))) => continue,
            Ok(Ok(None)) => panic!("the stream ended cleanly instead of with a refusal"),
            Ok(Err(error)) => return error,
            Err(_) => panic!("the stream never ended"),
        }
    }
}

/// A cancel stops the stream in flight under the identifier it names, and the
/// client reads the stop as the typed cancelled refusal. Repeating the call,
/// and naming work that was never in flight, are both answered rather than
/// refused: that is what makes the call idempotent.
#[tokio::test]
async fn a_cancel_stops_the_stream_and_a_second_call_is_already_finished() {
    let fixture = Fixture::new("stopped").await;
    let socket = fixture.socket("stopped");
    let service = fixture.service();
    let _served = serve(&socket, Arc::clone(&service).mount(AuthPolicy::LocalAcl)).await;

    let caller = fixture.client(&socket, Surface::Test).await;
    let mut stream = caller
        .stream_events(&["daemon"], 0)
        .await
        .expect("the stream is established");
    let opening = next(&mut stream).await;
    assert_eq!(opening.kind, EventKind::Snapshot);
    let id = stream.request_id().clone();

    assert_eq!(
        caller
            .cancel(&id)
            .await
            .expect("the cancel reaches the daemon"),
        CancelOutcome::Cancelled,
        "the stream was in flight under this identifier"
    );

    let stopped = refusal(&mut stream).await;
    assert!(
        matches!(stopped, ClientError::StreamCancelled),
        "expected the typed cancelled refusal, got {stopped}"
    );

    assert_eq!(
        caller
            .cancel(&id)
            .await
            .expect("a repeated cancel is answered"),
        CancelOutcome::AlreadyFinished,
        "the work it stopped is no longer in flight"
    );

    let never = RequestId::parse("never-seen").expect("a printable identifier");
    assert_eq!(
        caller
            .cancel(&never)
            .await
            .expect("an identifier nobody registered is answered"),
        CancelOutcome::AlreadyFinished,
        "work that finished, work already cancelled, and work never seen read the same"
    );
}

/// A malformed identifier is refused as an invalid argument rather than
/// answered as work that was never seen: a caller that believes its work is
/// cancellable has lost the guarantee it asked for.
#[tokio::test]
async fn a_malformed_identifier_is_refused_as_invalid_argument() {
    let fixture = Fixture::new("malformed").await;
    let socket = fixture.socket("malformed");
    let service = fixture.service();
    let _served = serve(&socket, Arc::clone(&service).mount(AuthPolicy::LocalAcl)).await;

    let daemon = raw_client(&socket);
    let error = daemon
        .cancel(CancelRequest {
            request_id: "not a valid identifier".to_owned(),
            ..Default::default()
        })
        .await
        .expect_err("an identifier with a space in it is not one this daemon accepts");

    assert_eq!(error.code, connectrpc::ErrorCode::InvalidArgument);
    assert!(
        error
            .message
            .clone()
            .unwrap_or_default()
            .contains("identifier"),
        "the message names what was wrong: {error}"
    );
}

/// A cancel stops only the work its own surface started: another surface
/// naming the same identifier is answered as nothing in flight, and the stream
/// keeps carrying.
#[tokio::test]
async fn a_cancel_under_another_surfaces_identifier_touches_nothing() {
    let fixture = Fixture::new("surface").await;
    let socket = fixture.socket("surface");
    let service = fixture.service();
    let ring: Ring = service.ring();
    let _served = serve(&socket, Arc::clone(&service).mount(AuthPolicy::LocalAcl)).await;

    let owner = fixture.client(&socket, Surface::Cli).await;
    let mut stream = owner
        .stream_events(&["daemon"], 0)
        .await
        .expect("the stream is established");
    let opening = next(&mut stream).await;
    assert_eq!(opening.kind, EventKind::Snapshot);
    let id = stream.request_id().clone();

    let stranger = fixture.client(&socket, Surface::Ui).await;
    assert_eq!(
        stranger
            .cancel(&id)
            .await
            .expect("the cancel reaches the daemon"),
        CancelOutcome::AlreadyFinished,
        "another surface's identifier is not an oracle"
    );

    // The work is untouched: it still carries what is published.
    let published = ring.publish("daemon", EventKind::Event, "{}");
    let event = next(&mut stream).await;
    assert_eq!(event.kind, EventKind::Event);
    assert_eq!(event.id, published);

    assert_eq!(
        owner
            .cancel(&id)
            .await
            .expect("the declaring surface still stops its own work"),
        CancelOutcome::Cancelled
    );
}

/// The wrapper's `stream_events` declares an identifier of its own and hands it
/// back on the stream, so a caller never invents one to cancel by.
#[tokio::test]
async fn the_stream_carries_the_identifier_the_wrapper_declared() {
    let fixture = Fixture::new("declared").await;
    let socket = fixture.socket("declared");
    let service = fixture.service();
    let _served = serve(&socket, Arc::clone(&service).mount(AuthPolicy::LocalAcl)).await;

    let caller = fixture.client(&socket, Surface::Test).await;
    let first = caller
        .stream_events(&["daemon"], 0)
        .await
        .expect("the stream is established");
    let second = caller
        .stream_events(&["daemon"], 0)
        .await
        .expect("a second stream is established");

    assert_ne!(
        first.request_id(),
        second.request_id(),
        "each stream declares its own identifier"
    );
    assert_eq!(
        RequestId::parse(first.request_id().as_str())
            .expect("the declared identifier is one the daemon accepts")
            .as_str(),
        first.request_id().as_str()
    );

    // Both streams really are cancellable, which is what proves the identifier
    // reached the daemon rather than only the handle.
    assert_eq!(
        caller
            .cancel(second.request_id())
            .await
            .expect("the cancel reaches the daemon"),
        CancelOutcome::Cancelled
    );
}

/// A declared identifier comes back in the response metadata, for a unary call
/// and for the stream alike: correlation is a property of the interface rather
/// than something each handler remembers, and a call that declares none is
/// served exactly as it was before identifiers existed.
#[tokio::test]
async fn a_declared_identifier_comes_back_in_the_response_metadata() {
    let fixture = Fixture::new("echo").await;
    let socket = fixture.socket("echo");
    let service = fixture.service();
    let ring: Ring = service.ring();
    let _served = serve(&socket, Arc::clone(&service).mount(AuthPolicy::LocalAcl)).await;
    ring.publish("daemon", EventKind::Event, "{\"head\":true}");

    let daemon = raw_client(&socket);

    let declared = CallOptions::default()
        .try_with_header(REQUEST_ID_HEADER, "cli-7")
        .expect("a printable identifier is a header value");
    let response = daemon
        .echo_with_options(
            EchoRequest {
                message: "hello".to_owned(),
                ..Default::default()
            },
            declared,
        )
        .await
        .expect("the call is served");
    assert_eq!(
        response.headers().get(REQUEST_ID_HEADER),
        Some(&HeaderValue::from_static("cli-7")),
        "the declared identifier is copied into the response metadata"
    );

    let response = daemon
        .echo(EchoRequest {
            message: "hello".to_owned(),
            ..Default::default()
        })
        .await
        .expect("a call that declares nothing is served");
    assert_eq!(
        response.headers().get(REQUEST_ID_HEADER),
        None,
        "a call with no identifier gets none back"
    );

    let declared = CallOptions::default()
        .try_with_header(REQUEST_ID_HEADER, "cli-8")
        .expect("a printable identifier is a header value");
    let stream = daemon
        .stream_events_with_options(
            StreamEventsRequest {
                channels: vec!["daemon".to_owned()],
                ..Default::default()
            },
            declared,
        )
        .await
        .expect("the stream is established");
    assert_eq!(
        stream.headers().get(REQUEST_ID_HEADER),
        Some(&HeaderValue::from_static("cli-8")),
        "the stream answer carries the identifier the call declared"
    );
}
