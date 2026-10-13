//! Where the client keeps its per-surface credential.
//!
//! The platform keychain is tried first, through its command line tool, and an
//! owner-only file answers when no such tool is on the machine. The fallback is
//! reported rather than silent, and a keychain that exists but fails is an
//! error: falling back there would hide a credential that is stored but cannot
//! be read.
//!
//! The keychain half runs against a stub tool the test owns, so the machine's
//! real keychain is never touched. Both halves pin the file's mode and the
//! directory's mode, which is what makes the fallback safe.

#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use client::credential::{self, Backend, CredentialError};

/// The tool this platform's keychain is reached through, spelled the way the
/// test's stub is named.
const TOOL: &str = if cfg!(target_os = "macos") {
    "security"
} else {
    "secret-tool"
};

/// A token in its canonical form: 64 lowercase hexadecimal characters.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// The environment variable pointing the stub tool at its own store.
const STUB_DIR_ENV: &str = "METIS_TEST_KEYCHAIN_DIR";

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
            "metis-credential-{label}-{}-{sequence}",
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

/// Holds the environment a credential test needs and restores every previous
/// value on drop, so a panic cannot leak one into another test. The lock it
/// carries serialises every environment test in this binary.
struct CredentialEnv {
    previous: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl CredentialEnv {
    /// Hold the environment with `PATH` pointing at `tool_dir` alone, so a real
    /// keychain tool on the machine cannot answer for a stub, and the client's
    /// data home pointing at `data_home`.
    fn new(tool_dir: &Path, data_home: &Path) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = ["PATH", "XDG_DATA_HOME", "HOME", STUB_DIR_ENV]
            .into_iter()
            .map(|name| (name, std::env::var_os(name)))
            .collect();
        let this = Self {
            previous,
            _lock: lock,
        };
        this.set("PATH", &tool_dir.to_string_lossy());
        this.set("XDG_DATA_HOME", &data_home.to_string_lossy());
        this.set(STUB_DIR_ENV, &data_home.to_string_lossy());
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

impl Drop for CredentialEnv {
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

/// Write an executable shell script at `dir/name`.
fn write_tool(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write the stub tool");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("make the stub tool executable");
}

/// Write a keychain stub that keeps one file per account, speaking both tools'
/// command lines and exit behaviour. It uses shell builtins only, because the
/// test removes every other directory from `PATH`.
fn keychain_stub(dir: &Path) {
    write_tool(
        dir,
        TOOL,
        &format!(
            r#"#!/bin/sh
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
        ),
    );
}

/// With no keychain tool on the machine, the credential lands in an owner-only
/// file, the fallback is reported, and the store answers with it again.
#[test]
fn without_a_keychain_tool_the_owner_only_file_answers() {
    let tools = TempDir::new("no-tool");
    let data = TempDir::new("data");
    let _env = CredentialEnv::new(tools.path(), data.path());

    let backend = credential::store("cli", TOKEN).expect("the file backend answers");
    let expected = data.join("metis/client/cli.token");
    assert_eq!(backend, Backend::File(expected.clone()));
    assert!(
        backend
            .to_string()
            .contains(&expected.display().to_string()),
        "the reported backend names the file it used: {backend}"
    );

    // The file is owner-only and the directory that holds it is private: the
    // fallback carries the access rule the keychain would have carried.
    let file = std::fs::metadata(&expected).expect("the credential file exists");
    assert_eq!(file.permissions().mode() & 0o777, 0o600);
    let directory = std::fs::metadata(data.join("metis/client")).expect("the directory exists");
    assert_eq!(directory.permissions().mode() & 0o777, 0o700);

    let stored = credential::load("cli")
        .expect("the store answers")
        .expect("the credential is there");
    assert_eq!(stored.token(), TOKEN);
    assert_eq!(stored.backend(), &Backend::File(expected.clone()));

    assert_eq!(
        credential::load("ui").expect("another surface is its own entry"),
        None
    );

    assert_eq!(
        credential::remove("cli").expect("the credential is removed"),
        Some(Backend::File(expected.clone()))
    );
    assert_eq!(credential::load("cli").expect("the store answers"), None);
    assert!(
        !expected.exists(),
        "removing the credential removes the file"
    );
    assert_eq!(
        credential::remove("cli").expect("removing nothing is not a failure"),
        None
    );
}

/// With a keychain tool on the machine, the credential goes there, and the
/// report says so.
#[test]
fn with_a_keychain_tool_the_keychain_answers() {
    let tools = TempDir::new("tool");
    let data = TempDir::new("data");
    keychain_stub(tools.path());
    let _env = CredentialEnv::new(tools.path(), data.path());

    let backend = credential::store("cli", TOKEN).expect("the keychain answers");
    assert_eq!(backend, Backend::Keychain);
    assert!(
        backend.to_string().contains("keychain"),
        "the reported backend says which one it was: {backend}"
    );

    let stored = credential::load("cli")
        .expect("the keychain answers")
        .expect("the credential is there");
    assert_eq!(stored.token(), TOKEN);
    assert_eq!(stored.backend(), &Backend::Keychain);

    // The fallback file is not written when the keychain answered.
    assert!(!data.join("metis/client/cli.token").exists());

    assert_eq!(
        credential::remove("cli").expect("the keychain clears the entry"),
        Some(Backend::Keychain)
    );
    assert_eq!(credential::load("cli").expect("the keychain answers"), None);
}

/// A keychain tool that is there but fails is an error, never a silent
/// fallback: falling back would hide a credential that is stored but cannot be
/// read, and the next pairing would report success while nothing works.
#[test]
fn a_failing_keychain_is_not_a_fallback() {
    let tools = TempDir::new("failing-tool");
    let data = TempDir::new("data");
    write_tool(
        tools.path(),
        TOOL,
        "#!/bin/sh\necho 'the keychain is locked' >&2\nexit 1\n",
    );
    let _env = CredentialEnv::new(tools.path(), data.path());

    let error = credential::store("cli", TOKEN).expect_err("the keychain refused");
    assert!(
        matches!(error, CredentialError::Keychain(_)),
        "the refusal keeps what the keychain said: {error}"
    );
    assert!(
        !data.join("metis/client/cli.token").exists(),
        "nothing was filed as a fallback"
    );
}

/// A surface name is a file name, so one that would step out of the credential
/// directory is refused rather than followed.
#[test]
fn an_unsafe_surface_name_is_refused() {
    let tools = TempDir::new("unsafe");
    let data = TempDir::new("data");
    let _env = CredentialEnv::new(tools.path(), data.path());

    for surface in ["", ".", "..", "../escape", "a/b"] {
        let error = credential::store(surface, TOKEN)
            .expect_err("a surface name that is not a file name is refused");
        assert!(
            matches!(error, CredentialError::InvalidSurface { .. }),
            "the refusal is typed for {surface:?}: {error}"
        );
    }
    assert!(
        !data.join("escape.token").exists(),
        "nothing was written outside the credential directory"
    );
}

/// With no data home and no home directory there is nowhere to keep a
/// credential, which is a typed refusal rather than a path in the current
/// directory.
#[test]
fn no_data_home_is_a_typed_refusal() {
    let tools = TempDir::new("no-data-home");
    let data = TempDir::new("data");
    let env = CredentialEnv::new(tools.path(), data.path());
    env.unset("XDG_DATA_HOME");
    env.unset("HOME");

    let error = credential::store("cli", TOKEN).expect_err("there is nowhere to write");
    assert!(matches!(error, CredentialError::NoDataDir), "got {error}");

    let error = credential::load("cli").expect_err("there is nowhere to read from either");
    assert!(
        matches!(error, CredentialError::NoDataDir),
        "reading answers the same way writing does: {error}"
    );
}
