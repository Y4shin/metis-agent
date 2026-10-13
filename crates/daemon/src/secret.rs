//! The preshared key's configured form: where the daemon reads its pairing
//! secret from.
//!
//! One string names the source, with an explicit prefix: `literal:`, `env:`,
//! `keystore:`, or `file:`. A value with no prefix is a literal, because that
//! is what an operator writes when the key is short and the configuration file
//! is already private. The reference never holds the secret it names: a
//! [`Secret`] is resolved on demand, keeps itself out of `Debug` output, and is
//! handed to exactly one comparison.

use std::fmt;
use std::path::{Path, PathBuf};

use api_contracts::keychain;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

/// The environment variable that carries the daemon's preshared key as a
/// [`SecretRef`].
pub const PRESHARED_KEY_ENV: &str = "METIS_PRESHARED_KEY";

/// The prefix naming a literal value.
const LITERAL_PREFIX: &str = "literal:";

/// The prefix naming an environment variable.
const ENV_PREFIX: &str = "env:";

/// The prefix naming a keychain account.
const KEYSTORE_PREFIX: &str = "keystore:";

/// The prefix naming a secret file.
const FILE_PREFIX: &str = "file:";

/// Where a secret comes from.
///
/// A bare value is a [`Literal`](SecretRef::Literal), so the shortest form is
/// also the one with no reference to resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretRef {
    /// The secret is written in the configuration itself.
    Literal(String),
    /// The secret is the value of an environment variable.
    Env(String),
    /// The secret is filed in the platform keychain under this account. The
    /// service name is the one `api_contracts::keychain` fixes.
    Keystore(String),
    /// The secret is the contents of a file only its owner may read.
    File(PathBuf),
}

