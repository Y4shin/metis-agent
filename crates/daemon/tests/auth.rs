//! Pairing and the authorization check, over a real socket.
//!
//! The daemon's own service is served the way a listener serves it, and the
//! client harness reaches it through the wrapper. Two things are asserted here
//! that no unit test can: that a refusal is produced by the interceptor rather
//! than by a handler (a recording service proves its handler never ran), and
//! that the whole credential lifecycle survives a daemon restart.
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

use api_contracts::connect::metis::v1::{
    DAEMON_SERVICE_ECHO_SPEC, DaemonService, DaemonServiceExt,
};
use api_contracts::proto::metis::v1::{EchoRequest, EchoResponse, PairRequest, PairResponse};
use api_contracts::transport::{serve_tcp_listener_service, serve_uds_service};
use client::credential::{self, Backend};
use client::{Client, ClientError, CredentialCause, Surface};
use connectrpc::{
    ConnectError, ConnectRpcService, ErrorCode, RequestContext, Response, Router, ServiceRequest,
};
use daemon::auth::{AuthInterceptor, AuthPolicy};
use daemon::secret::{Secret, SecretRef};
use daemon::service::service;
use daemon::state::{Store, Verify};
use daemon::token::Token;

/// A well-formed token. The store hashes the text, so these are exactly the
/// characters the daemon sees.
const TOKEN_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// A second well-formed token, for the call that presents the wrong one.
const TOKEN_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// The preshared key the fixture resolves.
const PRESHARED_KEY: &str = "hunter2";

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
            "metis-auth-{label}-{}-{sequence}",
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
struct AuthEnv {
    previous: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl AuthEnv {
    /// Point `PATH` at one empty directory, so no keychain tool answers, and
    /// the client's file is the backend; point the client's data home and the
    /// daemon's state at the fixture's directories.
    fn new(tools: &Path, data: &Path, database: &Path) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = [
            "PATH",
            "XDG_DATA_HOME",
            daemon::state::STATE_DB_ENV,
            "METIS_SOCKET_GROUP",
            daemon::secret::PRESHARED_KEY_ENV,
            "METIS_TEST_KEY",
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
            std::env::remove_var("METIS_TEST_KEY");
        }
        this
    }

    /// Set one of the variables this guard holds. The guard keeps the lock, so
    /// the write is as serialised as the ones above.
    fn set(&self, name: &str, value: &str) {
        // SAFETY: the guard holds `ENV_LOCK` for its whole lifetime.
        unsafe { std::env::set_var(name, value) };
    }
}

impl Drop for AuthEnv {
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

/// Everything a test needs to run a daemon and a client against each other.
struct Fixture {
    store: Store,
    /// Kept alive for the whole test: the credential file and the state
    /// database live inside these, and their `Drop` removes them.
    _tools: TempDir,
    _data: TempDir,
    state: TempDir,
    secret: Secret,
    _env: AuthEnv,
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
        let env = AuthEnv::new(tools.path(), data.path(), &database);
        let store = Store::open(&database).await.expect("the state store opens");
        let secret = SecretRef::parse(&format!("literal:{PRESHARED_KEY}"))
            .resolve()
            .expect("the literal resolves");
        Self {
            store,
            _tools: tools,
            _data: data,
            state,
            secret,
            _env: env,
        }
    }

