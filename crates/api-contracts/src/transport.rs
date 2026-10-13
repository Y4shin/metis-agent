//! The transport: one serve loop and one connector shape, shared by every
//! front-end.
//!
//! `serve_stream` is the accept-loop body. It is generic over any asynchronous
//! bidirectional stream, so a Unix socket, a named pipe, a TCP socket, or an
//! in-memory duplex pair all reach the same Connect service without a second
//! serve loop. `serve_uds` and `uds_transport` are the Unix front-ends;
//! `serve_tcp` (`serve_tcp_listener` when the caller must report the bound
//! address) and `tcp_transport` are the TCP ones, with `serve_tcp_service` and
//! `serve_tcp_listener_service` as the service-taking twins a caller mounts an
//! interceptor chain through; the later transport tickets add their own
//! listener and connector against the same shapes.
//!
//! The remote front-end is off unless the operator names a bind address:
//! `REMOTE_ADDR_ENV` carries an `IP:port` literal, `resolve_remote_addr` reads
//! it, and there is no default address to fall back to.
//!
//! The local front-end carries its access control itself, because the
//! operating system enforces it: `serve_uds` creates the socket's directory
//! with the mode and group this daemon is configured with, and refuses a
//! directory it did not create to spec rather than changing it. A local client
//! therefore needs no credential; `identity` carries the declaration it makes
//! instead. The state store uses the same private-path rule for the directory
//! and the file it owns, through `ensure_private_dir` and `restrict_file`.
//!
//! This module is the only place that knows how to serve and connect a stream.
//! Generated code stays mounted and untouched.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use connectrpc::{ConnectRpcService, Router};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite};

/// Serve a Connect router over any asynchronous bidirectional stream.
///
/// The stream is wrapped for hyper, the router is built into a
/// [`ConnectRpcService`] once, and a `hyper_util` auto connection drives
/// HTTP/1.1 or HTTP/2 until the stream closes. The caller owns the listener and
/// the accept loop; this function owns one connection.
///
/// The router is taken through `Into<Arc<Router>>` because `Router` is not
/// `Clone` in connectrpc 0.9.1: a listener builds the router once, wraps it in
/// an `Arc`, and hands each accepted stream a clone of that `Arc`. A one-shot
/// caller passes the router directly.
///
/// # Errors
///
/// Returns the connection's terminal error as an [`io::Error`]. A client that
/// disconnects ends the connection without an error on the server side.
pub async fn serve_stream<S>(stream: S, router: impl Into<Arc<Router>>) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    serve_connection(stream, ConnectRpcService::from_arc(router.into())).await
}

/// Serve an already built Connect service over one connection.
///
/// This is the same body [`serve_stream`] uses. It exists for the caller that
/// mounted an interceptor chain: `connectrpc` attaches interceptors to
/// [`ConnectRpcService`], not to a [`Router`], so an authorization policy can
/// only be served through the service it was installed on.
async fn serve_connection<S>(stream: S, service: ConnectRpcService<Router>) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let builder = AutoBuilder::new(TokioExecutor::new());
    builder
        .serve_connection(TokioIo::new(stream), TowerToHyperService::new(service))
        .await
        .map_err(io::Error::other)
}

/// Serve a Connect router over a Unix domain socket.
///
/// A stale socket file is removed first, so a crashed listener does not block
/// the next start. Each accepted stream is handed to [`serve_stream`] on its
/// own task.
///
/// # Errors
///
/// Returns the bind or accept error. Accepting runs until the task is dropped.
#[cfg(unix)]
pub async fn serve_uds(path: &std::path::Path, router: Router) -> io::Result<()> {
    serve_uds_service(path, ConnectRpcService::from_arc(Arc::new(router))).await
}

/// Serve an already built Connect service over a Unix domain socket.
///
/// The access control is the same one [`serve_uds`] applies, because it is the
/// same accept loop: a caller reaches this instead of `serve_uds` when it
/// installed an interceptor chain, which lives on the service rather than on
/// the router.
///
/// # Errors
///
/// Returns the bind or accept error. Accepting runs until the task is dropped.
#[cfg(unix)]
pub async fn serve_uds_service(
    path: &std::path::Path,
    service: ConnectRpcService<Router>,
) -> io::Result<()> {
    // The access control is applied here rather than by the caller, so every
    // path that binds a local socket carries it. The socket's directory is
    // created to spec first, then the stale socket file is removed, then the
    // socket is bound and the same rule applied to it: on Linux, connecting to
    // a Unix socket also checks write permission on the socket file.
    let acl = PathAcl::from_env().map_err(io::Error::other)?;
    ensure_dir(socket_dir(path).map_err(io::Error::other)?, &acl).map_err(io::Error::other)?;
    let _ = std::fs::remove_file(path);
    let listener = tokio::net::UnixListener::bind(path)?;
    apply_acl(path, &acl).map_err(io::Error::other)?;
    let service = Arc::new(service);
    loop {
        let (stream, _) = listener.accept().await?;
        let service = Arc::clone(&service);
        tokio::spawn(async move {
            if let Err(err) = serve_connection(stream, (*service).clone()).await {
                eprintln!("local connection ended with error: {err}");
            }
        });
    }
}

