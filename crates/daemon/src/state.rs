//! The daemon's persistent state: one SQLite database, opened and migrated when
//! the daemon starts.
//!
//! The path is resolved once, at startup: [`STATE_DB_ENV`] names the file
//! directly, and otherwise the file lives under the user's data home. The
//! daemon owns the directory it opens the database in, so it creates that
//! directory private to its own user and the file owner-only.

use std::fmt;
use std::path::PathBuf;

/// The environment variable that names the database file directly.
pub const STATE_DB_ENV: &str = "METIS_STATE_DB";

/// The environment variable that names the user's data home.
const XDG_DATA_HOME_ENV: &str = "XDG_DATA_HOME";

/// The environment variable that names the user's home directory, which carries
/// the data home's default location.
const HOME_ENV: &str = "HOME";

/// The directory below the data home that belongs to this daemon.
const STATE_DIR: &str = "metis";

/// The data home's default location, below the home directory.
const STATE_DIR_BELOW_HOME: &str = ".local/share";

/// The database file's name inside the state directory.
const STATE_FILE: &str = "metis.db";

/// The state could not be resolved, opened, or queried.
///
/// The enum is non-exhaustive: the later tables this store grows add their own
/// failures without breaking a caller that matches on it.
#[derive(Debug)]
#[non_exhaustive]
pub enum StateError {
    /// Neither a configured database path, nor a data home, nor a home
    /// directory is set, so there is no path the daemon may own.
    NoStateDir,
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStateDir => write!(
                f,
                "neither {STATE_DB_ENV}, nor {XDG_DATA_HOME_ENV}, nor {HOME_ENV} names a place to keep the daemon's state"
            ),
        }
    }
}

impl std::error::Error for StateError {}

/// The database path the daemon opens.
///
/// [`STATE_DB_ENV`] wins, then `$XDG_DATA_HOME/metis/metis.db`, then
/// `$HOME/.local/share/metis/metis.db`. An absent or empty variable is not a
/// configuration, so an empty override falls back to the default rather than
/// naming the current directory.
///
/// # Errors
///
/// Returns [`StateError::NoStateDir`] when none of the three variables names a
/// place to keep the state.
pub fn resolve_db_path() -> Result<PathBuf, StateError> {
    if let Some(value) = env_value(STATE_DB_ENV) {
        return Ok(PathBuf::from(value));
    }
    if let Some(data_home) = env_value(XDG_DATA_HOME_ENV) {
        return Ok(PathBuf::from(data_home).join(STATE_DIR).join(STATE_FILE));
    }
    if let Some(home) = env_value(HOME_ENV) {
        return Ok(PathBuf::from(home)
            .join(STATE_DIR_BELOW_HOME)
            .join(STATE_DIR)
            .join(STATE_FILE));
    }
    Err(StateError::NoStateDir)
}

/// An environment variable's value when it is set and not empty.
fn env_value(variable: &str) -> Option<String> {
    let value = std::env::var_os(variable)?;
    if value.is_empty() {
        return None;
    }
    Some(value.to_string_lossy().into_owned())
}
