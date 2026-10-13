//! The event stream, over a real socket.
//!
//! The daemon's own service is served the way a listener serves it, the client
//! harness reaches it through the wrapper, and the ring the service serves is
//! the one the test publishes into. What no unit test can show is asserted
//! here: the stream's opening message is the snapshot, an event published while
//! the stream is open arrives afterwards with a greater identifier, an idle
//! stream carries heartbeats as ordinary messages, and the identifiers never
//! decrease.
//!
//! The authorization half of the stream is asserted here too: the credential
//! check runs at stream establishment, where the recorded gap was, so a stream
//! without a credential is refused and one with a paired token is served.
//!
//! The keychain machinery is held out of the way: `PATH` names one empty
//! directory, so the client's owner-only file answers and the token round trips
//! through a path the test owns.

#![cfg(unix)]

use std::ffi::OsString;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use api_contracts::events::{Event, Ring};
use api_contracts::proto::metis::v1::EventKind;
use api_contracts::transport::{serve_tcp_listener_service, serve_uds_service};
use client::{Client, ClientError, EventStream, Surface};
use connectrpc::{ConnectRpcService, Router};
use daemon::auth::AuthPolicy;
use daemon::secret::{Secret, SecretRef};
use daemon::service::Service;
use daemon::state::Store;

/// The preshared key the fixture resolves.
const PRESHARED_KEY: &str = "hunter2";

/// The heartbeat a test waits on. Short, so a test never waits on the
/// production interval; the daemon's own default is fifteen seconds.
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
            "metis-stream-{label}-{}-{sequence}",
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

/// Holds the daemon's and the client's configuration for the duration of a
/// test and restores every previous value on drop.
struct StreamEnv {
    previous: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl StreamEnv {
    fn new(tools: &Path, data: &Path, database: &Path) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = [
            "PATH",
            "XDG_DATA_HOME",
            daemon::state::STATE_DB_ENV,
            "METIS_SOCKET_GROUP",
            daemon::secret::PRESHARED_KEY_ENV,
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
            std::env::remove_var(daemon::secret::PRESHARED_KEY_ENV);
        }
        this
    }
}

impl Drop for StreamEnv {
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
    secret: Secret,
    /// Kept alive for the whole test: the credential file and the state
    /// database live inside these, and their `Drop` removes them.
    _tools: TempDir,
    _data: TempDir,
    state: TempDir,
    _env: StreamEnv,
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
        let env = StreamEnv::new(tools.path(), data.path(), &database);
        let store = Store::open(&database).await.expect("the state store opens");
        let secret = SecretRef::parse(&format!("literal:{PRESHARED_KEY}"))
            .resolve()
            .expect("the literal resolves");
        Self {
            store,
            secret,
            _tools: tools,
            _data: data,
            state,
            _env: env,
        }
    }

    /// The daemon's own service, heartbeating every [`HEARTBEAT`].
    fn service(&self) -> Arc<Service> {
        Arc::new(Service::new(
            self.store.clone(),
            Some(self.secret.clone()),
            HEARTBEAT,
        ))
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

    /// A client that reaches `addr` over TCP and declares `surface`.
    async fn remote_client(&self, addr: SocketAddr, surface: Surface) -> Client {
        Client::with_remote_addr(addr)
            .await
            .expect("the remote transport builds")
            .with_surface(surface)
            .expect("the stored credential is readable")
    }
}

/// Serve `service` on `path`, and return once the socket is bound.
async fn serve(path: &Path, service: ConnectRpcService<Router>) {
    let served_path = path.to_path_buf();
    tokio::spawn(async move {
        if let Err(err) = serve_uds_service(&served_path, service).await {
            eprintln!("daemon listener error: {err}");
        }
    });
    assert!(
        client::skeleton::wait_for_socket(path).await,
        "the listener binds its socket"
    );
}

/// Serve `service` on a loopback listener the kernel binds, and report the
/// address it chose.
async fn serve_over_tcp(service: ConnectRpcService<Router>) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind a loopback port");
    let addr = listener.local_addr().expect("read the bound address");
    tokio::spawn(async move {
        if let Err(err) = serve_tcp_listener_service(listener, service).await {
            eprintln!("remote service listener error: {err}");
        }
    });
    addr
}

