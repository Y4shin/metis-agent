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
use support::signer::{self, TestChain};

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

/// A hand-written manifest for the fixture library, under `name`.
///
/// The text is a literal rather than a serialisation of any type in this tree,
/// because the format belongs to the author. It is written here rather than read
/// out of the shared support module because that module's manifest carries one
/// fixed name, and a case here needs two instances that a report can tell apart.
fn manifest(name: &str) -> String {
    let target = bundle::host_target();
    let library = bundle::library_path(&target, &bundle::library_file_name());
    let declared: String = bundle::CAPABILITIES
        .iter()
        .map(|capability| {
            format!(
                "\n[[capabilities]]\nname = \"{capability}\"\nkind = \"tool\"\nversion = 1\n\
                 request_schema = {{ type = \"object\" }}\n\
                 response_schema = {{ type = \"object\" }}\n"
            )
        })
        .collect();
    format!(
        r#"name = "{name}"
version = "1.0.0"
abi_major = 1
min_abi_minor = 2

[libraries]
{target} = "{library}"
{declared}
[config_schema]
type = "object"
"#
    )
}

/// The chain whose leaf common name is `name`, which is the name the load
/// compares the manifest against.
fn chain(name: &str) -> TestChain {
    let mut specs = signer::standard_specs(3);
    specs[0].common_name = name.to_string();
    TestChain::build(&specs, &[1, 2, 3])
}

/// A signed bundle carrying the fixture's library under a manifest named
/// `name`, and the trust store that accepts it.
///
/// The bundle file is removed when the guard drops, which is after the load has
/// read and staged it: the running instance needs no file.
fn signed_bundle(name: &str) -> (bundle::BundleFile, plugin_runtime::bundle::TrustStore) {
    let chain = chain(name);
    let trust = bundle::trust_for(&chain);
    let entries = bundle::entries(Some(&manifest(name)), &[]);
    let bytes = signer::signed_bundle(&chain, &bundle::archive(&entries));
    (bundle::BundleFile::write(&bytes), trust)
}

/// A serving plugin loaded from a bundle named `bundle`, whose configuration
/// and host context name `instance`.
///
/// The two names are given separately because the fixture's own constructor
/// compares the configuration with the host context, and its reserved names ask
/// for a fault: a case asks the fixture to refuse one teardown entry by naming
/// the instance it initialised, while the bundle it came from keeps its own name.
fn serving_with(host: &Host, bundle: &str, instance: &str) -> Serving {
    let (file, trust) = signed_bundle(bundle);
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

/// A serving plugin loaded under the name the fixture's own bundle declares.
fn serving(host: &Host) -> Serving {
    serving_with(host, NAME, NAME)
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
        report
            .refusal()
            .map(ToString::to_string)
            .unwrap_or_default()
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
    let answered = block_on(
        serving
            .registry()
            .call(&qualified, &support::request("on again"), 4),
    )
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
    let serving = serving_with(&host, NAME, REFUSING_SHUTDOWN);
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
        !ledger
            .iter()
            .any(|line| line.contains("instance destroyed")),
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
            error
                .to_string()
                .contains("a call still holds the instance"),
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
        !ledger
            .iter()
            .any(|line| line.contains("instance destroyed")),
        "a quarantined instance is not destroyed: {ledger:#?}"
    );
    assert!(
        !ledger.iter().any(|line| line.contains("library unloaded")),
        "nothing here unloads a library: {ledger:#?}"
    );
}

