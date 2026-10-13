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
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use subtle::ConstantTimeEq;

use crate::token::TokenHash;

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
    /// A surface that already has a token row was paired again. The row that is
    /// there is left alone: the first token is the surface's token until it is
    /// revoked.
    SurfaceAlreadyPaired,
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
            Self::SurfaceAlreadyPaired => write!(
                f,
                "the surface already has a token; revoke it before pairing again"
            ),
        }
    }
}

impl std::error::Error for StateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoStateDir | Self::SurfaceAlreadyPaired => None,
            Self::Io(source) => Some(source),
            Self::Database(source) => Some(source),
        }
    }
}

/// The outcome of verifying a presented token against the store.
///
/// The answer is the outcome alone: the stored hash is never part of it, so a
/// caller that verifies can never end up holding a credential it could replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Verify {
    /// The surface has a row, and the presented hash is the one it holds.
    Verified,
    /// The surface has no row: it was never paired, or its token was revoked.
    UnknownSurface,
    /// The surface has a row, and the presented hash is a different one.
    Mismatch,
}

/// The daemon's persistent state: one SQLite database.
///
/// The connection pool is private: the store is the boundary, so a caller
/// reaches the tables through the methods below and never through SQL of its
/// own.
///
/// The handle is cheap to clone, because it is a pool: the service and the
/// authorization interceptor mounted beside it share one store.
#[derive(Debug, Clone)]
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
        Self::migrated(pool).await
    }

    /// Open a store that keeps nothing: the database lives in memory and is
    /// gone when the store is dropped.
    ///
    /// The store's operations are the same ones the file-backed store serves, so
    /// a test that is not about the file uses this.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::Database`] when the database refuses to open.
    pub async fn open_in_memory() -> Result<Self, StateError> {
        // An in-memory database belongs to the connection that opened it, so the
        // pool is held to one connection: a second one would see a second, empty
        // database rather than the migrated one.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().in_memory(true))
            .await
            .map_err(StateError::Database)?;
        Self::migrated(pool).await
    }

    /// Apply the embedded schema and wrap the pool.
    async fn migrated(pool: SqlitePool) -> Result<Self, StateError> {
        // The schema is embedded in the binary and applied here, so a fresh
        // database is usable without a separate migration step and no tool has
        // to run beside the daemon.
        sqlx::migrate!()
            .run(&pool)
            .await
            .map_err(|source| StateError::Database(source.into()))?;
        Ok(Self { pool })
    }

    /// Insert the token hash for `surface`.
    ///
    /// The token itself is neither taken nor written: the caller hashes what it
    /// minted and hands over the digest, which is all the store ever holds.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::SurfaceAlreadyPaired`] when the surface already has
    /// a row, which is refused rather than replaced, and
    /// [`StateError::Database`] when the row cannot be written for another
    /// reason.
    pub async fn insert_token(&self, surface: &str, hash: &TokenHash) -> Result<(), StateError> {
        sqlx::query(
            "INSERT INTO tokens (surface, hash, paired_at) \
             VALUES (?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        )
        .bind(surface)
        .bind(hash.as_bytes().as_slice())
        .execute(&self.pool)
        .await
        .map_err(|source| match &source {
            sqlx::Error::Database(error) if error.is_unique_violation() => {
                StateError::SurfaceAlreadyPaired
            }
            _ => StateError::Database(source),
        })?;
        Ok(())
    }

    /// Verify a presented token against the surface's row.
    ///
    /// The comparison is constant time, and the answer carries no hash: the
    /// caller learns whether the surface is paired and whether the token is its
    /// own, and nothing it could replay.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::Database`] when the row cannot be read.
    pub async fn verify_token(
        &self,
        surface: &str,
        hash: &TokenHash,
    ) -> Result<Verify, StateError> {
        let stored: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT hash FROM tokens WHERE surface = ?")
                .bind(surface)
                .fetch_optional(&self.pool)
                .await
                .map_err(StateError::Database)?;
        let Some(stored) = stored else {
            return Ok(Verify::UnknownSurface);
        };
        let matches: bool = stored.as_slice().ct_eq(hash.as_bytes().as_slice()).into();
        Ok(if matches {
            Verify::Verified
        } else {
            Verify::Mismatch
        })
    }

    /// Revoke `surface`: its row is removed, so its token no longer verifies and
    /// the other surfaces are not touched.
    ///
    /// Revoking a surface that has no row is not a failure: the caller asked for
    /// the state it gets.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::Database`] when the row cannot be removed.
    pub async fn revoke_token(&self, surface: &str) -> Result<(), StateError> {
        sqlx::query("DELETE FROM tokens WHERE surface = ?")
            .bind(surface)
            .execute(&self.pool)
            .await
            .map_err(StateError::Database)?;
        Ok(())
    }

    /// The surfaces that currently have a token row, in name order.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::Database`] when the rows cannot be read.
    pub async fn surfaces(&self) -> Result<Vec<String>, StateError> {
        sqlx::query_scalar("SELECT surface FROM tokens ORDER BY surface")
            .fetch_all(&self.pool)
            .await
            .map_err(StateError::Database)
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
///
/// The value is kept as the operating system reported it, so a path that is not
/// valid UTF-8 is used exactly rather than lossily.
fn env_value(variable: &str) -> Option<std::ffi::OsString> {
    let value = std::env::var_os(variable)?;
    if value.is_empty() {
        return None;
    }
    Some(value)
}
