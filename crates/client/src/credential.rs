//! Where the client keeps its per-surface credential.
//!
//! The daemon keeps only a token's hash, so the token text itself has to live
//! somewhere on the surface that paired: the platform keychain when its command
//! line tool is on the machine, and an owner-only file when it is not. Which
//! backend answered is part of every answer, because "it is in your keychain"
//! and "it is in this file" are different things for an operator to look after.
//!
//! The fallback is not silent in the other direction either: a keychain that is
//! there but refuses is an error, because a credential that was stored and
//! cannot be read must not look like one that was never stored.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use api_contracts::keychain;
use api_contracts::transport;

/// The environment variable that names the user's data home, which carries the
/// credential directory's default location.
const XDG_DATA_HOME_ENV: &str = "XDG_DATA_HOME";

/// The environment variable that names the home directory.
const HOME_ENV: &str = "HOME";

/// The directory below the data home that belongs to this client.
const CREDENTIAL_DIR: &str = "metis/client";

/// The extension of the fallback file.
const CREDENTIAL_EXTENSION: &str = "token";

/// The mode the credential directory carries: this user, and nobody else.
#[cfg(unix)]
const CREDENTIAL_DIR_MODE: u32 = 0o700;

/// The mode the fallback file carries: owner-only, because it is a credential.
#[cfg(unix)]
const CREDENTIAL_FILE_MODE: u32 = 0o600;

/// Which backend holds a credential.
///
/// Reporting the backend is part of the interface: a caller that pairs learns
/// whether the token went to the keychain or to a file, and where that file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// The platform keychain, through its command line tool.
    Keychain,
    /// The owner-only file at this path, the fallback when no tool is on the
    /// machine.
    File(PathBuf),
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Keychain => write!(f, "the platform keychain"),
            Self::File(path) => write!(f, "the owner-only file {}", path.display()),
        }
    }
}

/// A credential the store answered with, and the backend that answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    token: String,
    backend: Backend,
}

impl Stored {
    /// The token text, as the daemon issued it.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The backend the credential was read from.
    #[must_use]
    pub fn backend(&self) -> &Backend {
        &self.backend
    }
}

/// The credential could not be kept, read, or removed.
///
/// The enum is non-exhaustive: a later backend adds its own failures without
/// breaking a caller that matches on it.
#[derive(Debug)]
#[non_exhaustive]
pub enum CredentialError {
    /// Neither a data home nor a home directory names a place to keep the
    /// fallback file, so there is no path this user owns.
    NoDataDir,
    /// The surface name is not a file name, so it cannot name a credential
    /// file without stepping out of the credential directory.
    InvalidSurface {
        /// The name that was refused.
        surface: String,
    },
    /// The platform keychain could not be reached or refused the operation.
    Keychain(keychain::KeychainError),
    /// The fallback file could not be written, read, or removed.
    Io(io::Error),
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDataDir => write!(
                f,
                "neither {XDG_DATA_HOME_ENV} nor {HOME_ENV} names a place to keep the client's credential"
            ),
            Self::InvalidSurface { surface } => write!(
                f,
                "{surface:?} is not a surface name: a surface names one file, so it carries no path separator"
            ),
            Self::Keychain(source) => write!(f, "the keychain could not answer: {source}"),
            Self::Io(source) => write!(f, "the credential file could not be used: {source}"),
        }
    }
}

impl std::error::Error for CredentialError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoDataDir | Self::InvalidSurface { .. } => None,
            Self::Keychain(source) => Some(source),
            Self::Io(source) => Some(source),
        }
    }
}

/// Keep `token` for `surface`, and answer with the backend that took it.
///
/// The keychain is tried first, and the fallback file is used only when no
/// keychain tool is on the machine.
///
/// # Errors
///
/// Returns [`CredentialError::InvalidSurface`] for a surface name that is not a
/// file name, [`CredentialError::Keychain`] when a tool is there and refuses,
/// [`CredentialError::NoDataDir`] when there is nowhere to keep the fallback,
/// and [`CredentialError::Io`] when the file cannot be written.
pub fn store(surface: &str, token: &str) -> Result<Backend, CredentialError> {
    usable_surface(surface)?;
    if keychain::available().is_some() {
        keychain::store(surface, token).map_err(CredentialError::Keychain)?;
        return Ok(Backend::Keychain);
    }
    let path = token_path(surface)?;
    write_private_file(&path, token)?;
    Ok(Backend::File(path))
}