/// Build an h2c client connection over a Unix domain socket, lazily on first
/// use.
///
/// The connector is a plain `tower::service_fn` that opens the socket, so a
/// later transport supplies its own connector against the same generated client
/// shape. The returned connection is an `Http2Connection`; wrap it with
/// `shared` to obtain a `ClientTransport` for the generated client.
#[cfg(unix)]
pub fn uds_transport(path: impl Into<std::path::PathBuf>) -> connectrpc::client::Http2Connection {
    let path = path.into();
    let connector = tower::service_fn(move |_uri: http::Uri| {
        let path = path.clone();
        async move {
            let stream = tokio::net::UnixStream::connect(path).await?;
            Ok::<_, io::Error>(TokioIo::new(stream))
        }
    });
    connectrpc::client::Http2Connection::lazy_with_connector(connector, local_authority())
}

/// Serve a Connect router over a TCP socket.
///
/// The listener is bound here, so a requested port of `0` is resolved by the
/// operating system. Each accepted stream is handed to the shared connection
/// body on its own task, carrying one service built from the router. The
/// transport is plaintext: TLS and the proxy trust rule belong to their own
/// tickets.
///
/// # Errors
///
/// Returns the bind or accept error. Accepting runs until the task is dropped.
pub async fn serve_tcp(addr: SocketAddr, router: Router) -> io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    serve_tcp_listener(listener, router).await
}

/// Serve a Connect router over an already bound TCP listener.
///
/// This is the same accept loop [`serve_tcp_service`] runs, not a second one: it
/// builds the service once and hands each accepted stream the same one. The
/// split exists for a caller that must report the address the operating system
/// assigned to a port of `0`: it binds first, reads `local_addr`, and hands the
/// listener on. `client::skeleton::serve_remote` is that caller.
///
/// # Errors
///
/// Returns the accept error. Accepting runs until the task is dropped.
pub async fn serve_tcp_listener(listener: tokio::net::TcpListener, router: Router) -> io::Result<()> {
    serve_tcp_listener_service(listener, ConnectRpcService::from_arc(Arc::new(router))).await
}

/// Serve an already built Connect service over a TCP socket.
///
/// The listener is bound here, so a requested port of `0` is resolved by the
/// operating system, exactly as [`serve_tcp`] resolves it. The transport is
/// plaintext: TLS and the proxy trust rule belong to their own tickets.
///
/// # Errors
///
/// Returns the bind or accept error. Accepting runs until the task is dropped.
pub async fn serve_tcp_service(
    addr: SocketAddr,
    service: ConnectRpcService<Router>,
) -> io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    serve_tcp_listener_service(listener, service).await
}

/// Serve an already built Connect service over an already bound TCP listener.
///
/// The accept loop is the same one [`serve_tcp_listener`] runs, and each
/// accepted stream reaches the same private connection body [`serve_stream`]
/// runs. A caller reaches this instead of `serve_tcp_listener` when it mounted
/// an interceptor chain, which lives on the service rather than on the router:
/// a remote listener that enforces a credential policy mounts a service.
///
/// # Errors
///
/// Returns the accept error. Accepting runs until the task is dropped.
pub async fn serve_tcp_listener_service(
    listener: tokio::net::TcpListener,
    service: ConnectRpcService<Router>,
) -> io::Result<()> {
    let service = Arc::new(service);
    loop {
        let (stream, _) = listener.accept().await?;
        let service = Arc::clone(&service);
        tokio::spawn(async move {
            if let Err(err) = serve_connection(stream, (*service).clone()).await {
                eprintln!("remote connection ended with error: {err}");
            }
        });
    }
}

/// Build an h2c client connection over TCP, lazily on first use.
///
/// The connector is the shape [`uds_transport`] uses: a plain
/// `tower::service_fn` that opens the socket. The authority is `http://<addr>`.
pub fn tcp_transport(addr: SocketAddr) -> connectrpc::client::Http2Connection {
    let connector = tower::service_fn(move |_uri: http::Uri| async move {
        let stream = tokio::net::TcpStream::connect(addr).await?;
        Ok::<_, io::Error>(TokioIo::new(stream))
    });
    connectrpc::client::Http2Connection::lazy_with_connector(connector, remote_authority(addr))
}

