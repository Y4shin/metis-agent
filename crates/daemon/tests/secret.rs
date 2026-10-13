//! The preshared key's configuration form, exercised through its public
//! interface.
//!
//! One string names where the key comes from: a literal, an environment
//! variable, the platform keystore, or a secret file. The file form is the one
//! with an access rule, because a key that group or others can read is not a
//! secret.

#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use daemon::secret::{PRESHARED_KEY_ENV, SecretError, SecretRef, resolve_preshared_key};

/// The tool this platform's keychain is reached through, spelled the way the
/// test's stub is named.
const TOOL: &str = if cfg!(target_os = "macos") {
    "security"
} else {
    "secret-tool"
};

/// Serialises the environment in this test binary: `std::env` is process wide
/// and the harness runs the tests on threads.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// A temporary directory that removes itself on drop, so the test cleans up on
/// the success path and on a panic alike.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("metis-secret-{}-{sequence}", std::process::id()));
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

/// Holds the variables a test sets for its duration and restores every previous
/// value on drop, so a panic cannot leak one into another test. The lock it
/// carries serialises every environment test in this binary.
struct SecretEnv {
    previous: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl SecretEnv {
    /// Hold the environment with `PATH` pointing at `tool_dir` alone, so a real
    /// keychain tool on the machine cannot answer for a stub.
    fn new(tool_dir: &Path) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = ["PATH", PRESHARED_KEY_ENV, "METIS_TEST_KEY"]
            .into_iter()
            .map(|name| (name, std::env::var_os(name)))
            .collect();
        let this = Self {
            previous,
            _lock: lock,
        };
        this.set("PATH", &tool_dir.to_string_lossy());
        this.unset(PRESHARED_KEY_ENV);
        this.unset("METIS_TEST_KEY");
        this
    }

    /// Edition 2024 makes mutating the process environment unsafe because it is
    /// not thread-safe. `_lock` serialises every environment test in this
    /// binary, so nothing races these writes.
    fn set(&self, name: &str, value: &str) {
        // SAFETY: the guard holds `ENV_LOCK` for its whole lifetime.
        unsafe { std::env::set_var(name, value) };
    }

    fn unset(&self, name: &str) {
        // SAFETY: the guard holds `ENV_LOCK` for its whole lifetime.
        unsafe { std::env::remove_var(name) };
    }
}