/// Read the credential kept for `surface`, or `None` when there is none.
///
/// Only one backend is consulted: the keychain when a tool is on the machine,
/// and the file otherwise. A keychain entry that is not there means the surface
/// has no credential here, not that the fallback file should be read.
///
/// # Errors
///
/// Returns the same refusals [`store`] returns, minus the write failures.
pub fn load(surface: &str) -> Result<Option<Stored>, CredentialError> {
    usable_surface(surface)?;
    if keychain::available().is_some() {
        let token = keychain::lookup(surface)
            .map_err(CredentialError::Keychain)?
            .map(|token| token.trim().to_owned());
        return Ok(token.map(|token| Stored {
            token,
            backend: Backend::Keychain,
        }));
    }
    let path = token_path(surface)?;
    let token = match std::fs::read_to_string(&path) {
        Ok(text) => text.trim().to_owned(),
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(CredentialError::Io(source)),
    };
    if token.is_empty() {
        return Ok(None);
    }
    Ok(Some(Stored {
        token,
        backend: Backend::File(path),
    }))
}

/// Remove the credential kept for `surface`, answering with the backend it was
/// removed from, or `None` when there was none.
///
/// # Errors
///
/// Returns the same refusals [`load`] returns.
pub fn remove(surface: &str) -> Result<Option<Backend>, CredentialError> {
    let Some(stored) = load(surface)? else {
        return Ok(None);
    };
    match &stored.backend {
        Backend::Keychain => keychain::remove(surface).map_err(CredentialError::Keychain)?,
        Backend::File(path) => match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(CredentialError::Io(source)),
        },
    }
    Ok(Some(stored.backend))
}

/// The file the fallback credential lives in.
///
/// # Errors
///
/// Returns [`CredentialError::NoDataDir`] when neither a data home nor a home
/// directory is set.
fn token_path(surface: &str) -> Result<PathBuf, CredentialError> {
    usable_surface(surface)?;
    let base = match env_value(XDG_DATA_HOME_ENV) {
        Some(data_home) => PathBuf::from(data_home),
        None => match env_value(HOME_ENV) {
            Some(home) => PathBuf::from(home).join(".local/share"),
            None => return Err(CredentialError::NoDataDir),
        },
    };
    Ok(base
        .join(CREDENTIAL_DIR)
        .join(format!("{surface}.{CREDENTIAL_EXTENSION}")))
}

/// Refuse a surface name that would not be one file name.
///
/// The surface arrives from the identity vocabulary, where it is a short name,
/// but the store is reachable with any string, and a name carrying a path
/// separator would write outside the directory the credential belongs in.
fn usable_surface(surface: &str) -> Result<(), CredentialError> {
    let valid = !surface.is_empty()
        && surface != "."
        && surface != ".."
        && !surface.starts_with('.')
        && surface.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        });
    if valid {
        return Ok(());
    }
    Err(CredentialError::InvalidSurface {
        surface: surface.to_owned(),
    })
}

/// Write `token` to `path` under the private-path rule the socket and the state
/// database use: the directory at 700, the file at 600.
#[cfg(unix)]
fn write_private_file(path: &Path, token: &str) -> Result<(), CredentialError> {
    use std::os::unix::fs::OpenOptionsExt as _;

    let directory = path.parent().ok_or_else(|| {
        CredentialError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the credential path has no directory to keep it in",
        ))
    })?;
    transport::ensure_private_dir(directory, CREDENTIAL_DIR_MODE).map_err(CredentialError::Io)?;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(CREDENTIAL_FILE_MODE)
        .open(path)
        .and_then(|mut file| {
            use std::io::Write as _;

            file.write_all(token.as_bytes())
        })
        .map_err(CredentialError::Io)?;
    // The file may have existed with a wider mode, so the rule is applied after
    // the write as well.
    transport::restrict_file(path, CREDENTIAL_FILE_MODE).map_err(CredentialError::Io)
}

/// Write `token` to `path`, creating the directory that holds it.
///
/// A platform without the Unix access rule relies on the file system's own
/// rule, the way the state store does.
#[cfg(not(unix))]
fn write_private_file(path: &Path, token: &str) -> Result<(), CredentialError> {
    let directory = path.parent().ok_or_else(|| {
        CredentialError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the credential path has no directory to keep it in",
        ))
    })?;
    std::fs::create_dir_all(directory).map_err(CredentialError::Io)?;
    std::fs::write(path, token).map_err(CredentialError::Io)
}

/// An environment variable's value when it is set and not empty.
fn env_value(variable: &str) -> Option<std::ffi::OsString> {
    let value = std::env::var_os(variable)?;
    if value.is_empty() {
        return None;
    }
    Some(value)
}