/// The `:authority` value used for a TCP transport to `addr`.
///
/// The connector and the generated client both name it, so a request carries
/// the address the connection was opened to.
///
/// # Panics
///
/// Never in practice: a socket address literal is a valid authority.
#[must_use]
pub fn remote_authority(addr: SocketAddr) -> http::Uri {
    format!("http://{addr}")
        .parse()
        .expect("a socket address is a valid authority")
}

/// The `:authority` value used for the local IPC transport. The server does not
/// validate it.
///
/// # Panics
///
/// Never in practice: the literal parses.
#[must_use]
pub fn local_authority() -> http::Uri {
    "http://localhost".parse().expect("static authority")
}

/// The mode a socket and its directory carry when no group is configured:
/// only the daemon's own user may reach it.
pub const SOCKET_MODE_PRIVATE: u32 = 0o700;

/// The mode they carry when a group is configured: the daemon's user and that
/// group may reach it, nobody else.
pub const SOCKET_MODE_GROUP: u32 = 0o770;

/// The environment variable that names the group the socket's directory and
/// the socket file are shared with.
///
/// The group is a name from the host's group database, and the daemon's user
/// must belong to it; an unknown name is a typed error rather than a silent
/// fall back to the user's primary group. An absent or empty value means no
/// group, and the private mode.
pub const SOCKET_GROUP_ENV: &str = "METIS_SOCKET_GROUP";

/// The access control a Unix socket path is created with.
///
/// The access control a private path is created with: the mode it carries, and
/// the group it is shared with when one is configured.
///
/// The owner is never configured: the daemon owns what it creates, as its own
/// user, and no privileged step is involved.
#[cfg(unix)]
#[derive(Debug, Clone, Copy)]
struct PathAcl {
    /// The mode of the path.
    mode: u32,
    /// The configured group, when there is one.
    group: Option<libc::gid_t>,
}

#[cfg(unix)]
impl PathAcl {
    /// The rule for a path nothing is configured for: `mode`, in this process's
    /// own group.
    fn private(mode: u32) -> Self {
        Self { mode, group: None }
    }

    /// Read the socket's configured access control from the environment.
    ///
    /// An absent or empty [`SOCKET_GROUP_ENV`] is the private mode with no
    /// group; a name resolves to the host's group database.
    fn from_env() -> Result<Self, ConfigError> {
        let Some(value) = env_value(SOCKET_GROUP_ENV) else {
            return Ok(Self {
                mode: SOCKET_MODE_PRIVATE,
                group: None,
            });
        };
        let group = resolve_group(&value).ok_or_else(|| ConfigError::UnknownSocketGroup {
            variable: SOCKET_GROUP_ENV,
            value: value.clone(),
        })?;
        Ok(Self {
            mode: SOCKET_MODE_GROUP,
            group: Some(group),
        })
    }
}

/// Create or accept the private directory at `dir` with `mode`, on the same
/// rule [`ensure_socket_dir`] applies to a socket's directory.
///
/// The directory is created with `mode` already applied, so it is never briefly
/// wider than the rule allows, and `mode` is applied again afterwards because
/// `umask` can narrow a creation mode. A pre-existing directory is refused,
/// never modified, when it is a symlink, is not a directory, is not owned by
/// this process's user, or carries a mode wider than `mode`.
///
/// Ownership is never changed and no group is ever applied here: the caller
/// owns what it creates, as its own user.
///
/// # Errors
///
/// Returns the creation, inspection, or permission failure. The reason is the
/// error's own message, so a caller reports it rather than parsing it.
#[cfg(unix)]
pub fn ensure_private_dir(dir: &std::path::Path, mode: u32) -> io::Result<()> {
    ensure_dir(dir, &PathAcl::private(mode))
}

/// Apply `mode` to the existing file at `path`: the file half of the same rule
/// [`ensure_private_dir`] applies to a directory.
///
/// # Errors
///
/// Returns the permission failure.
#[cfg(unix)]
pub fn restrict_file(path: &std::path::Path, mode: u32) -> io::Result<()> {
    apply_acl(path, &PathAcl::private(mode))
}

