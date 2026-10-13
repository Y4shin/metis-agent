//! The platform keychain, reached through its command line tool.
//!
//! The daemon reads its preshared key from a keychain when it is configured to,
//! and the client keeps its per-surface token in one when it can. Both need the
//! same three operations and the same tool, and this crate is the only one both
//! depend on, so the shell-out lives here once instead of in two crates that
//! would drift.
//!
//! The tool is `secret-tool` on Linux and `security` on macOS, probed on
//! [`PATH`](std::env::split_paths) rather than installed as a dependency: a
//! keychain crate would add a platform stack for three command lines. Where
//! neither tool exists, [`available`] answers [`None`] and the caller keeps its
//! own fallback. A tool that exists but fails is an error, never a silent
//! fallback: a credential that was stored but cannot be read must not look like
//! a credential that was never stored.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// The service name every Metis credential is filed under. The account is the
/// surface's name.
pub const SERVICE_NAME: &str = "metis-daemon";

/// The command line tool this platform keeps its credentials in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeychainTool {
    /// `secret-tool`, the libsecret command line tool, on Linux.
    SecretTool,
    /// `security`, the macOS keychain command line tool.
    Security,
}

impl KeychainTool {
    /// The tool's own name, which is also what `PATH` is searched for.
    #[must_use]
    pub fn command(self) -> &'static str {
        match self {
            Self::SecretTool => "secret-tool",
            Self::Security => "security",
        }
    }

    /// The tool this platform has, when it has one at all.
    fn for_platform() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::Security)
        } else if cfg!(target_os = "linux") {
            Some(Self::SecretTool)
        } else {
            None
        }
    }

    /// Run the tool with `args`, optionally writing `input` to its standard
    /// input, and return its standard output and exit status.
    fn run(self, args: &[&str], input: Option<&str>) -> Result<Output, KeychainError> {
        let mut command = Command::new(self.command());
        command
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|source| KeychainError::Spawn {
            tool: self.command(),
            source,
        })?;
        if let Some(input) = input {
            use std::io::Write as _;

            let stdin = child
                .stdin
                .take()
                .expect("stdin is piped when input is given");
            let mut stdin = stdin;
            stdin
                .write_all(input.as_bytes())
                .and_then(|()| stdin.write_all(b"\n"))
                .map_err(|source| KeychainError::Spawn {
                    tool: self.command(),
                    source,
                })?;
            // The write ends when `stdin` is dropped here, so the tool sees the
            // end of its input.
        }
        let output = child
            .wait_with_output()
            .map_err(|source| KeychainError::Spawn {
                tool: self.command(),
                source,
            })?;
        Ok(Output {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// What one run of the tool reported.
struct Output {
    /// The exit status, or `None` when a signal ended the process.
    status: Option<i32>,
    stdout: String,
    stderr: String,
}

/// The keychain tool this platform offers, or [`None`] when it has none.
///
/// The tool is resolved by searching `PATH`, so a caller that has installed one
/// gets it and a caller without one learns there is none. This is the only
/// question that decides between a keychain and a fallback: a tool that is
/// found but fails belongs to [`lookup`], [`store`] and [`remove`].
#[must_use]
pub fn available() -> Option<KeychainTool> {
    let tool = KeychainTool::for_platform()?;
    on_path(tool.command()).then_some(tool)
}

/// Read the secret filed under `account`.
///
/// # Errors
///
/// Returns [`KeychainError::Spawn`] when the tool cannot be started and
/// [`KeychainError::Refused`] when it runs and refuses the lookup, which is how
/// a locked or unreachable keychain answers. An entry that is simply not there
/// is `Ok(None)`, which is what both tools report with a silent non-zero exit.
pub fn lookup(account: &str) -> Result<Option<String>, KeychainError> {
    let Some(tool) = available() else {
        return Err(KeychainError::Spawn {
            tool: "the platform keychain tool",
            source: io::Error::new(
                io::ErrorKind::NotFound,
                "no keychain tool is on PATH for this platform",
            ),
        });
    };
    let output = match tool {
        KeychainTool::SecretTool => tool.run(
            &["lookup", "service", SERVICE_NAME, "account", account],
            None,
        )?,
        KeychainTool::Security => tool.run(
            &[
                "find-generic-password",
                "-s",
                SERVICE_NAME,
                "-a",
                account,
                "-w",
            ],
            None,
        )?,
    };
    if output.status == Some(0) {
        return Ok(Some(output.stdout));
    }
    // `secret-tool` exits 1 with nothing on standard error when there is no
    // matching item, and 44 is `security`'s "no such item". Anything else is
    // the tool refusing to answer, which the caller must not read as absence.
    if output.stderr.is_empty() {
        return Ok(None);
    }
    Err(KeychainError::Refused {
        tool: tool.command(),
        status: output.status,
        message: output.stderr,
    })
}

/// File `secret` under `account`, replacing any secret already there.
///
/// # Errors
///
/// Returns [`KeychainError::Spawn`] when the tool cannot be started and
/// [`KeychainError::Refused`] when it runs and refuses the write.
pub fn store(account: &str, secret: &str) -> Result<(), KeychainError> {
    let Some(tool) = available() else {
        return Err(KeychainError::Spawn {
            tool: "the platform keychain tool",
            source: io::Error::new(
                io::ErrorKind::NotFound,
                "no keychain tool is on PATH for this platform",
            ),
        });
    };
    let output = match tool {
        // The secret travels on standard input, so it never appears in the
        // process's argument list.
        KeychainTool::SecretTool => tool.run(
            &[
                "store",
                &format!("--label={SERVICE_NAME}"),
                "service",
                SERVICE_NAME,
                "account",
                account,
            ],
            Some(secret),
        )?,
        // `security` takes the secret as an argument; `-U` replaces an existing
        // item instead of refusing.
        KeychainTool::Security => tool.run(
            &[
                "add-generic-password",
                "-U",
                "-s",
                SERVICE_NAME,
                "-a",
                account,
                "-w",
                secret,
            ],
            None,
        )?,
    };
    if output.status == Some(0) {
        return Ok(());
    }
    Err(KeychainError::Refused {
        tool: tool.command(),
        status: output.status,
        message: output.stderr,
    })
}

/// Remove the secret filed under `account`.
///
/// Removing an account that has no entry is a refusal by both tools, and the
/// caller that removes a credential reports absence itself rather than reading
/// it out of this call.
///
/// # Errors
///
/// Returns [`KeychainError::Spawn`] when the tool cannot be started and
/// [`KeychainError::Refused`] when it runs and refuses the removal.
pub fn remove(account: &str) -> Result<(), KeychainError> {
    let Some(tool) = available() else {
        return Err(KeychainError::Spawn {
            tool: "the platform keychain tool",
            source: io::Error::new(
                io::ErrorKind::NotFound,
                "no keychain tool is on PATH for this platform",
            ),
        });
    };
    let output = match tool {
        KeychainTool::SecretTool => tool.run(
            &["clear", "service", SERVICE_NAME, "account", account],
            None,
        )?,
        KeychainTool::Security => tool.run(
            &["delete-generic-password", "-s", SERVICE_NAME, "-a", account],
            None,
        )?,
    };
    if output.status == Some(0) {
        return Ok(());
    }
    Err(KeychainError::Refused {
        tool: tool.command(),
        status: output.status,
        message: output.stderr,
    })
}

/// The keychain could not be reached or refused the operation.
///
/// The enum is non-exhaustive: a later platform adds its own failures without
/// breaking a caller that matches on it.
#[derive(Debug)]
#[non_exhaustive]
pub enum KeychainError {
    /// The tool could not be started, which includes the case of no tool being
    /// on `PATH` at all.
    Spawn {
        /// The tool that was to be run.
        tool: &'static str,
        /// What the process spawn reported.
        source: io::Error,
    },
    /// The tool ran and refused the operation, with what it said.
    Refused {
        /// The tool that refused.
        tool: &'static str,
        /// Its exit status, or `None` when a signal ended it.
        status: Option<i32>,
        /// Its standard error, trimmed.
        message: String,
    },
}

impl fmt::Display for KeychainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { tool, source } => {
                write!(f, "the keychain tool {tool} could not be started: {source}")
            }
            Self::Refused {
                tool,
                status,
                message,
            } => {
                let status = status.map_or_else(|| "a signal".to_owned(), |code| code.to_string());
                write!(
                    f,
                    "the keychain tool {tool} refused the operation (exit {status}): {message}"
                )
            }
        }
    }
}

impl std::error::Error for KeychainError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            Self::Refused { .. } => None,
        }
    }
}

/// Whether `name` is an executable file in one of `PATH`'s directories.
fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| is_executable(&directory.join(name)))
}

/// Whether `path` names a file this user may execute.
#[cfg(unix)]
fn is_executable(path: &PathBuf) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Whether `path` names a file.
#[cfg(not(unix))]
fn is_executable(path: &PathBuf) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}
