#![allow(dead_code)]
// rule: allow(dead_code): this module is compiled into each integration-test
// binary, and no single binary uses every helper it shares.
//!
//! What both runtime integration tests need: a built fixture, an instance, and
//! the smallest executor that can drive the runtime's futures.
//!
//! The library is rebuilt before it is loaded, for the same reason the ABI
//! harness rebuilds it: a test that loaded a stale `.so` would report the
//! results of an older source tree.
//!
//! The executor here is deliberately tiny and deliberately the only one. The
//! runtime hands back a `Future`; polling it is the caller's business, and these
//! tests are the caller.

use std::future::Future;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};

use plugin_fixture::payload::{
    EchoRequest, EchoResponse, FixtureConfig, HostContext, Initialisation,
};
use plugin_runtime::{Host, Loaded};

pub mod bundle;
pub mod signer;

/// How long a case may spin before it is called stuck.
pub const BUDGET: Duration = Duration::from_secs(5);

/// Polls one future to completion on this thread.
///
/// The waker is a no-op: every future the runtime hands back is either ready or
/// waiting on a host service the test answers itself, so a wake has nothing to
/// schedule. A real daemon passes its own task waker, which is exactly the
/// interface this exercises.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let mut cx = Context::from_waker(Waker::noop());
    let start = Instant::now();
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        assert!(start.elapsed() < BUDGET, "the future never finished");
        thread::sleep(Duration::from_millis(1));
    }
}

/// Builds the fixture cdylib and returns the path of the library it wrote.
///
/// The library is rebuilt before it is used, for the same reason the ABI
/// harness rebuilds it: a test that read a stale `.so` would report the results
/// of an older source tree.
pub fn fixture_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("plugin-fixture/Cargo.toml");
    let status = Command::new(env!("CARGO"))
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--lib")
        .status()
        .expect("cargo is runnable from a test, as the ABI harness already does");
    assert!(
        status.success(),
        "the fixture must build before it is loaded"
    );
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("../target/debug/libplugin_fixture.so")
        .canonicalize()
        .expect("the fixture library exists beside its build directory")
}

/// Builds and loads the fixture cdylib.
pub fn fixture() -> Arc<Loaded> {
    let path = fixture_path();
    // SAFETY: this test built the library from the tree it is testing.
    Arc::new(unsafe { Loaded::load(&path) }.expect("the fixture loads through the version gate"))
}

/// The fixture's request shape, serialised.
pub fn request(echo: &str) -> Vec<u8> {
    serde_json::to_vec(&EchoRequest {
        echo: echo.to_string(),
        repeat: 1,
    })
    .expect("the request serialises")
}

/// The fixture's response shape, parsed.
pub fn decode(bytes: &[u8]) -> EchoResponse {
    serde_json::from_slice(bytes).expect("the response is the fixture's shape")
}

/// The fixture's declared configuration and host context, as the one
/// initialisation body the runtime delivers.
pub fn initialisation_body(instance_name: &str) -> Vec<u8> {
    Initialisation::new(
        FixtureConfig {
            instance: instance_name.to_string(),
        },
        HostContext {
            instance_name: instance_name.to_string(),
            data_dir: None,
            capability_version: 1,
        },
    )
    .expect("the fixture's own payload is buildable")
    .body()
}

/// A loaded library, a host, and one initialised instance.
pub fn new_instance() -> (Host, Arc<plugin_runtime::Instance>) {
    let host = Host::new();
    let loaded = fixture();
    let instance = block_on(
        loaded
            .init_plugin(&host, &initialisation_body("fixture"))
            .expect("the fixture's initialisation entry starts"),
    )
    .expect("the fixture initialises with the configuration it declares");
    (host, instance)
}

/// The number of threads in this process, which is how "the daemon is the only
/// executor" is measured rather than argued.
pub fn thread_count() -> usize {
    std::fs::read_dir("/proc/self/task")
        .map(|entries| entries.count())
        .unwrap_or(0)
}