    /// The database file, for a test that restarts the daemon.
    fn database(&self) -> PathBuf {
        self.state.join("state").join("metis.db")
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

    /// Serve the daemon's own service with `policy`.
    async fn serve_daemon(&self, path: &Path, policy: AuthPolicy) {
        let service = service(self.store.clone(), self.secret.clone(), policy);
        serve(path, service).await;
    }

    /// Serve the daemon's own service with the key its configuration names,
    /// resolved the way the daemon resolves it at startup.
    async fn serve_configured_daemon(&self, path: &Path, policy: AuthPolicy) {
        let secret =
            daemon::secret::resolve_preshared_key().expect("the configured preshared key resolves");
        serve(path, service(self.store.clone(), secret, policy)).await;
    }

    /// Set one of the daemon's or the client's configuration variables.
    fn set_var(&self, name: &str, value: &str) {
        self._env.set(name, value);
    }

    /// Every byte the daemon keeps for this database: the file and, when a
    /// journal sits beside it, the journal too.
    fn database_bytes(&self) -> Vec<u8> {
        let path = self.database();
        let mut bytes = std::fs::read(&path).expect("the database file is readable");
        for suffix in ["-wal", "-journal"] {
            let mut sidecar = path.as_os_str().to_owned();
            sidecar.push(suffix);
            if let Ok(extra) = std::fs::read(PathBuf::from(sidecar)) {
                bytes.extend(extra);
            }
        }
        bytes
    }

    /// The recording service with `policy`, so a test can prove whether a
    /// handler ran, on either front-end.
    fn recording_service(&self, policy: AuthPolicy, served: &Served) -> ConnectRpcService<Router> {
        let router = Arc::new(RecordingDaemon {
            served: served.clone(),
        })
        .register(Router::new());
        ConnectRpcService::new(router)
            .with_interceptor(AuthInterceptor::new(self.store.clone(), policy))
    }

    /// Serve a recording service with `policy`, so a test can prove whether a
    /// handler ran.
    async fn serve_recording(&self, path: &Path, policy: AuthPolicy, served: &Served) {
        serve(path, self.recording_service(policy, served)).await;
    }

    /// A client that declares `surface`, which loads that surface's stored
    /// credential.
    async fn client(&self, path: &Path, surface: Surface) -> Client {
        Client::with_endpoint(path)
            .await
            .expect("the local transport builds")
            .with_surface(surface)
            .expect("the stored credential is readable")
    }

    /// A client that reaches `addr` over TCP and declares `surface`, which
    /// loads that surface's stored credential.
    async fn remote_client(&self, addr: SocketAddr, surface: Surface) -> Client {
        Client::with_remote_addr(addr)
            .await
            .expect("the remote transport builds")
            .with_surface(surface)
            .expect("the stored credential is readable")
    }
}

/// Serve `service` on a loopback TCP listener the kernel binds, and report the
/// address it chose.
///
/// The listener is bound here rather than by the front-end, so the test never
/// guesses a port: this is the entry point a caller that must report the bound
/// address uses, and the one every TCP test below mounts a credential policy
/// through.
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

/// The procedures a handler was asked to serve.
#[derive(Clone, Default)]
struct Served(Arc<Mutex<Vec<String>>>);

impl Served {
    fn paths(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, path: Option<&str>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(path.unwrap_or_default().to_owned());
    }
}

/// A service that records what reached it. It answers every call, so a test
/// that sees a refusal knows the refusal came from the interceptor in front of
/// it, and its recorder is empty exactly when the handler never ran.
struct RecordingDaemon {
    served: Served,
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
        self.served.record(ctx.path());
        Response::ok(EchoResponse {
            message: request.message.to_owned(),
            received_at: String::new(),
            ..Default::default()
        })
    }

    async fn pair(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, PairRequest>,
    ) -> Result<Response<PairResponse>, ConnectError> {
        self.served.record(ctx.path());
        Response::ok(PairResponse {
            token: String::new(),
            ..Default::default()
        })
    }
}

/// Keep a well-formed token for `surface` as if a previous pairing had done it.
fn plant(surface: &str, token: &str) -> Backend {
    credential::store(surface, token).expect("the credential store takes the token")
}

/// The hash the daemon's own rule computes for a token text.
fn landed_hash(token_text: &str) -> Token {
    Token::parse(token_text).expect("the token text is well formed")
}