impl Drop for SecretEnv {
    fn drop(&mut self) {
        for (name, value) in &self.previous {
            // SAFETY: the guard holds `ENV_LOCK` until it is dropped.
            unsafe {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

/// Write a secret file with `mode` and return its path.
fn secret_file(dir: &TempDir, name: &str, mode: u32, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("write the secret file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
        .expect("set the secret file's mode");
    path
}

/// Write a keychain stub that prints `secret` for any lookup, and return the
/// directory holding it.
fn keychain_stub(dir: &TempDir, secret: Option<&str>) {
    let body = match secret {
        Some(secret) => format!("#!/bin/sh\nprintf '%s\\n' '{secret}'\n"),
        // A keychain that is there but cannot answer: it exits non-zero and
        // says why, which is a refusal rather than an empty entry.
        None => "#!/bin/sh\necho 'the keychain is locked' >&2\nexit 1\n".to_owned(),
    };
    let path = dir.join(TOOL);
    std::fs::write(&path, body).expect("write the keychain stub");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("make the keychain stub executable");
}

/// A literal is the shortest form, and a bare value is a literal too: the four
/// prefixes are explicit, so nothing else has to be guessed at.
#[test]
fn a_literal_is_the_bare_value() {
    let tools = TempDir::new();
    let _env = SecretEnv::new(tools.path());

    let bare = SecretRef::parse("hunter2")
        .resolve()
        .expect("a bare value is a literal");
    assert_eq!(bare.expose(), "hunter2");
    let literal = SecretRef::parse("literal:hunter2")
        .resolve()
        .expect("the explicit form is the same literal");
    assert_eq!(literal.expose(), "hunter2");
}

/// A secret is never printed: its `Debug` form redacts the value, so an
/// accidental `{:?}` cannot put the key in a log line.
#[test]
fn a_secret_does_not_print_itself() {
    let tools = TempDir::new();
    let _env = SecretEnv::new(tools.path());

    let secret = SecretRef::parse("literal:hunter2")
        .resolve()
        .expect("a literal resolves");
    let printed = format!("{secret:?}");
    assert!(
        !printed.contains("hunter2"),
        "the value is redacted: {printed}"
    );
    assert!(
        printed.contains("redacted"),
        "the printed form says it is redacted: {printed}"
    );
}

/// The environment form reads the variable, and the value is used exactly as
/// the environment carries it.
#[test]
fn the_environment_form_reads_the_variable() {
    let tools = TempDir::new();
    let env = SecretEnv::new(tools.path());
    env.set("METIS_TEST_KEY", "from-the-environment");

    let secret = SecretRef::parse("env:METIS_TEST_KEY")
        .resolve()
        .expect("the variable is set");
    assert_eq!(secret.expose(), "from-the-environment");
}

/// An unset variable is a typed refusal, not an empty secret: an empty key
/// would let any caller pair with an empty one.
#[test]
fn an_unset_variable_is_unresolved() {
    let tools = TempDir::new();
    let _env = SecretEnv::new(tools.path());

    let error = SecretRef::parse("env:METIS_TEST_KEY")
        .resolve()
        .expect_err("the variable is not set");
    assert!(matches!(error, SecretError::Unresolved), "got {error}");
    assert!(
        !error.to_string().is_empty(),
        "the refusal says something a person can act on"
    );
    assert_eq!(
        SecretRef::parse("env:METIS_TEST_KEY").to_string(),
        "the variable METIS_TEST_KEY",
        "the reference names its source, so a log line can say which one failed"
    );
}

/// A variable that is set to nothing is not a secret either, for the same
/// reason an unset one is not.
#[test]
fn an_empty_variable_is_unresolved() {
    let tools = TempDir::new();
    let env = SecretEnv::new(tools.path());
    env.set("METIS_TEST_KEY", "");

    let error = SecretRef::parse("env:METIS_TEST_KEY")
        .resolve()
        .expect_err("an empty value is no secret");
    assert!(matches!(error, SecretError::Unresolved), "got {error}");
}

/// The file form reads a file only its owner can read, and the trailing newline
/// an editor leaves behind is not part of the key.
#[test]
fn an_owner_only_file_resolves() {
    let tools = TempDir::new();
    let _env = SecretEnv::new(tools.path());
    let file = secret_file(&tools, "key", 0o600, "from-the-file\n");

    let secret = SecretRef::parse(&format!("file:{}", file.display()))
        .resolve()
        .expect("an owner-only file resolves");
    assert_eq!(secret.expose(), "from-the-file");
}

/// A file group or others can read is refused before it is read, and the
/// refusal names the path and the mode so an operator knows what to fix.
#[test]
fn a_group_or_other_readable_file_is_refused() {
    let tools = TempDir::new();
    let _env = SecretEnv::new(tools.path());
    let file = secret_file(&tools, "key", 0o644, "visible-to-everyone");

    let error = SecretRef::parse(&format!("file:{}", file.display()))
        .resolve()
        .expect_err("a readable file is not a secret file");
    match &error {
        SecretError::TooPermissive { path, mode } => {
            assert_eq!(*path, file);
            assert_eq!(*mode, 0o644);
        }
        other => panic!("expected the permission refusal, got {other}"),
    }
    let message = error.to_string();
    assert!(
        message.contains(&file.display().to_string()),
        "the refusal names the path: {message}"
    );
    assert!(
        message.contains("644"),
        "the refusal names the mode: {message}"
    );
}

/// A secret file that is not there is a typed refusal that names the path, so a
/// misconfigured daemon says which file it wanted.
#[test]
fn a_missing_file_is_a_typed_error() {
    let tools = TempDir::new();
    let _env = SecretEnv::new(tools.path());
    let missing = tools.join("absent");

    let error = SecretRef::parse(&format!("file:{}", missing.display()))
        .resolve()
        .expect_err("there is no file to read");
    assert!(matches!(error, SecretError::Unreadable(_)), "got {error}");
    assert!(
        SecretRef::parse(&format!("file:{}", missing.display()))
            .to_string()
            .contains(&missing.display().to_string()),
        "the reference names the file it failed to read"
    );
}

/// The keystore form reads the platform keychain through its tool, which is the
/// same keychain the client keeps its token in.
#[test]
fn the_keystore_form_reads_the_platform_keychain() {
    let tools = TempDir::new();
    keychain_stub(&tools, Some("from-the-keychain"));
    let _env = SecretEnv::new(tools.path());

    let secret = SecretRef::parse("keystore:cli")
        .resolve()
        .expect("the keychain answers");
    assert_eq!(secret.expose(), "from-the-keychain");
}

/// A keychain tool that is there but refuses is a typed refusal. It is never
/// read as an absent secret: a credential that was stored and cannot be read
/// must not look like one that was never stored.
#[test]
fn an_unreachable_keystore_is_a_typed_error() {
    let tools = TempDir::new();
    keychain_stub(&tools, None);
    let _env = SecretEnv::new(tools.path());

    let error = SecretRef::parse("keystore:cli")
        .resolve()
        .expect_err("the keychain refuses");
    assert!(matches!(error, SecretError::Unreadable(_)), "got {error}");
}

/// With no keychain tool on the machine at all there is nothing to read, which
/// is a different refusal from a tool that refused.
#[test]
fn a_keystore_without_a_tool_is_unresolved() {
    let empty = TempDir::new();
    let _env = SecretEnv::new(empty.path());

    let error = SecretRef::parse("keystore:cli")
        .resolve()
        .expect_err("the platform has no keychain tool here");
    assert!(matches!(error, SecretError::Unresolved), "got {error}");
}

/// The daemon's own configuration is the same one string, and an unset
/// `METIS_PRESHARED_KEY` means pairing is closed, which is a typed refusal
/// rather than an empty key.
#[test]
fn the_configured_preshared_key_is_a_secret_reference() {
    let tools = TempDir::new();
    let env = SecretEnv::new(tools.path());

    let error = resolve_preshared_key().expect_err("nothing is configured");
    assert!(matches!(error, SecretError::Unresolved), "got {error}");
    assert!(
        error.to_string().contains(PRESHARED_KEY_ENV),
        "the refusal names the variable: {error}"
    );

    env.set(PRESHARED_KEY_ENV, "literal:hunter2");
    let secret = resolve_preshared_key().expect("the literal resolves");
    assert_eq!(secret.expose(), "hunter2");

    // The variable carries a reference, not necessarily a literal, so a
    // configured file is read through the same rule.
    let file = secret_file(&tools, "key", 0o600, "from-the-file");
    env.set(PRESHARED_KEY_ENV, &format!("file:{}", file.display()));
    let secret = resolve_preshared_key().expect("the file resolves");
    assert_eq!(secret.expose(), "from-the-file");
}