#[test]
fn the_shutdown_deadline_ends_the_wait_and_the_report_names_every_instance() {
    let host = Host::new();
    // Two instances a report can tell apart by name: one whose call the caller
    // never drops, and one whose abandoned poll quarantined it.
    let waiting = serving_with(&host, "waiting", "waiting");
    let instance = Arc::clone(waiting.instance());
    let mut call = instance
        .invoke("tool.cancellable", 9, &support::request("waits"))
        .expect("the fixture's cancellable capability starts");
    assert!(
        matches!(poll_once(&mut call), Poll::Pending),
        "the cancellable capability parks on its token"
    );
    drop(instance);
    let quarantined = serving_with(&host, "quarantined", "quarantined");
    quarantined
        .instance()
        .quarantine("a poll was abandoned by the watchdog");

    // The daemon's own shutdown deadline: one reading, given to both releases.
    let deadline = Instant::now() + Duration::from_millis(80);
    let waiting_report = block_on(waiting.release(deadline));
    assert!(
        Instant::now() >= deadline,
        "the wait ends at the deadline rather than before it"
    );
    let quarantined_report = block_on(quarantined.release(deadline));

    // The report names every instance that could not be released. The release
    // entry is per instance, so this is one report each and the daemon is what
    // aggregates them.
    let reports = [&waiting_report, &quarantined_report];
    let unreleased: Vec<&str> = reports
        .iter()
        .filter(|report| !report.released())
        .map(|report| report.name())
        .collect();
    assert_eq!(
        unreleased,
        vec!["waiting", "quarantined"],
        "every instance that could not be released is named"
    );
    assert!(
        reports.iter().all(|report| report.deadline() == deadline),
        "every report names back the deadline it was given"
    );
    assert!(
        waiting_report
            .to_string()
            .starts_with("waiting could not be released at the drain step"),
        "the report's own words name the instance and the step: {waiting_report}"
    );

    // The call the caller never dropped is what ended the wait, and the report
    // names it by id rather than by a count alone.
    assert_eq!(
        waiting_report.refusal(),
        Some(&ReleaseRefusal::FuturesInFlight(vec![9])),
        "the call still in flight at the deadline is named"
    );
    assert!(
        waiting_report
            .ledger()
            .iter()
            .any(|line| line.contains("still in flight at the deadline")),
        "the refusal is recorded: {:#?}",
        waiting_report.ledger()
    );
    assert!(
        !waiting_report
            .ledger()
            .iter()
            .any(|line| line.contains("instance quiesced")),
        "nothing quiesces while a future is alive: {:#?}",
        waiting_report.ledger()
    );
    assert!(
        waiting_report.resident().is_some(),
        "the library of an instance that could not be released stays mapped"
    );
    assert_eq!(
        quarantined_report.refusal(),
        Some(&ReleaseRefusal::Quarantined),
        "the second instance is unreleased for its own reason"
    );

    // The caller's own drop is what turns the deadline into the recorded
    // unacknowledged cancel, which is the level-4 rule read at this seam.
    drop(call);
    assert!(
        host.ordering()
            .iter()
            .any(|line| line.contains("cancel unacknowledged")),
        "a call dropped after the deadline is recorded: {:#?}",
        host.ordering()
    );
}

/// The name the child runs under: the fault test is ignored by default, so the
/// only way it runs is the parent below naming it.
const FAULTING_CHILD: &str = "a_copied_handle_dropped_after_unload_faults_the_process";

#[test]
fn the_release_entry_has_no_outstanding_clone_gate() {
    let host = Host::new();
    let serving = serving(&host);

    // One clone of the waker this call is polled with stays with the plugin, and
    // the call is then dropped: the clone outlives the call, and it is the state
    // a clone counter would have to count if one gated the unload.
    let instance = Arc::clone(serving.instance());
    let mut call = instance
        .invoke("tool.deferred", 5, &support::request("defer"))
        .expect("the fixture's deferred capability starts");
    assert!(
        matches!(poll_once(&mut call), Poll::Pending),
        "the deferred capability parks and keeps the waker clone it was polled with"
    );
    drop(call);
    drop(instance);

    let (handed_out, released, _) = host.waker_counts();
    assert!(
        handed_out > released,
        "one owning waker reference is still outstanding: {handed_out} handed out, {released} released"
    );

    // The release unloads anyway: no counter gates it, and the hazard findings
    // settled that one should not.
    let report = block_on(serving.release(deadline()));
    assert!(
        report.released(),
        "an outstanding clone is not a release refusal: {}",
        report
            .refusal()
            .map(ToString::to_string)
            .unwrap_or_default()
    );
    assert!(report.resident().is_none());
    assert!(
        report
            .ledger()
            .iter()
            .any(|line| line.contains("library unloaded")),
        "the library is unloaded with a clone outstanding: {:#?}",
        report.ledger()
    );
    assert!(
        !report.ledger().iter().any(|line| line.contains("refused")),
        "nothing refuses the release over a clone: {:#?}",
        report.ledger()
    );
}

#[test]
fn a_copied_handle_dropped_after_unload_faults_the_child() {
    if !cfg!(unix) {
        println!(
            "[skip] the fault is asserted by the signal the child dies with, which this platform does not report"
        );
        return;
    }
    let binary = std::env::current_exe().expect("the test binary's own path is readable");
    let child = std::process::Command::new(binary)
        .args(["--exact", FAULTING_CHILD, "--ignored", "--nocapture"])
        .env("RUST_TEST_THREADS", "1")
        .output()
        .expect("the test binary spawns its own faulting child");
    let said = String::from_utf8_lossy(&child.stderr);
    assert!(
        said.contains("the library is unloaded, and the copied handle's drop call runs now"),
        "the child reached the drop call after the unload, so the fault is that drop: {said}"
    );
    assert!(
        !child.status.success(),
        "the child is the fault, so it must not exit cleanly: {}",
        child.status
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            child.status.signal(),
            Some(11),
            "a handle dropped after its library was unloaded faults the child: {}",
            child.status
        );
    }
}

