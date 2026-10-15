//! The one release entry, tested at its one public seam: a serving plugin in,
//! a report of what the release did out, with the ordering ledger as the
//! assertion.
//!
//! Every plugin here is loaded from a real signed bundle built inside the test
//! from the fixture cdylib's own bytes and a hand-written manifest, the same way
//! the load tests build one. The release entry is the load entry's counterpart,
//! so it is driven the same way: the test owns the poll loop, the deadline and
//! the drop.
//!
//! Three properties shape the cases below.
//!
//! **The caller's own drop is what drains a future.** The runtime hands a call
//! back to whoever asked for it, so the release entry cannot drop it. It cancels
//! the instance's calls and waits, bounded by the caller's deadline, for the
//! caller to drop them; a call still in flight at the deadline is reported
//! rather than forced.
//!
//! **An instance that cannot be released stays mapped.** The report keeps the
//! library reference when it could not release the instance, so the library is
//! still mapped after the release returns and nothing inside plugin code can
//! have been unmapped underneath a poll that never returned.
//!
//! **Disable and enable are registry state.** They refuse or accept new calls
//! and touch neither the in-flight calls nor the library.

mod support;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use plugin_contract::abi::MetisStatus;
use plugin_runtime::load::{Serving, load_bundle};
use plugin_runtime::{CallError, Host, PluginCall};
use serde_json::{Value, json};
use support::block_on;
use support::bundle;

/// The name the test chain's leaf-certifying certificate carries, which is the
/// name the manifest declares and the name the instance is registered under.
const NAME: &str = bundle::INSTANCE_NAME;

/// The configuration the host resolved for the fixture, and the host context
/// that names the same instance: the fixture's own constructor refuses a pair
/// that disagrees.
fn configuration() -> Value {
    json!({ "instance": NAME })
}

/// The host context, as contract 18's named part.
fn context() -> Value {
    json!({ "instance_name": NAME, "capability_version": 1 })
}

/// A serving plugin loaded from the fixture's own signed bundle.
///
/// The bundle file is removed when the guard drops, which is after the load has
/// read and staged it: the running instance needs no file.
fn serving(host: &Host) -> Serving {
    let (file, trust) = bundle::bundle_file(Some(&bundle::manifest(&bundle::CAPABILITIES)));
    match block_on(load_bundle(
        host,
        file.path(),
        &configuration(),
        &context(),
        &trust,
    )) {
        Ok(serving) => serving,
        Err(error) => panic!("the fixture's own bundle loads, and this one refused: {error}"),
    }
}

/// Polls one call once.
///
/// The waker is the no-op one: every poll here is either ready or parked on
/// something the test drives itself, so a wake has nothing to schedule. A real
/// daemon passes its own task waker, which is the same interface.
fn poll_once(call: &mut PluginCall) -> Poll<Result<Vec<u8>, CallError>> {
    let mut context = Context::from_waker(Waker::noop());
    Pin::new(call).poll(&mut context)
}

/// The deadline every release here is given, which is the caller's own
/// monotonic reading as `Instance::cancel_all` already takes one.
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(1)
}

#[test]
fn a_release_cancels_the_calls_it_finds_then_quiesces_destroys_and_unloads() {
    let host = Host::new();
    let serving = serving(&host);
    let name = serving.name().to_string();

    // One call in flight, parked on the fixture's cancellation token. The
    // caller holds no reference of its own: the call holds the one that keeps
    // the instance alive, which is what the unload's last-reference rule reads.
    let instance = Arc::clone(serving.instance());
    let mut call = instance
        .invoke("tool.cancellable", 1, &support::request("release"))
        .expect("the fixture's cancellable capability starts");
    assert!(
        matches!(poll_once(&mut call), Poll::Pending),
        "the cancellable capability parks on its token"
    );
    drop(instance);

    let deadline = deadline();
    let mut release = Box::pin(serving.release(deadline));
    let mut context = Context::from_waker(Waker::noop());

    // The first poll unregisters the instance, cancels the call it found, and
    // waits: the caller's own drop is what drains the future.
    assert!(
        matches!(release.as_mut().poll(&mut context), Poll::Pending),
        "the release waits for the caller to drop the futures it cancelled"
    );
    assert!(
        matches!(
            poll_once(&mut call),
            Poll::Ready(Err(CallError::Status(MetisStatus::Cancelled)))
        ),
        "the cancelled call resolves as cancelled on its next poll"
    );
    drop(call);
    let report = match release.as_mut().poll(&mut context) {
        Poll::Ready(report) => report,
        Poll::Pending => panic!("the release finishes once the caller's futures are gone"),
    };

    assert_eq!(report.name(), name, "the report names the instance");
    assert_eq!(
        report.deadline(),
        deadline,
        "the report names back the deadline it was given"
    );
    assert!(
        report.released(),
        "the instance was released: {}",
        report.refusal().map(ToString::to_string).unwrap_or_default()
    );
    assert!(report.refusal().is_none());
    assert!(
        report.resident().is_none(),
        "a released instance keeps no library reference"
    );
    assert_eq!(
        report.cancelled().cancelled,
        vec![1],
        "the call the release found is the call it cancelled"
    );

    // The ledger proves the documented order, on the report's own record.
    let ledger = report.ledger();
    let position = |needle: &str| {
        ledger
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("the ledger carries {needle:?}: {ledger:#?}"))
    };
    let unregistered = position("unregistered");
    let cancelled = position("cancelled");
    let dropped = position("call 1 dropped, context reclaimed");
    let quiesced = position("instance quiesced after every future was dropped");
    let destroyed = position("instance destroyed");
    let unloaded = position("library unloaded");
    assert!(
        unregistered < cancelled && cancelled < dropped,
        "the instance is unregistered and its calls are cancelled first: {ledger:#?}"
    );
    assert!(
        dropped < quiesced,
        "nothing quiesces while a future is alive: {ledger:#?}"
    );
    assert!(
        quiesced < destroyed,
        "the plugin's shutdown entry runs before the instance is destroyed: {ledger:#?}"
    );
    assert!(
        destroyed < unloaded,
        "the library is unloaded only after the instance is destroyed: {ledger:#?}"
    );

    // The plugin's own record shows the cancel reached it, which is what "the
    // in-flight calls had to be cancelled first" means on the plugin's side.
    assert!(
        host.logs()
            .iter()
            .any(|line| line.contains("cancel accepted for request 1")),
        "the accepted cancel reached the plugin: {:#?}",
        host.logs()
    );
}
