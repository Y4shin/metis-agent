//! The optional remote transport: the TCP listener, its configuration rule, and
//! the wrapper's TCP path.
//!
//! TCP is the first front-end every target has, so this test binary is not
//! platform-gated, unlike the Unix socket test in `echo.rs`. The seams are the
//! off-by-default start, the listener that serves the landed router, and the
//! listener task releasing its port when it is dropped.

use std::ffi::OsString;
use std::net::{SocketAddr, TcpListener};
use std::sync::{Mutex, MutexGuard, PoisonError};

use api_contracts::transport::REMOTE_ADDR_ENV;
use client::Client;
use client::skeleton;

/// Serialises the environment in this test binary: `std::env` is process wide
/// and the harness runs the tests on threads.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Holds one value of `METIS_TCP_ADDR` for the duration of a test and restores
/// the previous value on drop, so a panic cannot leak the value into another
/// test. The lock it carries serialises every environment test in this binary.
struct RemoteAddrOverride {
    previous: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl RemoteAddrOverride {
    fn set(value: &str) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = std::env::var_os(REMOTE_ADDR_ENV);
        // Edition 2024 makes mutating the process environment unsafe because it
        // is not thread-safe. Every environment test in this binary holds
        // `ENV_LOCK`, so nothing races this write.
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

impl Drop for RemoteAddrOverride {
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

/// Reserve a loopback port by binding and releasing it, so a test can require a
/// bind on that exact port afterwards.
fn reserve_loopback_port() -> SocketAddr {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("reserve a loopback port");
    let addr = listener.local_addr().expect("read the reserved address");
    drop(listener);
    addr
}

/// With no remote configuration the start path binds nothing, and the measured
/// form of "nothing" is that the port the test reserved before the call is
/// still free after it.
#[tokio::test]
async fn no_remote_configuration_opens_no_tcp_socket() {
    let reserved = reserve_loopback_port();
    let env = RemoteAddrOverride::unset();

    let started = skeleton::serve_remote()
        .await
        .expect("an absent variable is not an error");
    assert!(started.is_none(), "an absent variable enables no listener");

    let probe = TcpListener::bind(reserved).expect("no listener took the port");
    drop(probe);
    drop(env);
}

/// The listener owns its socket through the task it runs on, so dropping that
/// task releases the port. A listener that outlived its test would still hold
/// the port, which is what the rebind below measures.
#[test]
fn the_listener_task_releases_its_port_when_it_is_dropped() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build the test runtime");
    let env = RemoteAddrOverride::set("127.0.0.1:0");

    let addr = runtime
        .block_on(skeleton::serve_remote())
        .expect("a literal address is accepted")
        .expect("naming an address enables the listener");

    drop(env);
    drop(runtime);

    let probe =
        TcpListener::bind(addr).expect("the port is free once the listener task is dropped");
    drop(probe);
}

/// The wrapper reaches the skeleton over a real loopback listener and returns
/// the echoed message. The address comes from the listener, so the test never
/// guesses a port, and the call traverses the same `serve_stream` the duplex
/// and socket tests drive: a second serve loop would have to reproduce the
/// service to answer it.
#[tokio::test]
async fn the_client_echoes_over_the_remote_listener() {
    let env = RemoteAddrOverride::set("127.0.0.1:0");

    let addr = skeleton::serve_remote()
        .await
        .expect("a literal address is accepted")
        .expect("naming an address enables the listener");

    let caller = Client::with_remote_addr(addr)
        .await
        .expect("the remote transport builds");
    let echoed = caller
        .echo("hello over the remote listener")
        .await
        .expect("the wrapper performs the echo RPC");

    assert_eq!(echoed, "hello over the remote listener");
    drop(env);
}