/// A pairing call mints one token, the daemon keeps only its hash, and a second
/// call for the same surface is refused rather than replacing the first.
#[tokio::test]
async fn pairing_mints_one_token_the_daemon_keeps_only_as_a_hash() {
    let fixture = Fixture::new("pairing").await;
    let socket = fixture.socket("pairing");
    fixture.serve_daemon(&socket, AuthPolicy::LocalAcl).await;

    let mut caller = fixture.client(&socket, Surface::Cli).await;
    let backend = caller
        .pair(PRESHARED_KEY)
        .await
        .expect("the preshared key pairs the surface");
    let Backend::File(path) = &backend else {
        panic!("the file backend answers when no keychain tool is on PATH");
    };
    assert!(
        backend.to_string().contains(&path.display().to_string()),
        "the reported backend says where the credential went: {backend}"
    );

    // The daemon holds a row for the surface and nothing else.
    assert_eq!(
        fixture.store.surfaces().await.expect("the rows read"),
        vec!["cli".to_owned()]
    );

    // The token the client kept verifies under the landed hash rule, so the two
    // halves agree on what the credential is.
    let stored = credential::load("cli")
        .expect("the credential store answers")
        .expect("the token was kept");
    let presented = landed_hash(stored.token());

    // What the daemon kept is the hash: the token text it answered with is
    // nowhere in the database, file or journal.
    let database = fixture.database_bytes();
    assert!(
        !database
            .windows(stored.token().len())
            .any(|window| window == stored.token().as_bytes()),
        "the database holds only the hash"
    );
    assert_eq!(
        fixture
            .store
            .verify_token("cli", &presented.hash())
            .await
            .expect("the row reads"),
        Verify::Verified
    );

    // The client that paired carries its credential, and the local listener
    // serves it.
    assert_eq!(
        caller
            .echo("paired and served")
            .await
            .expect("the call lands"),
        "paired and served"
    );

    // A second pairing is refused, and the first token is still the surface's:
    // the same call cannot mint a second credential.
    let error = caller
        .pair(PRESHARED_KEY)
        .await
        .expect_err("an already paired surface is refused");
    match error {
        ClientError::Call(source) => assert_eq!(source.code, ErrorCode::AlreadyExists),
        other => panic!("expected the already-paired refusal, got {other}"),
    }
    assert_eq!(
        fixture
            .store
            .verify_token("cli", &presented.hash())
            .await
            .expect("the row reads"),
        Verify::Verified,
        "the first row is the one that is left"
    );
    assert_eq!(
        fixture.store.surfaces().await.expect("the rows read").len(),
        1
    );
}

/// Pairing is gated by the key the daemon's own configuration names: the
/// reference is resolved the way the daemon resolves it at startup, and the
/// resolved secret is what the pairing call is checked against.
#[tokio::test]
async fn the_pairing_gate_uses_the_configured_key_reference() {
    let fixture = Fixture::new("configured-key").await;
    let socket = fixture.socket("configured-key");

    // The configuration names a secret file, which is one of the four forms,
    // and the file is private to its owner.
    let key_file = fixture.state.join("preshared");
    std::fs::write(&key_file, "the-configured-key\n").expect("write the secret file");
    std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600))
        .expect("restrict the secret file");
    fixture.set_var(
        daemon::secret::PRESHARED_KEY_ENV,
        &format!("file:{}", key_file.display()),
    );
    fixture
        .serve_configured_daemon(&socket, AuthPolicy::TokenRequired)
        .await;

    let mut caller = fixture.client(&socket, Surface::Cli).await;
    let wrong = caller
        .pair("not-the-configured-key")
        .await
        .expect_err("the configured key is the only one that pairs");
    match wrong {
        ClientError::Call(source) => assert_eq!(source.code, ErrorCode::PermissionDenied),
        other => panic!("expected the pairing refusal, got {other}"),
    }
    assert!(
        fixture
            .store
            .surfaces()
            .await
            .expect("the rows read")
            .is_empty()
    );

    caller
        .pair("the-configured-key")
        .await
        .expect("the key the configuration resolved pairs the surface");
    assert_eq!(
        caller
            .echo("paired with the configured key")
            .await
            .expect("the daemon serves the paired surface"),
        "paired with the configured key"
    );
}

/// A wrong preshared key mints nothing: no row, and no credential on the
/// client.
#[tokio::test]
async fn a_wrong_preshared_key_is_refused_and_mints_nothing() {
    let fixture = Fixture::new("wrong-key").await;
    let socket = fixture.socket("wrong-key");
    fixture.serve_daemon(&socket, AuthPolicy::LocalAcl).await;

    let mut caller = fixture.client(&socket, Surface::Cli).await;
    let error = caller
        .pair("not-the-key")
        .await
        .expect_err("the key does not match");
    match error {
        ClientError::Call(source) => assert_eq!(source.code, ErrorCode::PermissionDenied),
        other => panic!("expected the pairing refusal, got {other}"),
    }
    assert!(
        fixture
            .store
            .surfaces()
            .await
            .expect("the rows read")
            .is_empty(),
        "nothing was paired"
    );
    assert_eq!(
        credential::load("cli").expect("the credential store answers"),
        None,
        "the client kept no credential"
    );
}