/// The next message on `stream`, or a panic naming what was expected when the
/// stream ends first.
async fn next(stream: &mut EventStream) -> Event {
    stream
        .message()
        .await
        .expect("the stream carries a message")
        .expect("the stream has not ended")
}

/// Publish into `ring` until its window has moved past identifier `1`, so a
/// cursor of `1` is older than the ring can serve.
///
/// A cursor of `earliest_resumable() - 1` is still servable, because the event
/// that opened the window is exactly what follows it, so the window has to begin
/// at `3` or beyond before `1` is expired. The daemon's capacity is private, and
/// the ring's own bounds are the source of truth, so the test moves the window
/// rather than restating the capacity.
fn expire_cursor_one(ring: &Ring) {
    ring.publish("daemon", EventKind::Event, "{}");
    while ring.earliest_resumable() <= 2 {
        ring.publish("daemon", EventKind::Event, "{}");
        assert!(
            ring.latest_id() < 10_000,
            "the ring's window must advance past identifier 1"
        );
    }
}

/// The stream opens with the snapshot of the selected channel, carries the
/// event published while it was open with a greater identifier, skips the
/// channel it did not select, and never decreases an identifier.
#[tokio::test]
async fn a_stream_opens_with_the_snapshot_and_then_carries_the_next_event() {
    let fixture = Fixture::new("snapshot").await;
    let socket = fixture.socket("snapshot");
    let service = fixture.service();
    let ring: Ring = service.ring();
    serve(&socket, Arc::clone(&service).mount(AuthPolicy::LocalAcl)).await;

    // The event the stream will open with: it was published before the stream
    // existed, so it can only reach the client through the snapshot.
    let first = ring.publish("daemon", EventKind::Event, "{\"head\":true}");
    assert_eq!(first, 1, "identifiers start at one");

    let client = fixture.client(&socket, Surface::Test).await;
    let mut stream = client
        .stream_events(&["daemon"], 0)
        .await
        .expect("the stream is established");

    let snapshot = next(&mut stream).await;
    assert_eq!(snapshot.kind, EventKind::Snapshot);
    assert_eq!(snapshot.id, first);
    assert_eq!(snapshot.channel, "daemon");
    assert_eq!(snapshot.payload, "{\"head\":true}");

    // An event on a channel this stream did not select must not appear on it.
    let other = ring.publish("session:7", EventKind::Event, "{}");
    assert!(other > first);
    // The event the stream does carry.
    let second = ring.publish("daemon", EventKind::Event, "{\"next\":true}");
    assert!(second > other);

    let mut identifiers = vec![snapshot.id];
    let mut events = 0;
    let mut heartbeats = 0;
    while events < 1 || heartbeats < 1 {
        let message = tokio::time::timeout(Duration::from_secs(5), next(&mut stream))
            .await
            .expect("the stream keeps carrying messages");
        assert!(
            message.id >= *identifiers.last().expect("one identifier at least"),
            "identifiers never decrease: {identifiers:?} then {}",
            message.id
        );
        identifiers.push(message.id);
        match message.kind {
            EventKind::Event => {
                assert_eq!(message.channel, "daemon");
                assert_eq!(message.id, second);
                assert_eq!(message.payload, "{\"next\":true}");
                events += 1;
            }
            EventKind::Heartbeat => {
                assert!(
                    message.channel.is_empty(),
                    "a heartbeat belongs to the stream, not to a channel"
                );
                assert_eq!(
                    message.id, second,
                    "a heartbeat carries the latest identifier the daemon assigned"
                );
                heartbeats += 1;
            }
            other => panic!("the stream carried an unexpected kind: {other:?}"),
        }
    }
    assert_eq!(
        identifiers,
        vec![first, second, second],
        "the snapshot, then the event, then the heartbeat"
    );
}