impl SecretRef {
    /// Read a reference from one configured string.
    ///
    /// The prefixes are matched literally and case-sensitively; anything else,
    /// including a value that happens to contain a colon, is a literal. This is
    /// infallible: a reference naming an empty variable, a missing file, or an
    /// account with no entry resolves to a refusal, not to a parse error.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        if let Some(value) = text.strip_prefix(LITERAL_PREFIX) {
            Self::Literal(value.to_owned())
        } else if let Some(variable) = text.strip_prefix(ENV_PREFIX) {
            Self::Env(variable.to_owned())
        } else if let Some(account) = text.strip_prefix(KEYSTORE_PREFIX) {
            Self::Keystore(account.to_owned())
        } else if let Some(path) = text.strip_prefix(FILE_PREFIX) {
            Self::File(PathBuf::from(path))
        } else {
            Self::Literal(text.to_owned())
        }
    }

    /// Resolve the reference to the secret it names.
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::Unresolved`] when there is no secret to read (an
    /// unset or empty variable, a keychain without an entry, or a platform with
    /// no keychain tool), [`SecretError::Unreadable`] when the source exists
    /// but cannot be read, and [`SecretError::TooPermissive`] when a secret file
    /// is readable by group or others.
    pub fn resolve(&self) -> Result<Secret, SecretError> {
        match self {
            Self::Literal(value) => Secret::checked(value.clone()),
            Self::Env(variable) => {
                let value = env_value(variable).ok_or(SecretError::Unresolved)?;
                let value = value
                    .into_string()
                    .map_err(|_| SecretError::Unreadable(UnreadableSource::Env))?;
                Secret::checked(value)
            }
            Self::Keystore(account) => match keychain::lookup(account) {
                Ok(Some(secret)) => Secret::checked(secret.trim().to_owned()),
                Ok(None) => Err(SecretError::Unresolved),
                // A tool that is not on the machine is not a keychain the
                // operator configured: there is nothing to read at all. A tool
                // that ran and refused is a keychain that exists and cannot be
                // read, which is a different refusal.
                Err(keychain::KeychainError::Spawn { .. }) => Err(SecretError::Unresolved),
                Err(_) => Err(SecretError::Unreadable(UnreadableSource::Keychain)),
            },
            Self::File(path) => resolve_file(path),
        }
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal(_) => f.write_str("a literal"),
            Self::Env(variable) => write!(f, "the variable {variable}"),
            Self::Keystore(account) => write!(f, "the keychain account {account}"),
            Self::File(path) => write!(f, "the file {}", path.display()),
        }
    }
}

/// The daemon's configured preshared key, resolved.
///
/// [`PRESHARED_KEY_ENV`] carries the reference; an absent or empty variable
/// means pairing is not configured, which is a refusal rather than an empty
/// key.
///
/// # Errors
///
/// Returns the same refusals [`SecretRef::resolve`] returns, with
/// [`SecretError::Unresolved`] for an unset variable.
pub fn resolve_preshared_key() -> Result<Secret, SecretError> {
    let reference = env_value(PRESHARED_KEY_ENV)
        .ok_or(SecretError::Unresolved)?
        .into_string()
        .map_err(|_| SecretError::Unreadable(UnreadableSource::Env))?;
    SecretRef::parse(&reference).resolve()
}

/// Which source could not be read.
///
/// It exists so a refusal can say what kind of source failed without a caller
/// matching on error text. It carries no secret and no path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnreadableSource {
    /// An environment variable's value is not valid UTF-8.
    Env,
    /// The platform keychain's tool refused the lookup.
    Keychain,
    /// The secret file could not be read.
    File,
}

impl fmt::Display for UnreadableSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Env => f.write_str("the configured variable"),
            Self::Keychain => f.write_str("the platform keychain"),
            Self::File => f.write_str("the secret file"),
        }
    }
}

/// A resolved secret.
///
/// The value is private and never reaches a log line: `Debug` prints a redacted
/// form, and the caller that needs the value asks for it by name. The daemon
/// compares the preshared key with it in constant time and keeps no copy of it
/// anywhere else.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// A secret from a resolved value, refusing an empty one.
    ///
    /// An empty secret would let a caller pair by presenting an empty key, so
    /// it is treated as no secret at all.
    fn checked(value: String) -> Result<Self, SecretError> {
        if value.is_empty() {
            return Err(SecretError::Unresolved);
        }
        Ok(Self(value))
    }

    /// The secret's value.
    ///
    /// The name is deliberately explicit: a secret a caller reads is a secret
    /// that can be printed, so the call site says so.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// The secret could not be resolved.
///
/// The enum is non-exhaustive: a later source adds its own failures without
/// breaking a caller that matches on it. No variant carries the secret's value.
#[derive(Debug)]
#[non_exhaustive]
pub enum SecretError {
    /// There is no secret to read: the variable is unset or empty, the keychain
    /// has no entry, or the platform has no keychain tool.
    Unresolved,
    /// The source exists but could not be read.
    Unreadable(UnreadableSource),
    /// A secret file is readable by group or others, so it is refused rather
    /// than read.
    TooPermissive {
        /// The file that is too permissive.
        path: PathBuf,
        /// The mode it carries, as `st_mode & 0o777`.
        mode: u32,
    },
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unresolved => write!(
                f,
                "no preshared key is configured: {PRESHARED_KEY_ENV} names a source, or that source has nothing to read"
            ),
            Self::Unreadable(source) => write!(f, "{source} could not be read"),
            Self::TooPermissive { path, mode } => write!(
                f,
                "the secret file {} is readable by group or others (mode {mode:o}); restrict it to its owner",
                path.display()
            ),
        }
    }
}

impl std::error::Error for SecretError {}

/// Read a secret file, refusing one that group or others can read.
///
/// The permission check happens before the file is read, and a file that is not
/// there is refused with the same "could not be read" answer as one that is
/// there but unreadable: in both cases the configured source has no secret to
/// offer.
fn resolve_file(path: &Path) -> Result<Secret, SecretError> {
    if std::fs::metadata(path).is_ok_and(|metadata| !owner_only(&metadata)) {
        let mode = std::fs::metadata(path)
            .map(|metadata| mode_of(&metadata))
            .unwrap_or_default();
        return Err(SecretError::TooPermissive {
            path: path.to_path_buf(),
            mode,
        });
    }
    let contents = std::fs::read_to_string(path)
        .map_err(|_| SecretError::Unreadable(UnreadableSource::File))?;
    // A file's trailing newline is an editor's, not part of the key.
    Secret::checked(contents.trim_end_matches(['\n', '\r']).to_owned())
}

/// Whether a file's mode grants nothing to group or others.
#[cfg(unix)]
fn owner_only(metadata: &std::fs::Metadata) -> bool {
    mode_of(metadata) & 0o077 == 0
}

/// The mode bits a file carries.
#[cfg(unix)]
fn mode_of(metadata: &std::fs::Metadata) -> u32 {
    metadata.permissions().mode() & 0o777
}

/// A platform without POSIX modes leaves the file's access rule to the file
/// system, which is what `windows-named-pipe` mirrors for the socket.
#[cfg(not(unix))]
fn owner_only(_metadata: &std::fs::Metadata) -> bool {
    true
}

/// The mode reported on a platform without POSIX modes: there is none.
#[cfg(not(unix))]
fn mode_of(_metadata: &std::fs::Metadata) -> u32 {
    0
}

/// An environment variable's value when it is set and not empty.
///
/// The value is kept as the operating system reported it, so a key that is not
/// valid UTF-8 is reported as unreadable rather than lossily truncated into a
/// different secret.
fn env_value(variable: &str) -> Option<std::ffi::OsString> {
    let value = std::env::var_os(variable)?;
    if value.is_empty() {
        return None;
    }
    Some(value)
}