/// Under the local policy the operating system's access control has already
/// authenticated the caller, so a call with no credential is served, and the
/// procedure path it reaches the handler with carries nothing but the method.
#[tokio::test]
async fn the_local_policy_serves_a_call_with_no_credential() {
    let fixture = Fixture::new("local-acl").await;
    let socket = fixture.socket("local-acl");
    let served = Served::default();
    fixture
        .serve_recording(&socket, AuthPolicy::LocalAcl, &served)
        .await;

    let caller = fixture.client(&socket, Surface::Cli).await;
    assert_eq!(
        caller
            .echo("no credential needed here")
            .await
            .expect("the local policy serves the call"),
        "no credential needed here"
    );
    assert_eq!(
        served.paths(),
        vec![DAEMON_SERVICE_ECHO_SPEC.procedure.to_owned()],
        "the handler ran, and the request carried no credential in its URL"
    );
}

/// A call with no credential at all is refused as missing, which is what tells
/// a caller to pair.
#[tokio::test]
async fn a_call_with_no_credential_is_missing_rather_than_rejected() {
    let fixture = Fixture::new("missing").await;
    let socket = fixture.socket("missing");
    let served = Served::default();
    fixture
        .serve_recording(&socket, AuthPolicy::TokenRequired, &served)
        .await;

    // The surface has no stored credential, so the call carries none.
    let caller = fixture.client(&socket, Surface::Cli).await;
    let error = caller
        .echo("who am i")
        .await
        .expect_err("a credential is required here");
    assert!(
        matches!(error, ClientError::CredentialMissing),
        "expected the missing-credential refusal, got {error}"
    );
    assert!(
        served.paths().is_empty(),
        "the handler never ran: {served:?}",
        served = served.paths()
    );
}

/// A surface with no row is refused as revoked, even when the credential is
/// well formed: the daemon never paired it, or its token was taken away.
#[tokio::test]
async fn an_unknown_surface_is_a_revoked_credential() {
    let fixture = Fixture::new("revoked").await;
    let socket = fixture.socket("revoked");
    let served = Served::default();
    fixture
        .serve_recording(&socket, AuthPolicy::TokenRequired, &served)
        .await;

    plant("cli", TOKEN_A);
    let caller = fixture.client(&socket, Surface::Cli).await;
    let error = caller
        .echo("let me in")
        .await
        .expect_err("the surface has no row");
    assert!(
        matches!(
            error,
            ClientError::CredentialRejected {
                cause: CredentialCause::Revoked
            }
        ),
        "expected the revoked refusal, got {error}"
    );
    assert!(served.paths().is_empty(), "the handler never ran");
}

/// A surface that has a row, presented with a different token, is refused as
/// invalid: a revoked credential and a wrong one are different answers.
#[tokio::test]
async fn a_wrong_token_is_an_invalid_credential() {
    let fixture = Fixture::new("invalid").await;
    let socket = fixture.socket("invalid");
    let served = Served::default();
    fixture
        .serve_recording(&socket, AuthPolicy::TokenRequired, &served)
        .await;

    fixture
        .store
        .insert_token("cli", &landed_hash(TOKEN_A).hash())
        .await
        .expect("the row is written");
    plant("cli", TOKEN_B);

    let caller = fixture.client(&socket, Surface::Cli).await;
    let error = caller
        .echo("let me in")
        .await
        .expect_err("the token is not this surface's");
    assert!(
        matches!(
            error,
            ClientError::CredentialRejected {
                cause: CredentialCause::Invalid
            }
        ),
        "expected the invalid-credential refusal, got {error}"
    );
    assert!(served.paths().is_empty(), "the handler never ran");
}

/// A valid credential is served, and the request reaches the handler with the
/// method path alone: the credential travels as metadata, never in a URL.
#[tokio::test]
async fn a_valid_credential_is_served() {
    let fixture = Fixture::new("valid").await;
    let socket = fixture.socket("valid");
    let served = Served::default();
    fixture
        .serve_recording(&socket, AuthPolicy::TokenRequired, &served)
        .await;

    fixture
        .store
        .insert_token("cli", &landed_hash(TOKEN_A).hash())
        .await
        .expect("the row is written");
    plant("cli", TOKEN_A);

    let caller = fixture.client(&socket, Surface::Cli).await;
    assert_eq!(
        caller
            .echo("let me in")
            .await
            .expect("the token is this surface's"),
        "let me in"
    );
    assert_eq!(
        served.paths(),
        vec![DAEMON_SERVICE_ECHO_SPEC.procedure.to_owned()],
        "the handler ran, and no credential is in the path"
    );
    assert!(
        !served.paths()[0].contains(TOKEN_A) && !served.paths()[0].contains('?'),
        "the path carries neither the credential nor a query"
    );
}