/// A stream opened before anything was published opens with the empty snapshot,
/// heartbeats while it is idle, and carries the event that follows. The
/// heartbeat is an ordinary message: the same message type, the same identifier
/// space.
#[tokio::test]
async fn an_idle_stream_carries_heartbeats_and_then_the_event_that_follows() {
    let fixture = Fixture::new("heartbeat").await;
    let socket = fixture.socket("heartbeat");
    let service = fixture.service();
    let ring = service.ring();
    serve(&socket, Arc::clone(&service).mount(AuthPolicy::LocalAcl)).await;

    let client = fixture.client(&socket, Surface::Test).await;
    let mut stream = client
        .stream_events(&["daemon"], 0)
        .await
        .expect("the stream is established");

    // Nothing has been published, so the snapshot is the empty one: no channel,
    // no payload, and the daemon's latest identifier, which is `0` here.
    let empty = next(&mut stream).await;
    assert_eq!(empty.kind, EventKind::Snapshot);
    assert_eq!(empty.id, 0, "nothing has been assigned an identifier yet");
    assert!(empty.channel.is_empty());
    assert!(empty.payload.is_empty());

    // The stream then heartbeats while it stays idle.
    let heartbeat = tokio::time::timeout(Duration::from_secs(5), next(&mut stream))
        .await
        .expect("the stream heartbeats an idle stretch");
    assert_eq!(heartbeat.kind, EventKind::Heartbeat);
    assert_eq!(
        heartbeat.id, 0,
        "nothing has been assigned an identifier yet"
    );

    let published = ring.publish("daemon", EventKind::Event, "{\"late\":true}");
    assert_eq!(published, 1);
    let event = tokio::time::timeout(Duration::from_secs(5), next(&mut stream))
        .await
        .expect("the stream carries the event that follows");
    assert_eq!(event.kind, EventKind::Event);
    assert_eq!(event.id, published);
    assert_eq!(event.payload, "{\"late\":true}");
}

/// A stream opened with a cursor older than the ring is refused before any
/// message is served, and the client reads the refusal as the typed expired
/// cursor. The answer is the resync, which is the cursorless form of the same
/// call: a fresh stream whose first message is the snapshot.
#[tokio::test]
async fn an_expired_cursor_is_refused_and_the_resync_opens_with_the_snapshot() {
    let fixture = Fixture::new("expired").await;
    let socket = fixture.socket("expired");
    let service = fixture.service();
    let ring = service.ring();
    serve(&socket, Arc::clone(&service).mount(AuthPolicy::LocalAcl)).await;

    expire_cursor_one(&ring);
    assert!(
        ring.earliest_resumable() > 2,
        "cursor 1 is older than the ring"
    );

    let client = fixture.client(&socket, Surface::Test).await;
    let refused = client
        .stream_events(&["daemon"], 1)
        .await
        .expect_err("a cursor older than the window is refused");
    assert!(
        matches!(refused, ClientError::CursorExpired),
        "expected the typed expired-cursor refusal, got {refused}"
    );

    // The resync: the cursorless form of the same call, which begins with the
    // snapshot of the selected channel's head.
    let mut resync = client
        .stream_events(&["daemon"], 0)
        .await
        .expect("a cursorless stream is served");
    let snapshot = next(&mut resync).await;
    assert_eq!(snapshot.kind, EventKind::Snapshot);
    assert_eq!(snapshot.id, ring.latest_id());
    assert_eq!(snapshot.channel, "daemon");
}

/// The credential check runs at stream establishment: a stream without a
/// credential is refused, and the same call with a paired token is served.
#[tokio::test]
async fn the_stream_is_refused_without_a_credential_and_served_with_a_paired_token() {
    let fixture = Fixture::new("stream-auth").await;
    let service = fixture.service();
    let ring = service.ring();
    let addr = serve_over_tcp(Arc::clone(&service).mount(AuthPolicy::TokenRequired)).await;

    let head = ring.publish("daemon", EventKind::Event, "{\"head\":true}");

    // A surface with no token is refused before any handler runs.
    let stranger = fixture.remote_client(addr, Surface::Ui).await;
    let refused = stranger
        .stream_events(&["daemon"], 0)
        .await
        .expect_err("a stream without a credential is refused");
    assert!(
        matches!(refused, ClientError::CredentialMissing),
        "expected the typed missing-credential refusal, got {refused}"
    );

    // The same surface, once paired, is served.
    let mut caller = fixture.remote_client(addr, Surface::Cli).await;
    caller
        .pair(PRESHARED_KEY)
        .await
        .expect("the preshared key pairs the surface");
    let mut stream = caller
        .stream_events(&["daemon"], 0)
        .await
        .expect("a paired surface is served the stream");
    let snapshot = next(&mut stream).await;
    assert_eq!(snapshot.kind, EventKind::Snapshot);
    assert_eq!(snapshot.id, head);
}