/// Create the socket's directory with the configured ownership and mode, or
/// accept one that already carries them.
///
/// The directory is the socket path's parent. It is created with the mode
/// already applied, so it is never briefly wider than the rule allows, and the
/// mode is applied again afterwards because `umask` can narrow a creation mode.
///
/// A pre-existing directory is refused, never modified, when it is a symlink,
/// is not a directory, is not owned by this process's user, or carries a mode
/// wider than the rule allows. A directory that passes is used and has the
/// configured mode and group applied, which is how a daemon restarted under a
/// different group setting reaches its own directory.
///
/// # Errors
///
/// Returns [`ConfigError::SocketAcl`] when the directory cannot be created,
/// inspected, or used, and [`ConfigError::UnknownSocketGroup`] when
/// [`SOCKET_GROUP_ENV`] names no group on this host.
#[cfg(unix)]
pub fn ensure_socket_dir(socket: &std::path::Path) -> Result<(), ConfigError> {
    let acl = PathAcl::from_env()?;
    let dir = socket_dir(socket).map_err(|source| ConfigError::SocketAcl {
        path: socket.to_path_buf(),
        source,
    })?;
    ensure_dir(dir, &acl).map_err(|source| ConfigError::SocketAcl {
        path: dir.to_path_buf(),
        source,
    })
}

/// The directory a socket path lives in.
#[cfg(unix)]
fn socket_dir(socket: &std::path::Path) -> io::Result<&std::path::Path> {
    socket
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the socket path has no directory to own",
            )
        })
}

/// Create or accept the directory at `dir` under one access control rule.
#[cfg(unix)]
fn ensure_dir(dir: &std::path::Path, acl: &PathAcl) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    match std::fs::symlink_metadata(dir) {
        Ok(metadata) => check_dir(dir, &metadata, acl)?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(acl.mode)
                .create(dir)?;
        }
        Err(source) => return Err(source),
    }
    apply_acl(dir, acl)
}

/// Refuse a pre-existing directory the daemon may not use, without touching it.
#[cfg(unix)]
fn check_dir(dir: &std::path::Path, metadata: &std::fs::Metadata, acl: &PathAcl) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let refuse = |reason: String| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("cannot use {}: {reason}", dir.display()),
        )
    };
    if metadata.file_type().is_symlink() {
        return Err(refuse(
            "it is a symlink, which is never followed".to_owned(),
        ));
    }
    if !metadata.is_dir() {
        return Err(refuse("it is not a directory".to_owned()));
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(refuse(format!(
            "it is owned by user {}, not by this user",
            metadata.uid()
        )));
    }
    let mode = metadata.mode() & 0o7777;
    if mode & !acl.mode != 0 {
        return Err(refuse(format!(
            "it is mode {mode:o}, wider than the allowed {:o}",
            acl.mode
        )));
    }
    Ok(())
}

/// Apply the configured mode, and the configured group when there is one.
#[cfg(unix)]
fn apply_acl(path: &std::path::Path, acl: &PathAcl) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(acl.mode))?;
    if let Some(group) = acl.group {
        let raw = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the path holds an interior NUL byte",
            )
        })?;
        // `uid_t::MAX` leaves the owner alone: the daemon owns what it
        // created, and only the group is ever configured.
        let code = unsafe { libc::chown(raw.as_ptr(), libc::uid_t::MAX, group) };
        if code != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// A group name from the host's group database, through the reentrant lookup
/// so a concurrent name lookup cannot race a shared static entry.
#[cfg(unix)]
fn resolve_group(name: &str) -> Option<libc::gid_t> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut buffer = vec![0 as libc::c_char; 1024];
    loop {
        let mut entry: libc::group = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::group = std::ptr::null_mut();
        let code = unsafe {
            libc::getgrnam_r(
                name.as_ptr(),
                &mut entry,
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut result,
            )
        };
        if code == libc::ERANGE {
            if buffer.len() >= 1 << 16 {
                return None;
            }
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if code != 0 || result.is_null() {
            return None;
        }
        return Some(entry.gr_gid);
    }
}

/// An environment variable's value when it is set and not empty.
fn env_value(variable: &str) -> Option<String> {
    let value = std::env::var_os(variable)?;
    if value.is_empty() {
        return None;
    }
    Some(value.to_string_lossy().into_owned())
}

/// The environment variable that names the remote bind address.
///
/// Naming an `IP:port` literal enables the remote listener; an absent or empty
/// value leaves it off. There is no default address, so remote access cannot be
/// enabled by accident.
pub const REMOTE_ADDR_ENV: &str = "METIS_TCP_ADDR";