/// The fault itself, in its own process: a future handle copied out of the table
/// and dropped after the library was unloaded.
///
/// It is ignored by default and run by the test above, because the fault is the
/// point: the child dies inside the drop call, so no assertion about what
/// happened can be made in the process that does it. What the parent asserts is
/// the signal.
///
/// The path is the raw table, which is the only one that reaches the fault: the
/// runtime's own `unload` refuses while any future it handed out is alive, and
/// `Instance::invoke` ties every future to the instance, so a handle copied out
/// of the table is the one thing that outlives both guards. The ABI allows a
/// caller to copy that handle, which is why the contract states the after-unload
/// rule as a hard precondition rather than a check the host can make.
#[test]
#[ignore = "run in a subprocess by a_copied_handle_dropped_after_unload_faults_the_child"]
fn a_copied_handle_dropped_after_unload_faults_the_process() {
    use std::ffi::c_void;

    use plugin_contract::abi::{
        METIS_BUFFER_OWNER_HOST, MetisBuffer, MetisFutureHandle, MetisStatus, MetisWaker,
    };
    use plugin_contract::payload::metis_encode_payload;

    /// A host-owned buffer carrying the payload envelope the contract fixes.
    fn host_buffer(body: &[u8]) -> (Vec<u8>, MetisBuffer) {
        let encoded = metis_encode_payload(body);
        let buffer = MetisBuffer {
            ptr: encoded.as_ptr() as *mut u8,
            len: encoded.len(),
            cap: encoded.len(),
            owner: METIS_BUFFER_OWNER_HOST,
            reserved: 0,
        };
        (encoded, buffer)
    }

    let loaded = Arc::new(
        // SAFETY: this test built the library from the tree it is testing, which
        // is what the version gate's caller has to establish.
        unsafe { plugin_runtime::Loaded::load(support::fixture_path()) }
            .expect("the fixture loads through the version gate"),
    );
    let host = Host::new();
    host.bind_release(loaded.table().release_buffer);

    // Initialisation, driven by hand: the raw path is what has to be exercised,
    // so the future is polled with the null waker a caller with no executor has.
    let (_init_bytes, init_buffer) = host_buffer(&support::initialisation_body("fixture"));
    let mut instance: *mut c_void = std::ptr::null_mut();
    let mut init = MetisFutureHandle::null();
    // SAFETY: the table is the fixture's own, the host table is live, and the
    // payload buffer is host-owned and valid for the call.
    let status =
        unsafe { (loaded.table().init_plugin)(host.api(), init_buffer, &mut instance, &mut init) };
    assert_eq!(
        status,
        MetisStatus::Ok,
        "the fixture's initialisation entry starts"
    );
    let mut out = MetisBuffer::empty();
    // SAFETY: the handle came from this table's `init_plugin`.
    let status = unsafe { ((*init.vtable).poll)(init.ctx, MetisWaker::null(), &mut out) };
    assert_eq!(
        status,
        MetisStatus::Ok,
        "the fixture's initialisation resolves on its first poll"
    );

    // One call, and a copy of the handle it hands out.
    let capability = b"tool.echo";
    let (_request_bytes, request_buffer) = host_buffer(&support::request("after unload"));
    let mut handle = MetisFutureHandle::null();
    // SAFETY: the instance is live, the request buffer is host-owned and valid
    // for this call, and the out parameter is a local.
    let status = unsafe {
        (loaded.table().invoke_future)(
            instance,
            capability.as_ptr(),
            capability.len(),
            77,
            request_buffer,
            host.api(),
            &mut handle,
        )
    };
    assert_eq!(status, MetisStatus::Ok, "the call starts");
    let copied = handle;
    assert!(
        !copied.is_null(),
        "the call handed out a future handle to copy"
    );

    // The contract's rule: the table is dead after `destroy_instance`. The
    // instance is destroyed and the last reference to the library is dropped,
    // which is what unmaps it.
    // SAFETY: the pointer came from this table's `init_plugin`.
    let destroyed = unsafe { (loaded.table().destroy_instance)(instance) };
    assert_eq!(destroyed, MetisStatus::Ok, "the instance is destroyed");
    drop(loaded);
    eprintln!("[child] the library is unloaded, and the copied handle's drop call runs now");

    // SAFETY: none. This is the fault the case exists for: the drop function the
    // copy names was in the library that is no longer mapped.
    unsafe { ((*copied.vtable).drop)(copied.ctx) };
    unreachable!("a copied handle dropped after its library was unloaded faults the process")
}
