//! The local access control and the endpoint rule, exercised together.
//!
//! The daemon restricts the directory the endpoint points into, and a local
//! client reaches it with no credential: the operating system is the access
//! control, and the declaration it carries is only a declaration. The daemon's
//! directory and socket modes are asserted here at the level the client sees
//! them; the configured group and the ownership half are unit-tested beside the
//! rule in `api_contracts::transport`.

#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;

use api_contracts::transport::ENDPOINT_ENV;
use client::Client;
use client::skeleton;

/// The mode the daemon gives a socket directory and the socket itself when no
/// group is configured. Restated rather than imported: this test asserts what
/// the daemon leaves on disk, so it reads the mode the same way an operator
/// would.
const DAEMON_PRIVATE_MODE: u32 = 0o700;

/// A temporary base directory that removes itself on drop. It is deliberately
/// not created: the daemon creates the directory the socket lives in, which is
/// half of what this test asserts.
struct TempBase {
    path: PathBuf,
}

impl TempBase {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "metis-acl-client-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self { path }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempBase {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Sets `METIS_ENDPOINT` for the duration of a test and restores the previous
/// value on drop, so a panic cannot leak the override into another test.
struct EndpointOverride {
    previous: Option<OsString>,
}

impl EndpointOverride {
    fn set(value: &std::path::Path) -> Self {
        let previous = std::env::var_os(ENDPOINT_ENV);
        // Edition 2024 makes mutating the process environment unsafe because it
        // is not thread-safe. This is the only test in this binary that reads
        // or writes the variable, so nothing races it.
        unsafe { std::env::set_var(ENDPOINT_ENV, value) };
        Self { previous }
    }
}

impl Drop for EndpointOverride {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(ENDPOINT_ENV, value),
                None => std::env::remove_var(ENDPOINT_ENV),
            }
        }
    }
}

fn mode_of(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).expect("the path exists").mode() & 0o7777
}

/// The daemon creates the directory the endpoint points into, private to its
/// own user, and a local client reaches the daemon through it without any
/// credential.
#[tokio::test]
async fn the_daemon_restricts_the_directory_its_endpoint_points_into() {
    let base = TempBase::new();
    let socket = base.join("daemon.sock");
    let override_guard = EndpointOverride::set(&socket);

    let server_socket = socket.clone();
    tokio::spawn(async move {
        if let Err(err) = skeleton::serve_uds(&server_socket).await {
            eprintln!("skeleton server error: {err}");
        }
    });
    assert!(
        skeleton::wait_for_socket(&socket).await,
        "the listener binds its socket"
    );

    let directory = std::fs::metadata(&base.path).expect("the daemon created the directory");
    assert_eq!(
        directory.mode() & 0o7777,
        DAEMON_PRIVATE_MODE,
        "the daemon's socket directory is private"
    );
    assert_eq!(
        std::fs::symlink_metadata(&socket)
            .expect("the socket exists")
            .mode()
            & 0o7777,
        DAEMON_PRIVATE_MODE,
        "the socket itself is private"
    );

    // The wrapper resolves the override and connects to the socket the daemon
    // restricted. Nothing here carries a credential.
    let caller = Client::local().await.expect("the local client builds");
    let echoed = caller
        .echo("hello through the private socket")
        .await
        .expect("the wrapper performs the echo RPC");
    assert_eq!(echoed, "hello through the private socket");

    drop(caller);
    drop(override_guard);
}

/// A directory somebody else prepared, wider than the rule allows, is refused
/// instead of being adopted and widened: the daemon owns its directory or it
/// does not start.
#[tokio::test]
async fn a_wider_pre_existing_directory_is_refused() {
    let base = TempBase::new();
    std::fs::create_dir_all(&base.path).expect("the test creates the directory");
    std::fs::set_permissions(&base.path, std::fs::Permissions::from_mode(0o755))
        .expect("widen the directory");
    let socket = base.join("daemon.sock");

    // Bounded, so a refusal that never comes fails the test instead of
    // hanging it: an accepted directory would leave the accept loop running.
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        skeleton::serve_uds(&socket),
    )
    .await
    .expect("the listener refuses before it accepts")
    .expect_err("the listener refuses a shared socket directory");
    assert!(
        error.to_string().contains("wider"),
        "the refusal names the reason: {error}"
    );
    assert!(!socket.exists(), "nothing was bound");
    assert_eq!(
        mode_of(&base.path),
        0o755,
        "the directory is left exactly as it was"
    );
}