/// The configuration could not be applied.
///
/// The first configuration error in the repository. It is non-exhaustive
/// because the transport tickets that follow add their own settings.
#[derive(Debug)]
#[non_exhaustive]
pub enum ConfigError {
    /// The variable names something that is not an `IP:port` literal.
    InvalidRemoteAddr {
        /// The variable that was read.
        variable: &'static str,
        /// The value it held.
        value: String,
        /// The address parse failure.
        source: std::net::AddrParseError,
    },
    /// The configured remote bind address could not be bound.
    RemoteBind {
        /// The address that was refused.
        addr: SocketAddr,
        /// The bind failure.
        source: io::Error,
    },
    /// The socket's directory could not be created, is one the daemon may not
    /// use, or could not be given its access control.
    SocketAcl {
        /// The directory that was refused or could not be prepared.
        path: std::path::PathBuf,
        /// The reason, as the operating system reported it.
        source: io::Error,
    },
    /// [`SOCKET_GROUP_ENV`] names no group on this host.
    UnknownSocketGroup {
        /// The variable that was read.
        variable: &'static str,
        /// The value it held.
        value: String,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRemoteAddr {
                variable, value, ..
            } => write!(f, "{variable} is not an `IP:port` literal: {value:?}"),
            Self::RemoteBind { addr, source } => {
                write!(f, "cannot bind the remote listener to {addr}: {source}")
            }
            Self::SocketAcl { path, source } => {
                write!(
                    f,
                    "cannot use {} as the socket directory: {source}",
                    path.display()
                )
            }
            Self::UnknownSocketGroup { variable, value } => {
                write!(f, "{variable} names no group on this host: {value:?}")
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidRemoteAddr { source, .. } => Some(source),
            Self::RemoteBind { source, .. } => Some(source),
            Self::SocketAcl { source, .. } => Some(source),
            Self::UnknownSocketGroup { .. } => None,
        }
    }
}

