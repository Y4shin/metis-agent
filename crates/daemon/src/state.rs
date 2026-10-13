//! The daemon's persistent state: one SQLite database, opened and migrated when
//! the daemon starts.
//!
//! The path is resolved once, at startup: [`STATE_DB_ENV`] names the file
//! directly, and otherwise the file lives under the user's data home. The
//! daemon owns the directory it opens the database in, so it creates that
//! directory private to its own user and the file owner-only.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use api_contracts::transport;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;

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

/// The mode the state directory carries: the daemon's own user, and nobody
/// else, because the file it holds is the token table.
#[cfg(unix)]
const STATE_DIR_MODE: u32 = 0o700;

/// The mode the database file carries: owner-only, for the same reason.
#[cfg(unix)]
const STATE_FILE_MODE: u32 = 0o600;

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
    /// The state directory or the database file could not be prepared.
    Io(io::Error),
    /// The database refused to open or to run an operation.
    Database(sqlx::Error),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStateDir => write!(
                f,
                "neither {STATE_DB_ENV}, nor {XDG_DATA_HOME_ENV}, nor {HOME_ENV} names a place to keep the daemon's state"
            ),
            Self::Io(source) => {
                write!(f, "the daemon's state path could not be prepared: {source}")
            }
            Self::Database(source) => {
                write!(f, "the state database refused an operation: {source}")
            }
        }
    }
}

impl std::error::Error for StateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoStateDir => None,
            Self::Io(source) => Some(source),
            Self::Database(source) => Some(source),
        }
    }
}

/// The daemon's persistent state: one SQLite database.
///
/// The connection pool is private: the store is the boundary, so a caller
/// reaches the tables through the methods below and never through SQL of its
/// own.
#[derive(Debug)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Open the database at `path`, creating it and the directory that holds it
    /// when they are missing.
    ///
    /// The file is opened through a connection pool, because the daemon serves
    /// several calls at once, and the schema is applied before this returns.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::Io`] when the path cannot be prepared and
    /// [`StateError::Database`] when the database refuses to open.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StateError> {
        let path = path.as_ref();
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| {
                StateError::Io(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "the database path has no directory to keep it in",
                ))
            })?;
        prepare(directory, path)?;

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options)
            .await
            .map_err(StateError::Database)?;

        // The schema is embedded in the binary and applied here, so a fresh
        // database is usable without a separate migration step and no tool has
        // to run beside the daemon.
        sqlx::migrate!()
            .run(&pool)
            .await
            .map_err(|source| StateError::Database(source.into()))?;
        Ok(Self { pool })
    }
}

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

/// Create the directory that holds the database and the file itself, under the
/// one private-path rule the socket's directory and socket file use.
///
/// The directory is created at 700 and the file at 600, so neither is ever
/// readable by another user, and the file exists before the database opens it.
/// A file that is already there keeps its contents: this never truncates the
/// state, and it narrows a file whose mode was widened outside the daemon.
#[cfg(unix)]
fn prepare(directory: &Path, path: &Path) -> Result<(), StateError> {
    use std::os::unix::fs::OpenOptionsExt;

    transport::ensure_private_dir(directory, STATE_DIR_MODE).map_err(StateError::Io)?;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .mode(STATE_FILE_MODE)
        .open(path)
        .map_err(StateError::Io)?;
    transport::restrict_file(path, STATE_FILE_MODE).map_err(StateError::Io)
}

/// Create the directory that holds the database and the file itself.
///
/// A platform without the Unix access rule creates both without a mode; the
/// front-end for it brings its own rule, the way it brings its own transport.
#[cfg(not(unix))]
fn prepare(directory: &Path, path: &Path) -> Result<(), StateError> {
    std::fs::create_dir_all(directory).map_err(StateError::Io)?;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(path)
        .map_err(StateError::Io)?;
    Ok(())
}

/// An environment variable's value when it is set and not empty.
fn env_value(variable: &str) -> Option<String> {
    let value = std::env::var_os(variable)?;
    if value.is_empty() {
        return None;
    }
    Some(value.to_string_lossy().into_owned())
}
