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
use plugin_fixture::REFUSING_SHUTDOWN;
use plugin_runtime::load::{ReleaseRefusal, Serving, load_bundle};
use plugin_runtime::{CallError, Host, PluginCall, RouteError};
use serde_json::{Value, json};
use support::block_on;
use support::bundle;

/// The name the test chain's leaf-certifying certificate carries, which is the
/// name the manifest declares and the name the instance is registered under.
const NAME: &str = bundle::INSTANCE_NAME;

/// The configuration the host resolved for the fixture, and the host context
/// that names the same instance: the fixture's own constructor refuses a pair
/// that disagrees.
fn configuration(instance: &str) -> Value {
    json!({ "instance": instance })
}

/// The host context, as contract 18's named part.
fn context(instance: &str) -> Value {
    json!({ "instance_name": instance, "capability_version": 1 })
}

/// A serving plugin loaded from the fixture's own signed bundle.
///
/// The bundle file is removed when the guard drops, which is after the load has
/// read and staged it: the running instance needs no file.
fn serving(host: &Host) -> Serving {
    serving_as(host, NAME)
}

/// The same, under the configuration and host context `instance` names.
///
/// The fixture's constructor compares those two names with each other, and its
/// reserved names ask for a fault: this is how a case asks the fixture to refuse
/// one teardown entry. The manifest's name is the chain's leaf common name and
/// is not what this chooses.
fn serving_as(host: &Host, instance: &str) -> Serving {
    let (file, trust) = bundle::bundle_file(Some(&bundle::manifest(&bundle::CAPABILITIES)));
    match block_on(load_bundle(
        host,
        file.path(),
        &configuration(instance),
        &context(instance),
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

#[test]
fn disabling_refuses_new_calls_and_enabling_accepts_them_with_no_reload() {
    let host = Host::new();
    let mut serving = serving(&host);
    let name = serving.name().to_string();
    let qualified = format!("{name}.tool.echo");
    let instance = Arc::clone(serving.instance());

    // One call in flight before the entry is turned off.
    let mut call = instance
        .invoke("tool.cancellable", 3, &support::request("off"))
        .expect("the fixture's cancellable capability starts");
    assert!(
        matches!(poll_once(&mut call), Poll::Pending),
        "the cancellable capability parks on its token"
    );

    assert!(
        serving.registry_mut().enable(&name, false),
        "the entry is registered, so it is there to turn off"
    );
    let refusal = serving
        .registry()
        .route(&qualified)
        .err()
        .unwrap_or_else(|| panic!("{qualified} is still served while the entry is off"));
    match refusal {
        RouteError::Disabled {
            name: refused,
            reason,
        } => {
            assert_eq!(refused, qualified, "the refusal names what was asked for");
            assert!(
                !reason.is_empty(),
                "the refusal carries the reason the entry is off: {reason:?}"
            );
        }
        other => panic!("a disabled entry refuses a new call, and this answered {other}"),
    }

    // The call in flight is untouched: it still resolves the way the plugin
    // resolves it, and nothing unloaded the library underneath it.
    assert!(
        matches!(poll_once(&mut call), Poll::Pending),
        "disabling touches no in-flight call"
    );
    instance
        .cancel(3)
        .expect("the host still holds the call's id");
    assert!(
        matches!(
            poll_once(&mut call),
            Poll::Ready(Err(CallError::Status(MetisStatus::Cancelled)))
        ),
        "the in-flight call reaches the plugin's own cancelled answer"
    );
    drop(call);

    // Enabling accepts a call again, on the same instance and the same library:
    // nothing was reloaded and nothing was unmapped.
    assert!(
        serving.registry_mut().enable(&name, true),
        "the entry is still there to turn on"
    );
    let answered = block_on(serving.registry().call(
        &qualified,
        &support::request("on again"),
        4,
    ))
    .expect("an enabled entry serves the call again");
    assert_eq!(
        support::decode(&answered).echo,
        "on again",
        "the same instance answered"
    );
    assert!(
        Arc::ptr_eq(serving.instance(), &instance),
        "no reload: the instance that served before is the one serving now"
    );
    assert!(
        !host
            .ordering()
            .iter()
            .any(|line| line.contains("library unloaded")),
        "disabling and enabling touch no library: {:#?}",
        host.ordering()
    );
}

#[test]
fn a_shutdown_that_answers_a_non_ok_status_leaves_the_library_mapped() {
    let host = Host::new();
    // The fixture's own reserved name asks its teardown entry to refuse, which
    // is a status no in-tree plugin could reach any other way: `stopped` is
    // infallible, and the adapter answers `internal` for a panic inside it.
    let serving = serving_as(&host, REFUSING_SHUTDOWN);
    let name = serving.name().to_string();

    let report = block_on(serving.release(deadline()));

    assert_eq!(report.name(), name, "the report names the instance");
    assert!(
        !report.released(),
        "an instance whose shutdown refused was not released"
    );
    match report.refusal() {
        Some(ReleaseRefusal::Shutdown(CallError::Status(status))) => assert_eq!(
            status,
            &MetisStatus::Internal,
            "the refusal carries the status the shutdown entry answered"
        ),
        other => panic!("the shutdown entry refused, and this is the outcome: {other:?}"),
    }
    assert_eq!(
        report.refusal().map(ReleaseRefusal::step),
        Some("shutdown"),
        "the report names the step that refused"
    );

    // The library is still mapped: reading the plugin's own table through it
    // would fault if the platform had unmapped it.
    let resident = report
        .resident()
        .expect("an instance that could not be released keeps its library mapped");
    assert!(
        resident.has_service_complete(),
        "the library is still mapped after a refused shutdown"
    );

    let ledger = report.ledger();
    assert!(
        ledger
            .iter()
            .any(|line| line.contains("release refused: the shutdown entry answered")),
        "the refusal is recorded rather than inferred: {ledger:#?}"
    );
    assert!(
        !ledger.iter().any(|line| line.contains("instance destroyed")),
        "a plugin that did not quiesce is not destroyed: {ledger:#?}"
    );
    assert!(
        !ledger.iter().any(|line| line.contains("library unloaded")),
        "nothing here unloads a library: {ledger:#?}"
    );
}

#[test]
fn an_unload_refused_for_another_reason_leaves_the_library_mapped() {
    let host = Host::new();
    let serving = serving(&host);
    let name = serving.name().to_string();
    // A caller that still holds the instance is exactly what the landed unload
    // guard refuses: the release reaches the unload step and the guard answers.
    let held = Arc::clone(serving.instance());

    let report = block_on(serving.release(deadline()));

    assert_eq!(report.name(), name, "the report names the instance");
    assert!(
        !report.released(),
        "an instance whose unload was refused was not released"
    );
    match report.refusal() {
        Some(ReleaseRefusal::Unload(error)) => assert!(
            error.to_string().contains("a call still holds the instance"),
            "the refusal is the landed guard's own error: {error}"
        ),
        other => panic!("the unload guard refused, and this is the outcome: {other:?}"),
    }
    assert_eq!(
        report.refusal().map(ReleaseRefusal::step),
        Some("unload"),
        "the report names the step that refused"
    );

    let resident = report
        .resident()
        .expect("an instance that could not be released keeps its library mapped");
    assert!(
        resident.has_service_complete(),
        "the library is still mapped after a refused unload"
    );
    assert!(
        Arc::ptr_eq(resident, held.loaded()),
        "the kept reference is the library the unreleased instance came from"
    );

    // The plugin quiesced before the unload was attempted, and the refusal is
    // the guard's own line rather than a restatement of it.
    let ledger = report.ledger();
    assert!(
        ledger.iter().any(|line| line.contains("instance quiesced")),
        "the plugin quiesced first: {ledger:#?}"
    );
    assert!(
        ledger
            .iter()
            .any(|line| line.contains("unload refused: a call still holds the instance")),
        "the guard's refusal is recorded rather than inferred: {ledger:#?}"
    );
    assert!(
        !ledger.iter().any(|line| line.contains("library unloaded")),
        "nothing here unloads a library: {ledger:#?}"
    );
}

#[test]
fn a_quarantined_instance_keeps_its_library_resident() {
    let host = Host::new();
    let serving = serving(&host);
    let name = serving.name().to_string();
    // A poll this instance abandoned is still running: no new future is
    // accepted, and nothing may unmap the library under it.
    serving
        .instance()
        .quarantine("a poll was abandoned by the watchdog");

    let report = block_on(serving.release(deadline()));

    assert_eq!(report.name(), name, "the report names the instance");
    assert!(
        !report.released(),
        "a quarantined instance is not released, and it is not forced"
    );
    assert_eq!(
        report.refusal(),
        Some(&ReleaseRefusal::Quarantined),
        "the refusal is the quarantine, not a later consequence of it"
    );
    assert_eq!(
        report.refusal().map(ReleaseRefusal::step),
        Some("unload"),
        "the step the quarantine refuses is the unload"
    );

    // Its library stays resident, which is what quarantine is for.
    let resident = report
        .resident()
        .expect("a quarantined instance keeps its library mapped");
    assert!(
        resident.has_service_complete(),
        "the library is still mapped after the refusal"
    );

    // The plugin was never asked to quiesce or destroy: an abandoned poll may
    // still be running, and the instance stops accepting new futures instead.
    let ledger = report.ledger();
    assert!(
        ledger
            .iter()
            .any(|line| line.contains("release refused: the instance is quarantined")),
        "the refusal is recorded: {ledger:#?}"
    );
    assert!(
        !ledger.iter().any(|line| line.contains("instance quiesced")),
        "a quarantined instance is not asked to quiesce: {ledger:#?}"
    );
    assert!(
        !ledger.iter().any(|line| line.contains("instance destroyed")),
        "a quarantined instance is not destroyed: {ledger:#?}"
    );
    assert!(
        !ledger.iter().any(|line| line.contains("library unloaded")),
        "nothing here unloads a library: {ledger:#?}"
    );
}
