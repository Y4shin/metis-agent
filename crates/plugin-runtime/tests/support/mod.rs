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

use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
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
pub mod sample;
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

/// Compiles one of the `tests/fixtures/` sources into a cdylib, once per name.
///
/// A fixture is a library this tree did not write: it restates the contract's
/// shapes itself, so a rule that only holds for the fixture cannot be shown to
/// it. The library is rebuilt whenever its source is newer than it, so a run
/// never loads a stale one.
pub fn compiled_fixture(name: &str) -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(format!("{name}.rs"));
    let directory = repository_root().join("target/fixture-tests");
    std::fs::create_dir_all(&directory).expect("the test's scratch directory is creatable");
    let library = directory.join(format!("lib{name}{}", std::env::consts::DLL_SUFFIX));
    if built_after(&library, &source) {
        return library;
    }
    // The compiler writes beside the library and the result is moved into
    // place, so a run that is killed mid-compile leaves no partial library.
    let building = directory.join(format!(
        "lib{name}.building{}",
        std::env::consts::DLL_SUFFIX
    ));
    let status = Command::new(rustc())
        .arg("--crate-type=cdylib")
        .arg("--edition=2024")
        .arg("-o")
        .arg(&building)
        .arg(&source)
        .status()
        .expect("rustc is runnable from a test, as the fixture build already assumes");
    assert!(status.success(), "the fixture {} builds", source.display());
    std::fs::rename(&building, &library).expect("the built library is moved into place");
    library
}

/// The repository root, which is two levels above this crate.
fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("the repository root")
        .to_path_buf()
}

/// The `rustc` a fixture is compiled with.
///
/// Every command of this effort runs inside the devenv shell, which is where the
/// toolchain's `rustc` is on `PATH`; `RUSTC` names it when a caller set that.
fn rustc() -> OsString {
    std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into())
}

/// Whether a built fixture is at least as new as the source it came from.
fn built_after(library: &Path, source: &Path) -> bool {
    let built = library.metadata().and_then(|metadata| metadata.modified());
    let written = source.metadata().and_then(|metadata| metadata.modified());
    match (built, written) {
        (Ok(built), Ok(written)) => built >= written,
        _ => false,
    }
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
