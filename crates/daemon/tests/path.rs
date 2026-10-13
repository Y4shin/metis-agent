//! The database path the daemon resolves, and the rule that decides it.
//!
//! `METIS_STATE_DB` wins, then `$XDG_DATA_HOME`, then `$HOME/.local/share`, and
//! with neither a data home nor a home directory there is no path the daemon may
//! own: the result is a typed error rather than a guess.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use daemon::state::{STATE_DB_ENV, StateError, resolve_db_path};

/// The two standard variables the default path is derived from. They are
/// restated rather than imported: this test asserts the documented rule, so it
/// names the variables the documentation names.
const XDG_DATA_HOME_ENV: &str = "XDG_DATA_HOME";
const HOME_ENV: &str = "HOME";

/// Serialises the environment in this test binary: `std::env` is process wide,
/// so two tests mutating it at once would race. Every test that reads the rule
/// holds this lock, including the ones that want nothing set.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// A temporary directory that removes itself on drop, so the test cleans up on
/// the success path and on a panic alike.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "metis-state-path-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the temporary directory");
        Self { path }
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

/// Holds the three variables the rule reads for the duration of a test and
/// restores them on drop. The lock it carries serialises every environment test
/// in this binary.
struct StateEnv {
    previous: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl StateEnv {
    /// Take the lock and start from nothing set: every rule is then observed in
    /// isolation rather than against whatever the developer's shell exported.
    fn empty() -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let variables = [STATE_DB_ENV, XDG_DATA_HOME_ENV, HOME_ENV];
        let previous = variables
            .iter()
            .map(|variable| (*variable, std::env::var_os(variable)))
            .collect();
        // Edition 2024 makes mutating the process environment unsafe because it
        // is not thread-safe. Every test that touches these variables holds
        // `ENV_LOCK`, so nothing races these writes.
        unsafe {
            for variable in variables {
                std::env::remove_var(variable);
            }
        }
        Self {
            previous,
            _lock: lock,
        }
    }

    fn set(&self, variable: &str, value: impl AsRef<std::ffi::OsStr>) {
        // SAFETY: the lock is held for the guard's whole lifetime.
        unsafe { std::env::set_var(variable, value) };
    }
}

impl Drop for StateEnv {
    fn drop(&mut self) {
        // SAFETY: see `set`; the lock is held until this guard is dropped.
        unsafe {
            for (variable, value) in &self.previous {
                match value {
                    Some(value) => std::env::set_var(variable, value),
                    None => std::env::remove_var(variable),
                }
            }
        }
    }
}

/// The variable wins, then the data home, then the home directory, and with
/// none of the three set there is no path to own.
#[test]
fn the_database_path_follows_the_variable_then_the_data_home_then_home() {
    let env = StateEnv::empty();
    let base = TempDir::new();
    let configured = base.join("configured/metis.db");
    let data_home = base.join("data");
    let home = base.join("home");

    assert!(
        matches!(resolve_db_path(), Err(StateError::NoStateDir)),
        "with nothing set there is no state directory"
    );

    env.set(HOME_ENV, &home);
    assert_eq!(
        resolve_db_path().expect("the home directory carries the data home default"),
        home.join(".local/share/metis/metis.db")
    );

    env.set(XDG_DATA_HOME_ENV, &data_home);
    assert_eq!(
        resolve_db_path().expect("the data home wins over the home directory"),
        data_home.join("metis/metis.db")
    );

    env.set(STATE_DB_ENV, &configured);
    assert_eq!(
        resolve_db_path().expect("the configured path wins over both defaults"),
        configured
    );

    // An empty value is not a configuration, matching the endpoint rule the
    // client resolves: it falls back to the default rather than naming "".
    env.set(STATE_DB_ENV, "");
    assert_eq!(
        resolve_db_path().expect("an empty override falls back"),
        data_home.join("metis/metis.db")
    );

    env.set(XDG_DATA_HOME_ENV, "");
    assert_eq!(
        resolve_db_path().expect("an empty data home falls back to home"),
        home.join(".local/share/metis/metis.db")
    );
}