/// Resolve the remote bind address from the environment.
///
/// An absent or empty [`REMOTE_ADDR_ENV`] is `None`, which is the whole of off
/// by default. An `IP:port` literal is `Some`; anything else, a bare hostname
/// included, is a [`ConfigError`] naming the variable and the value, never a
/// silent disable. The literal is parsed here and bound later, so resolution
/// happens at bind time.
///
/// # Errors
///
/// Returns [`ConfigError::InvalidRemoteAddr`] when the variable holds anything
/// that is not an `IP:port` literal.
pub fn resolve_remote_addr() -> Result<Option<SocketAddr>, ConfigError> {
    let Some(value) = std::env::var_os(REMOTE_ADDR_ENV) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    let value = value.to_string_lossy().into_owned();
    value
        .parse::<SocketAddr>()
        .map(Some)
        .map_err(|source| ConfigError::InvalidRemoteAddr {
            variable: REMOTE_ADDR_ENV,
            value,
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::{REMOTE_ADDR_ENV, resolve_remote_addr};
    use std::ffi::OsString;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// Serialises the environment in this test binary: `std::env` is process
    /// wide and the harness runs the tests on threads.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Holds one value of `METIS_TCP_ADDR` for the duration of a test and
    /// restores the previous value on drop, so a panic cannot leak the value
    /// into another test. The lock it carries serialises every environment
    /// test in this binary.
    struct RemoteAddrEnv {
        previous: Option<OsString>,
        _lock: MutexGuard<'static, ()>,
    }

    impl RemoteAddrEnv {
        fn set(value: &str) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            let previous = std::env::var_os(REMOTE_ADDR_ENV);
            // Edition 2024 makes mutating the process environment unsafe
            // because it is not thread-safe. Every test that touches the
            // variable holds `ENV_LOCK`, so nothing races this write.
            unsafe { std::env::set_var(REMOTE_ADDR_ENV, value) };
            Self {
                previous,
                _lock: lock,
            }
        }

        fn unset() -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            let previous = std::env::var_os(REMOTE_ADDR_ENV);
            // SAFETY: see `set`.
            unsafe { std::env::remove_var(REMOTE_ADDR_ENV) };
            Self {
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for RemoteAddrEnv {
        fn drop(&mut self) {
            // SAFETY: see `set`; the lock is held until this guard is dropped.
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var(REMOTE_ADDR_ENV, value),
                    None => std::env::remove_var(REMOTE_ADDR_ENV),
                }
            }
        }
    }

    /// An absent variable and an empty one both mean "no remote listener",
    /// which is the whole of off by default: there is no address to fall back
    /// to.
    #[test]
    fn an_absent_or_empty_remote_addr_disables_the_listener() {
        let absent = RemoteAddrEnv::unset();
        assert_eq!(
            resolve_remote_addr().expect("an absent variable is not an error"),
            None
        );
        drop(absent);

        let empty = RemoteAddrEnv::set("");
        assert_eq!(
            resolve_remote_addr().expect("an empty value is treated as unset"),
            None
        );
        drop(empty);
    }

    /// Naming an `IP:port` literal is the enablement, for both address
    /// families, and a port of `0` is allowed so the operating system can pick
    /// one at bind time.
    #[test]
    fn an_ip_literal_with_a_port_is_the_bind_address() {
        let env = RemoteAddrEnv::set("127.0.0.1:0");
        assert_eq!(
            resolve_remote_addr().expect("an IPv4 literal with a port parses"),
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        );

        // SAFETY: `env` holds `ENV_LOCK` for the rest of this test.
        unsafe { std::env::set_var(REMOTE_ADDR_ENV, "127.0.0.1:8443") };
        assert_eq!(
            resolve_remote_addr().expect("an IPv4 literal parses"),
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8443))
        );

        // SAFETY: `env` holds `ENV_LOCK` for the rest of this test.
        unsafe { std::env::set_var(REMOTE_ADDR_ENV, "[::1]:8443") };
        assert_eq!(
            resolve_remote_addr().expect("an IPv6 literal parses"),
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 8443))
        );

        drop(env);
    }

    /// Anything that is not an `IP:port` literal is a typed error naming the
    /// variable and the value. Never a silent disable: a misspelled address
    /// must not look like an intentional one.
    #[test]
    fn a_value_that_is_not_an_ip_port_literal_is_a_typed_error() {
        for value in [
            "127.0.0.1",
            "example.com:80",
            "not an address",
            "127.0.0.1:port",
        ] {
            let env = RemoteAddrEnv::set(value);
            let error = resolve_remote_addr().expect_err("a non-literal value is refused");
            let message = error.to_string();
            assert!(
                message.contains(REMOTE_ADDR_ENV),
                "the error names the variable: {message}"
            );
            assert!(
                message.contains(value),
                "the error names the value: {message}"
            );
            drop(env);
        }
    }

    /// The socket ACL, Unix-only like the front-end it guards.
    #[cfg(unix)]
    mod acl {
        use super::ENV_LOCK;
        use crate::transport::{
            ConfigError, SOCKET_GROUP_ENV, SOCKET_MODE_GROUP, SOCKET_MODE_PRIVATE,
            ensure_private_dir, ensure_socket_dir, restrict_file, serve_uds,
        };
        use connectrpc::Router;
        use std::ffi::OsString;
        use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt, symlink};
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{MutexGuard, PoisonError};
        use std::time::Duration;

        /// A temporary directory that removes itself on drop, so the ACL tests
        /// clean up on the success path and on a panic alike.
        struct TempDir {
            path: PathBuf,
        }

        impl TempDir {
            /// The directory is created private, the way a runtime directory
            /// is: it is the base the daemon works under, not the daemon's own
            /// directory.
            fn new(name: &str) -> Self {
                static NEXT: AtomicUsize = AtomicUsize::new(0);
                let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "metis-acl-{name}-{}-{sequence}",
                    std::process::id()
                ));
                std::fs::create_dir_all(&path).expect("create the temporary directory");
                std::fs::set_permissions(
                    &path,
                    std::fs::Permissions::from_mode(SOCKET_MODE_PRIVATE),
                )
                .expect("restrict the temporary directory");
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

        /// Holds `METIS_SOCKET_GROUP` for the duration of a test and restores
        /// the previous value on drop. The lock it carries serialises every
        /// environment test in this binary, so a test that wants no group
        /// configured holds it too.
        struct SocketGroupEnv {
            previous: Option<OsString>,
            _lock: MutexGuard<'static, ()>,
        }

        impl SocketGroupEnv {
            fn set(value: &str) -> Self {
                let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
                let previous = std::env::var_os(SOCKET_GROUP_ENV);
                // Edition 2024 makes mutating the process environment unsafe
                // because it is not thread-safe. Every test that touches the
                // variable holds `ENV_LOCK`, so nothing races this write.
                unsafe { std::env::set_var(SOCKET_GROUP_ENV, value) };
                Self {
                    previous,
                    _lock: lock,
                }
            }

            fn unset() -> Self {
                let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
                let previous = std::env::var_os(SOCKET_GROUP_ENV);
                // SAFETY: see `set`.
                unsafe { std::env::remove_var(SOCKET_GROUP_ENV) };
                Self {
                    previous,
                    _lock: lock,
                }
            }
        }

        impl Drop for SocketGroupEnv {
            fn drop(&mut self) {
                // SAFETY: see `set`; the lock is held until this guard is
                // dropped.
                unsafe {
                    match &self.previous {
                        Some(value) => std::env::set_var(SOCKET_GROUP_ENV, value),
                        None => std::env::remove_var(SOCKET_GROUP_ENV),
                    }
                }
            }
        }

        /// A group this process belongs to, preferring one that is not its
        /// primary group, so the assertion can tell an applied group from the
        /// group the file system would have used anyway.
        fn a_group_the_process_belongs_to() -> (String, u32) {
            let primary = unsafe { libc::getegid() };
            let count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
            if count > 0 {
                let mut groups = vec![0 as libc::gid_t; count as usize];
                let read = unsafe { libc::getgroups(count, groups.as_mut_ptr()) };
                for &group in groups.iter().take(read.max(0) as usize) {
                    if group != primary
                        && let Some(name) = group_name(group)
                    {
                        return (name, group);
                    }
                }
            }
            let name =
                group_name(primary).expect("the primary group is named in the group database");
            (name, primary)
        }

        /// The name the group database holds for `gid`, through the reentrant
        /// lookup so parallel tests cannot race a shared static entry.
        fn group_name(gid: u32) -> Option<String> {
            let mut buffer = vec![0 as libc::c_char; 1024];
            let mut entry: libc::group = unsafe { std::mem::zeroed() };
            let mut result: *mut libc::group = std::ptr::null_mut();
            let code = unsafe {
                libc::getgrgid_r(
                    gid,
                    &mut entry,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut result,
                )
            };
            if code != 0 || result.is_null() {
                return None;
            }
            let name = unsafe { std::ffi::CStr::from_ptr(entry.gr_name) };
            name.to_str().ok().map(str::to_owned)
        }

        fn mode_of(path: &Path) -> u32 {
            std::fs::metadata(path).expect("the path exists").mode() & 0o7777
        }

        /// The mode-taking half of the rule, which the daemon's state store
        /// uses: a directory is created at the mode it is given, a directory
        /// wider than that mode is refused untouched, and a file is narrowed to
        /// the mode it is given.
        #[test]
        fn the_private_path_rule_takes_its_mode() {
            let base = TempDir::new("state-modes");
            let directory = base.join("state");
            ensure_private_dir(&directory, 0o700).expect("the directory is created private");
            assert_eq!(mode_of(&directory), 0o700);

            let wider = base.join("wide");
            std::fs::create_dir(&wider).expect("create the pre-existing directory");
            std::fs::set_permissions(&wider, std::fs::Permissions::from_mode(0o777))
                .expect("widen the pre-existing directory");
            ensure_private_dir(&wider, 0o700).expect_err("mode 777 is wider than 700");
            assert_eq!(mode_of(&wider), 0o777, "the directory is not chmodded");

            let file = base.join("metis.db");
            std::fs::write(&file, b"").expect("create the file");
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644))
                .expect("widen the file");
            restrict_file(&file, 0o600).expect("the file mode is applied");
            assert_eq!(mode_of(&file), 0o600);
        }

        /// A socket directory the daemon creates is private to it: mode 700,
        /// owned by its own user, created under a base that may not exist yet,
        /// and accepted when the daemon starts again.
        #[test]
        fn a_fresh_socket_directory_is_created_private() {
            let _group = SocketGroupEnv::unset();
            let base = TempDir::new("fresh");
            let socket = base.join("nested/daemon.sock");

            ensure_socket_dir(&socket).expect("the daemon creates its own directory");
            let directory = std::fs::metadata(socket.parent().expect("the socket has a directory"))
                .expect("the directory exists");
            assert_eq!(directory.mode() & 0o7777, SOCKET_MODE_PRIVATE);
            assert_eq!(directory.uid(), unsafe { libc::geteuid() });

            ensure_socket_dir(&socket).expect("the directory it created is accepted again");
        }

        /// A configured group widens the mode to 770 and applies that group,
        /// whether the daemon creates the directory or finds it already private.
        #[test]
        fn a_configured_group_is_applied() {
            let (name, group) = a_group_the_process_belongs_to();
            let base = TempDir::new("group");
            let socket = base.join("created/daemon.sock");
            let directory = socket
                .parent()
                .expect("the socket has a directory")
                .to_path_buf();

            let ungrouped = SocketGroupEnv::unset();
            ensure_socket_dir(&socket).expect("the directory is created");
            assert_eq!(mode_of(&directory), SOCKET_MODE_PRIVATE);
            drop(ungrouped);

            let grouped = SocketGroupEnv::set(&name);
            ensure_socket_dir(&socket).expect("the configured group is applied");
            let metadata = std::fs::metadata(&directory).expect("the directory exists");
            assert_eq!(metadata.mode() & 0o7777, SOCKET_MODE_GROUP);
            assert_eq!(
                metadata.gid(),
                group,
                "the directory carries the configured group"
            );
            drop(grouped);
        }

        /// A directory wider than the rule allows is refused and left exactly
        /// as it is: the daemon does not undo an operator's own permission
        /// change behind their back.
        #[test]
        fn a_wider_pre_existing_directory_is_refused_and_left_alone() {
            let _group = SocketGroupEnv::unset();
            let base = TempDir::new("wide");
            let directory = base.join("shared");
            std::fs::create_dir(&directory).expect("create the pre-existing directory");
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
                .expect("widen the pre-existing directory");

            let error = ensure_socket_dir(&directory.join("daemon.sock"))
                .expect_err("mode 755 is wider than 700");
            match &error {
                ConfigError::SocketAcl { path, .. } => assert_eq!(path, &directory),
                other => panic!("expected a socket ACL refusal, got {other:?}"),
            }
            assert_eq!(mode_of(&directory), 0o755, "the directory is not chmodded");
        }

        /// The same rule follows the configured group: with a group the mode
        /// may be 770, and anything wider is still refused untouched.
        #[test]
        fn a_directory_wider_than_the_group_mode_is_refused() {
            let (name, _group) = a_group_the_process_belongs_to();
            let base = TempDir::new("group-wide");
            let directory = base.join("shared");
            std::fs::create_dir(&directory).expect("create the pre-existing directory");
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o777))
                .expect("widen the pre-existing directory");

            let grouped = SocketGroupEnv::set(&name);
            ensure_socket_dir(&directory.join("daemon.sock"))
                .expect_err("mode 777 is wider than 770");
            assert_eq!(mode_of(&directory), 0o777, "the directory is not chmodded");
            drop(grouped);
        }

        /// A symlink is refused rather than followed, so the ACL cannot be
        /// aimed at a directory the daemon was not told about.
        #[test]
        fn a_symlinked_directory_is_refused() {
            let _group = SocketGroupEnv::unset();
            let base = TempDir::new("symlink");
            let target = base.join("real");
            std::fs::create_dir(&target).expect("create the target directory");
            std::fs::set_permissions(
                &target,
                std::fs::Permissions::from_mode(SOCKET_MODE_PRIVATE),
            )
            .expect("restrict the target directory");
            let link = base.join("link");
            symlink(&target, &link).expect("create the symlink");

            ensure_socket_dir(&link.join("daemon.sock")).expect_err("a symlink is refused");
            assert!(
                std::fs::symlink_metadata(&link)
                    .expect("the symlink exists")
                    .file_type()
                    .is_symlink(),
                "the symlink is left in place"
            );
            assert_eq!(
                mode_of(&target),
                SOCKET_MODE_PRIVATE,
                "the target is untouched"
            );
        }

        /// A path that is not a directory is refused: the ACL is a directory
        /// rule, and a file at the socket's own path is not one.
        #[test]
        fn a_file_where_the_directory_belongs_is_refused() {
            let _group = SocketGroupEnv::unset();
            let base = TempDir::new("file");
            let file = base.join("not-a-directory");
            std::fs::write(&file, b"").expect("create the file");

            ensure_socket_dir(&file.join("daemon.sock")).expect_err("a file is not a directory");
        }

        /// A group the host does not have is a typed error, not a silent
        /// fallback to the primary group.
        #[test]
        fn an_unknown_group_is_a_typed_error() {
            let _group = SocketGroupEnv::set("metis-no-such-group");
            let base = TempDir::new("unknown-group");
            let socket = base.join("nested/daemon.sock");

            let error = ensure_socket_dir(&socket).expect_err("the group does not exist");
            assert!(
                matches!(error, ConfigError::UnknownSocketGroup { .. }),
                "expected an unknown group error, got {error:?}"
            );
        }

        /// The socket file is created under the same rule as its directory, so
        /// the ACL covers both halves of the path.
        #[tokio::test]
        async fn the_bound_socket_is_private_too() {
            let _group = SocketGroupEnv::unset();
            let base = TempDir::new("bound");
            let socket = base.join("nested/daemon.sock");
            let server_socket = socket.clone();
            tokio::spawn(async move {
                if let Err(err) = serve_uds(&server_socket, Router::new()).await {
                    eprintln!("listener error: {err}");
                }
            });
            assert!(
                wait_for_path(&socket).await,
                "the listener binds its socket"
            );

            let directory = std::fs::metadata(socket.parent().expect("the socket has a directory"))
                .expect("the directory exists");
            assert_eq!(directory.mode() & 0o7777, SOCKET_MODE_PRIVATE);
            assert_eq!(directory.uid(), unsafe { libc::geteuid() });

            let bound = std::fs::symlink_metadata(&socket).expect("the socket exists");
            assert!(bound.file_type().is_socket(), "the path is a socket");
            assert_eq!(bound.mode() & 0o7777, SOCKET_MODE_PRIVATE);
            assert_eq!(bound.uid(), unsafe { libc::geteuid() });
        }

        /// Wait, bounded, for a listener to create its socket file.
        async fn wait_for_path(path: &Path) -> bool {
            for _ in 0..100 {
                if path.exists() {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            false
        }
    }
}