/// Every paired surface reaches the whole exposed surface: a token is the
/// daemon's whole authorization model, with no per-client filtering.
#[tokio::test]
async fn every_paired_surface_reaches_the_same_surface() {
    let fixture = Fixture::new("no-filtering").await;
    let socket = fixture.socket("no-filtering");
    fixture
        .serve_daemon(&socket, AuthPolicy::TokenRequired)
        .await;

    for (surface, name) in [(Surface::Cli, "cli"), (Surface::Ui, "ui")] {
        fixture
            .store
            .insert_token(name, &landed_hash(TOKEN_A).hash())
            .await
            .expect("the row is written");
        plant(name, TOKEN_A);
        let caller = fixture.client(&socket, surface).await;
        assert_eq!(
            caller
                .echo("the whole surface")
                .await
                .expect("every paired surface is served"),
            "the whole surface"
        );
    }
}

/// Removing one surface's row and restarting the daemon revokes that surface
/// while the others keep working.
#[tokio::test]
async fn removing_one_row_and_restarting_revokes_only_that_surface() {
    let fixture = Fixture::new("restart").await;
    let first_socket = fixture.socket("first");
    fixture
        .serve_daemon(&first_socket, AuthPolicy::TokenRequired)
        .await;

    for (surface, name) in [(Surface::Cli, "cli"), (Surface::Ui, "ui")] {
        fixture
            .store
            .insert_token(name, &landed_hash(TOKEN_A).hash())
            .await
            .expect("the row is written");
        plant(name, TOKEN_A);
        let caller = fixture.client(&first_socket, surface).await;
        caller
            .echo("before the restart")
            .await
            .expect("both surfaces work before the revocation");
    }

    // The operator removes one surface's row, and the daemon is restarted: a
    // fresh store on the same file, serving a fresh listener.
    fixture
        .store
        .revoke_token("cli")
        .await
        .expect("the row is removed");
    let restarted = Store::open(fixture.database())
        .await
        .expect("the restarted daemon opens the same database");
    let second_socket = fixture.socket("second");
    serve(
        &second_socket,
        service(
            restarted.clone(),
            fixture.secret.clone(),
            AuthPolicy::TokenRequired,
        ),
    )
    .await;

    let cli = fixture.client(&second_socket, Surface::Cli).await;
    let error = cli
        .echo("after the restart")
        .await
        .expect_err("the revoked surface is refused");
    assert!(
        matches!(
            error,
            ClientError::CredentialRejected {
                cause: CredentialCause::Revoked
            }
        ),
        "expected the revoked refusal, got {error}"
    );

    let ui = fixture.client(&second_socket, Surface::Ui).await;
    assert_eq!(
        ui.echo("still served")
            .await
            .expect("the other surface keeps working"),
        "still served"
    );
    assert_eq!(
        restarted.surfaces().await.expect("the rows read"),
        vec!["ui".to_owned()]
    );
}

/// A service carrying `AuthPolicy::TokenRequired`, served over a real TCP
/// socket, refuses a call with no credential.
///
/// This is what the landed TCP front-ends cannot do: they take a bare `Router`,
/// and the interceptor chain lives on the service, so the policy only travels
/// through a front-end that takes a service.
#[tokio::test]
async fn the_tcp_service_front_end_refuses_a_call_with_no_credential() {
    let fixture = Fixture::new("tcp-missing").await;
    let served = Served::default();
    let service = fixture.recording_service(AuthPolicy::TokenRequired, &served);
    let addr = serve_over_tcp(service).await;

    // The surface has no stored credential, so the call carries none.
    let caller = fixture.remote_client(addr, Surface::Cli).await;
    let error = caller
        .echo("who am i")
        .await
        .expect_err("a credential is required here");
    assert!(
        matches!(error, ClientError::CredentialMissing),
        "expected the missing-credential refusal, got {error}"
    );
    assert!(
        served.paths().is_empty(),
        "the handler never ran: {served:?}",
        served = served.paths()
    );
}
