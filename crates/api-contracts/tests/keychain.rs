//! The platform keychain, driven through its command line tool.
//!
//! `available()` probes `PATH`, and the three operations drive whichever tool
//! the platform uses. The assertions run against a stub tool the test owns: the
//! machine's real keychain is never touched, and the stub's protocol is the one
//! the real tool documents, so a change to the arguments the module passes
//! shows up here rather than on a user's desktop.

#![cfg(unix)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use api_contracts::keychain::{self, KeychainError, SERVICE_NAME};

/// The tool this platform uses, spelled the way the test's stub is named.
const TOOL: &str = if cfg!(target_os = "macos") {
    "security"
} else {
    "secret-tool"
};

/// Serialises the environment in this test binary: `std::env` is process wide
/// and the harness runs the tests on threads.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// A temporary directory that removes itself on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "metis-keychain-{label}-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the temporary directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Holds `PATH` and the stub's own directory for the duration of a test and
/// restores both on drop, so a panic cannot leak either into another test.
struct PathEnv {
    previous: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl PathEnv {
    fn set(path: &Path, stub_dir: &Path) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = ["PATH", STUB_DIR_ENV]
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
            std::env::set_var("PATH", path);
            std::env::set_var(STUB_DIR_ENV, stub_dir);
        }
        this
    }
}

impl Drop for PathEnv {
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

/// The environment variable pointing the stub tool at its own store.
const STUB_DIR_ENV: &str = "METIS_TEST_KEYCHAIN_DIR";

/// Write the stub tool into `dir`.
///
/// The stub speaks both tools' command lines and exit behaviour, so the module's
/// arguments are checked on either platform: `store` reads the secret from
/// standard input on Linux and from `-w` on macOS, `lookup` prints it and exits
/// non-zero with nothing on standard error when there is no entry, and `clear`
/// empties it. One file per account is all the state it needs. It uses shell
/// builtins only, because the test removes every other directory from `PATH`.
fn install_stub(dir: &Path) {
    let tool = dir.join(TOOL);
    let script = format!(
        r#"#!/bin/sh
# A stub keychain tool: one file per account, under $METIS_TEST_KEYCHAIN_DIR.
set -eu
dir="${{{stub_dir}:?}}"
cmd="${{1:-}}"
shift || true
account=""
secret=""
while [ $# -gt 0 ]; do
  case "$1" in
    account | -a) account="${{2:-}}" ;;
    -w) secret="${{2:-}}" ;;
  esac
  shift
done
if [ -z "$account" ]; then
  echo "stub: no account" >&2
  exit 2
fi
file="$dir/$account.secret"
case "$cmd" in
  store | add-generic-password)
    : > "$file"
    if [ -n "$secret" ]; then
      printf '%s\n' "$secret" > "$file"
    else
      while IFS= read -r line; do
        printf '%s\n' "$line" >> "$file"
      done
    fi
    ;;
  lookup | find-generic-password)
    if [ ! -s "$file" ]; then
      exit 1
    fi
    while IFS= read -r line; do
      printf '%s\n' "$line"
    done < "$file"
    ;;
  clear | delete-generic-password)
    if [ ! -s "$file" ]; then
      exit 1
    fi
    : > "$file"
    ;;
  *)
    echo "stub: unknown command $cmd" >&2
    exit 2
    ;;
esac
"#,
        stub_dir = STUB_DIR_ENV
    );
    std::fs::write(&tool, script).expect("write the stub tool");
    let mut permissions = std::fs::metadata(&tool)
        .expect("the stub tool exists")
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    std::fs::set_permissions(&tool, permissions).expect("make the stub tool executable");
}

/// With nothing on `PATH`, there is no keychain tool, and the caller falls back
/// to a file backend instead of guessing.
#[test]
fn no_tool_on_the_path_means_no_keychain() {
    let empty = TempDir::new("empty-path");
    let stub_dir = TempDir::new("stub-dir");
    let _env = PathEnv::set(empty.path(), stub_dir.path());

    assert_eq!(keychain::available(), None);
}

/// A tool on `PATH` is probed and named, and it is the one every operation
/// then drives.
#[test]
fn a_tool_on_the_path_is_available() {
    let stub_dir = TempDir::new("stub");
    install_stub(stub_dir.path());
    let _env = PathEnv::set(stub_dir.path(), stub_dir.path());

    let available = keychain::available().expect("the stub tool is on PATH");
    assert_eq!(available.command(), TOOL);
}

/// The three operations round-trip through the tool: store writes the secret,
/// lookup reads it back, and remove clears it.
#[test]
fn store_lookup_and_remove_round_trip_through_the_tool() {
    let stub_dir = TempDir::new("round-trip");
    install_stub(stub_dir.path());
    let _env = PathEnv::set(stub_dir.path(), stub_dir.path());

    assert_eq!(
        keychain::lookup("cli").expect("a missing entry is not a failure"),
        None
    );

    keychain::store("cli", "a-surface-token").expect("the tool stores the secret");
    assert_eq!(
        keychain::lookup("cli")
            .expect("the tool reads the secret back")
            .as_deref(),
        Some("a-surface-token\n"),
        "the secret comes back exactly as the tool printed it"
    );
    assert_eq!(
        keychain::lookup("ui").expect("another account is its own entry"),
        None
    );

    keychain::remove("cli").expect("the tool clears the entry");
    assert_eq!(keychain::lookup("cli").expect("the entry is gone"), None);
}

/// A tool that runs and fails is a typed error, not an empty answer: a
/// keychain that exists but cannot answer must not look like "no credential".
#[test]
fn a_refusing_tool_is_a_typed_error() {
    let stub_dir = TempDir::new("refusing");
    let tool = stub_dir.path().join(TOOL);
    std::fs::write(
        &tool,
        "#!/bin/sh\necho 'the keychain is locked' >&2\nexit 1\n",
    )
    .expect("write the refusing tool");
    let mut permissions = std::fs::metadata(&tool)
        .expect("the tool exists")
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    std::fs::set_permissions(&tool, permissions).expect("make the tool executable");
    let _env = PathEnv::set(stub_dir.path(), stub_dir.path());

    let error = keychain::lookup("cli").expect_err("the tool refused the lookup");
    match error {
        KeychainError::Refused { message, .. } => {
            assert!(
                message.contains("locked"),
                "the refusal carries what the tool said: {message}"
            );
        }
        other => panic!("expected a refusal, got {other}"),
    }
}

/// The service name is the one place every Metis credential is filed under, so
/// it is pinned rather than spelled at each call site.
#[test]
fn the_service_name_is_metis_daemon() {
    assert_eq!(SERVICE_NAME, "metis-daemon");
}
