//! The authoring surface's seams, driven in-process through the exported table
//! and the host adapter.
//!
//! `crates/plugin-fixture` drives the same adapter through a real `cdylib`, so
//! these tests cover what a second fixture library would be there for: a
//! deliberately inconsistent plugin, a capability that answers `Cancelled`
//! without a successful cancel, a lifecycle hook that fails, and the
//! cancellation token's waker.
//!
//! Request ids are namespaced per test (`1_0xx`, `2_0xx`, ...) because the
//! adapter's pending registry and its accounting ledger are process-wide: the
//! exported table is a `const fn` (D1, D8) with no user data to key them by.

#![cfg(feature = "authoring")]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use plugin_contract::abi::{MetisPluginTableV1, MetisStatus, metis_read_prefix};
use plugin_contract::authoring::{
    Call, CapabilityDescriptor, CapabilityKind, Host, HostAdapter, LogLevel, Plugin, PluginError,
    Shape, accounting, table,
};
use serde::{Deserialize, Serialize};

/// How long a test waits for a completion before calling it missing. The host
/// never waits unbounded (mechanics spike finding): a completion that will not
/// arrive looks exactly like a hang from the outside.
const WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Ping {
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Pong {
    text: String,
}

/// The payload shape every probe plugin reuses.
struct PingShape;

impl Shape for PingShape {
    type Request = Ping;
    type Response = Pong;
}

/// Serialises the cases in this binary.
///
/// The adapter's pending registry, its worker list, and its ledger are
/// process-wide: the exported table is a `const fn` (D1, D8), so request ids are
/// the only key they have, and one case's worker is joined by any case's
/// `shutdown` (D12). Running the cases one at a time is what makes assertions
/// about those counters mean what they say.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn descriptor(name: &str, kind: CapabilityKind) -> CapabilityDescriptor {
    CapabilityDescriptor {
        name: name.to_string(),
        kind,
        version: 1,
        schema: format!("{name}/v1"),
    }
}

fn ping(text: &str) -> Vec<u8> {
    serde_json::to_vec(&Ping {
        text: text.to_string(),
    })
    .expect("the request serialises")
}

fn pong(body: &[u8]) -> Pong {
    serde_json::from_slice(body).expect("the response deserialises")
}

/// The exported table for a probe plugin.
///
/// A plugin instantiates `table::<P>()` into a static and hands the host a
/// pointer to it; the host keeps that pointer for as long as the library is
/// loaded, and a worker thread releases buffers through it. Leaking one here is
/// how a test gives the host a pointer with the same lifetime.
fn table_for<P: Plugin + Default>() -> &'static MetisPluginTableV1 {
    Box::leak(Box::new(table::<P>()))
}

/// Creates an instance of `P` through the exported table, and attaches the
/// table to the host so plugin buffers can be released.
fn instance<P: Plugin + Default>(host: &HostAdapter) -> *mut std::ffi::c_void {
    let table = table_for::<P>();
    host.attach(table);
    let mut handle: *mut std::ffi::c_void = std::ptr::null_mut();
    let status = unsafe { (table.create_instance)(host.api(), &mut handle) };
    assert_eq!(status, MetisStatus::Ok, "the probe instance is created");
    handle
}

/// Waits for a log line, bounded, so a missing line fails instead of hanging.
fn wait_for_log(host: &HostAdapter, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if host.saw_log(needle) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The number of milliseconds inside the first log line containing `after Nms`.
fn logged_millis(host: &HostAdapter, needle: &str) -> Option<u128> {
    let line = host
        .logs()
        .into_iter()
        .map(|(_, line)| line)
        .find(|line| line.contains(needle))?;
    line.split("after ")
        .nth(1)?
        .split("ms")
        .next()?
        .trim()
        .parse()
        .ok()
}

/// The probe plugin: one arm per capability shape the adapter dispatches on.
#[derive(Default)]
struct Probe;

impl Plugin for Probe {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![
            descriptor(Self::ANSWER, CapabilityKind::Tool),
            descriptor(Self::DEFERRED, CapabilityKind::Provider),
            descriptor(Self::CANCELLABLE, CapabilityKind::Tool),
            descriptor(Self::FAULT, CapabilityKind::Tool),
        ]
    }

    const ANSWER: &'static str = "probe.answer";
    const DEFERRED: &'static str = "probe.deferred";
    const CANCELLABLE: &'static str = "probe.cancellable";
    const FAULT: &'static str = "probe.fault";

    type Answer = PingShape;
    type Deferred = PingShape;
    type Cancellable = PingShape;
    type Fault = PingShape;

    async fn answer(&self, call: Call, request: Ping) -> Result<Pong, PluginError> {
        call.emit_event(
            "probe.answered",
            &serde_json::json!({ "text": request.text }),
        )?;
        Ok(Pong { text: request.text })
    }

    async fn deferred(&self, call: Call, request: Ping) -> Result<Pong, PluginError> {
        call.progress(&serde_json::json!({ "stage": "working" }))?;
        std::thread::sleep(Duration::from_millis(40));
        DEFERRED_DONE.store(true, Ordering::SeqCst);
        Ok(Pong { text: request.text })
    }

    async fn cancellable(&self, _call: Call, request: Ping) -> Result<Pong, PluginError> {
        Ok(Pong { text: request.text })
    }

    async fn fault(&self, _call: Call, _request: Ping) -> Result<Pong, PluginError> {
        panic!("probe: deliberate panic inside a capability")
    }

    fn started(&self, host: &Host) -> Result<(), PluginError> {
        STARTED.fetch_add(1, Ordering::SeqCst);
        host.emit_event("probe.ready", &serde_json::json!({ "ready": true }))
    }

    fn stopped(&self, host: &Host) {
        STOPPED.fetch_add(1, Ordering::SeqCst);
        if DEFERRED_DONE.load(Ordering::SeqCst) {
            STOPPED_SAW_A_FINISHED_WORKER.store(true, Ordering::SeqCst);
        }
        host.log(LogLevel::Info, "probe: stopping");
    }
}

static STARTED: AtomicUsize = AtomicUsize::new(0);
static STOPPED: AtomicUsize = AtomicUsize::new(0);
static DEFERRED_DONE: AtomicBool = AtomicBool::new(false);
static STOPPED_SAW_A_FINISHED_WORKER: AtomicBool = AtomicBool::new(false);

/// A plugin whose `cancellable` arm parks on the token until a cancel arrives.
///
/// Nothing but the token can wake this future, so a completion is proof that
/// the cancel reached it.
#[derive(Default)]
struct Parked;

impl Plugin for Parked {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![descriptor(Self::CANCELLABLE, CapabilityKind::Tool)]
    }

    const CANCELLABLE: &'static str = "parked.cancellable";

    type Answer = PingShape;
    type Deferred = PingShape;
    type Cancellable = PingShape;
    type Fault = PingShape;

    async fn cancellable(&self, call: Call, request: Ping) -> Result<Pong, PluginError> {
        let restarted = Instant::now();
        // Parked on I/O: this future makes no progress until the cancel lands.
        call.cancellation().cancelled().await;
        call.log(
            LogLevel::Info,
            &format!(
                "parked: token observed after {}ms cancelled={}",
                restarted.elapsed().as_millis(),
                call.is_cancelled()
            ),
        );
        Ok(Pong { text: request.text })
    }
}

/// A plugin that declares a capability no arm answers.
#[derive(Default)]
struct Inconsistent;

impl Plugin for Inconsistent {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![descriptor("probe.missing", CapabilityKind::Tool)]
    }

    const ANSWER: &'static str = "inconsistent.answer";

    type Answer = PingShape;
    type Deferred = PingShape;
    type Cancellable = PingShape;
    type Fault = PingShape;
}

/// A plugin whose initialisation fails, which is what makes creation fail.
#[derive(Default)]
struct Unstartable;

impl Plugin for Unstartable {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![descriptor(Self::ANSWER, CapabilityKind::Tool)]
    }

    const ANSWER: &'static str = "unstartable.answer";

    type Answer = PingShape;
    type Deferred = PingShape;
    type Cancellable = PingShape;
    type Fault = PingShape;

    async fn answer(&self, _call: Call, request: Ping) -> Result<Pong, PluginError> {
        Ok(Pong { text: request.text })
    }

    fn started(&self, _host: &Host) -> Result<(), PluginError> {
        Err(PluginError::Internal("no configuration".to_string()))
    }
}

/// A plugin that gives up with `Cancelled` without ever being cancelled.
#[derive(Default)]
struct GivesUp;

impl Plugin for GivesUp {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![descriptor(Self::ANSWER, CapabilityKind::Tool)]
    }

    const ANSWER: &'static str = "givesup.answer";

    type Answer = PingShape;
    type Deferred = PingShape;
    type Cancellable = PingShape;
    type Fault = PingShape;

    async fn answer(&self, _call: Call, _request: Ping) -> Result<Pong, PluginError> {
        Err(PluginError::Cancelled)
    }
}

/// A plugin that parks on the token and then gives up.
#[derive(Default)]
struct GivesUpLate;

impl Plugin for GivesUpLate {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![descriptor(Self::CANCELLABLE, CapabilityKind::Tool)]
    }

    const CANCELLABLE: &'static str = "givesuplately.cancellable";

    type Answer = PingShape;
    type Deferred = PingShape;
    type Cancellable = PingShape;
    type Fault = PingShape;

    async fn cancellable(&self, call: Call, _request: Ping) -> Result<Pong, PluginError> {
        call.cancellation().cancelled().await;
        Err(PluginError::Cancelled)
    }
}

#[test]
fn the_adapter_lists_the_capabilities_the_plugin_declares() {
    let _serial = serial();
    let host = HostAdapter::new();
    let handle = instance::<Probe>(&host);

    let mut buffer = plugin_contract::abi::MetisBuffer::empty();
    let status = unsafe { (table_for::<Probe>().list_capabilities)(handle, &mut buffer) };
    let body = host
        .read_and_release(buffer)
        .expect("the capability list is a plugin buffer the host releases");
    let list: plugin_contract::authoring::CapabilityList =
        serde_json::from_slice(&body).expect("the capability list deserialises");

    assert_eq!(status, MetisStatus::Ok);
    assert_eq!(list.capabilities, Probe.capabilities());
}

#[test]
fn the_trait_adapter_always_completes_through_the_host_callback() {
    let _serial = serial();
    let table = table_for::<Probe>();
    let prefix = unsafe { metis_read_prefix(table) }.expect("the table carries a readable prefix");
    assert_eq!(prefix.size, std::mem::size_of::<MetisPluginTableV1>());

    let host = HostAdapter::new();
    let handle = instance::<Probe>(&host);
    let completion = host.register(1_001);

    // D15: the trait path is always pending, even for a capability whose future
    // is ready on its first poll. The ABI keeps its inline path for direct
    // implementers; nothing here reaches it.
    let status = unsafe { host.invoke(handle, Probe::ANSWER, 1_001, &ping("ping")) };
    assert_eq!(status, MetisStatus::Pending);

    let completed = completion
        .wait_timeout(WAIT)
        .expect("the completion arrives through the host callback");
    assert_eq!(completed.status, MetisStatus::Ok);
    assert_eq!(pong(&completed.body).text, "ping");
}

#[test]
fn the_awating_half_of_the_pending_path_resolves_through_the_callback() {
    let _serial = serial();
    let host = HostAdapter::new();
    let handle = instance::<Probe>(&host);
    let awaiting = host.register(1_002);

    let status = unsafe { host.invoke(handle, Probe::ANSWER, 1_002, &ping("awaited")) };
    assert_eq!(status, MetisStatus::Pending);
    // The host's future is resolved by the plugin's callback, driven by the
    // crate's own executor: no other thread polls it.
    let completed = plugin_contract::authoring::executor::block_on(awaiting);
    assert_eq!(completed.status, MetisStatus::Ok);
    assert_eq!(pong(&completed.body).text, "awaited");
}

#[test]
fn a_call_parked_on_the_token_is_woken_promptly_by_a_cancel() {
    let _serial = serial();
    let host = HostAdapter::new();
    let handle = instance::<Parked>(&host);
    let completion = host.register(2_001);

    let status = unsafe { host.invoke(handle, Parked::CANCELLABLE, 2_001, &ping("parked")) };
    assert_eq!(status, MetisStatus::Pending);
    // The arm is parked on I/O: no timer and no completion in sight.
    assert!(
        completion
            .wait_timeout(Duration::from_millis(150))
            .is_none(),
        "nothing but a cancel can wake the parked arm"
    );

    let cancel = unsafe { host.cancel(handle, 2_001) };
    assert_eq!(cancel, MetisStatus::Ok);
    assert!(
        wait_for_log(&host, "parked: token observed", WAIT),
        "the cancel woke the arm and it reported the token"
    );
    // Promptly means the token's waker, not the next poll: the arm has no
    // budget of its own, so a slow observation would be a lost wake.
    let observed = logged_millis(&host, "parked: token observed").expect("the arm logged its wait");
    assert!(
        observed < 250,
        "the arm observed the token after {observed}ms, which is not prompt"
    );
    // The poll convenience agrees with the token.
    assert!(host.saw_log("cancelled=true"));
}

#[test]
fn an_instance_is_created_only_when_the_declaration_has_an_arm_for_every_capability() {
    let _serial = serial();
    let host = HostAdapter::new();
    let table = table_for::<Inconsistent>();
    host.attach(table);
    let mut handle: *mut std::ffi::c_void = std::ptr::null_mut();

    let status = unsafe { (table.create_instance)(host.api(), &mut handle) };

    assert_ne!(
        status,
        MetisStatus::Ok,
        "a capability with no dispatch arm is a creation failure"
    );
    assert!(handle.is_null(), "no instance is handed out");
    assert!(
        host.saw_log("probe.missing"),
        "the log names the capability that has no arm: {:?}",
        host.logs()
    );

    // The same table creates an instance when the declaration lines up.
    let consistent = HostAdapter::new();
    let _ = instance::<Probe>(&consistent);
}

#[test]
fn a_failing_started_hook_fails_creation_and_yields_no_instance() {
    let _serial = serial();
    let host = HostAdapter::new();
    let table = table_for::<Unstartable>();
    host.attach(table);
    let mut handle: *mut std::ffi::c_void = std::ptr::null_mut();

    let status = unsafe { (table.create_instance)(host.api(), &mut handle) };

    assert_eq!(status, MetisStatus::Internal);
    assert!(handle.is_null(), "no instance is handed out");
    assert!(
        host.saw_log("instance start failed"),
        "the log names the failing hook: {:?}",
        host.logs()
    );
}

#[test]
fn the_lifecycle_hooks_run_once_each_and_stopped_runs_after_the_workers() {
    let _serial = serial();
    let host = HostAdapter::new();
    let started_before = STARTED.load(Ordering::SeqCst);
    let stopped_before = STOPPED.load(Ordering::SeqCst);
    let handle = instance::<Probe>(&host);
    assert_eq!(
        STARTED.load(Ordering::SeqCst),
        started_before + 1,
        "started runs at creation"
    );

    let completion = host.register(3_001);
    let status = unsafe { host.invoke(handle, Probe::DEFERRED, 3_001, &ping("deferred")) };
    assert_eq!(status, MetisStatus::Pending);
    assert_eq!(
        completion
            .wait_timeout(WAIT)
            .expect("the deferred arm completes")
            .status,
        MetisStatus::Ok
    );

    let shutdown = unsafe { (table_for::<Probe>().shutdown)(handle) };
    assert_eq!(shutdown, MetisStatus::Ok);
    assert_eq!(
        STOPPED.load(Ordering::SeqCst),
        stopped_before + 1,
        "stopped runs once, from shutdown"
    );
    assert!(
        STOPPED_SAW_A_FINISHED_WORKER.load(Ordering::SeqCst),
        "shutdown joined the worker before it ran the stopped hook (D12)"
    );
}

#[test]
fn an_event_from_started_and_from_a_capability_reaches_the_host() {
    let _serial = serial();
    let host = HostAdapter::new();
    let handle = instance::<Probe>(&host);
    // The lifecycle hook runs inside create_instance, so its event is already
    // there before any call.
    assert!(
        host.events()
            .iter()
            .any(|(topic, body)| topic == "probe.ready"
                && serde_json::from_slice::<serde_json::Value>(body).is_ok()),
        "started emits an event: {:?}",
        host.events()
    );

    let completion = host.register(4_001);
    let status = unsafe { host.invoke(handle, Probe::ANSWER, 4_001, &ping("eventful")) };
    assert_eq!(status, MetisStatus::Pending);
    assert_eq!(
        completion
            .wait_timeout(WAIT)
            .expect("the arm completes")
            .status,
        MetisStatus::Ok
    );
    assert!(
        host.events().iter().any(|(topic, body)| {
            topic == "probe.answered"
                && serde_json::from_slice::<serde_json::Value>(body)
                    .is_ok_and(|value| value["text"] == serde_json::json!("eventful"))
        }),
        "the capability emits an event: {:?}",
        host.events()
    );
}

#[test]
fn cancelled_without_a_successful_cancel_still_completes() {
    let _serial = serial();
    let host = HostAdapter::new();
    let handle = instance::<GivesUp>(&host);
    let completion = host.register(5_001);

    let status = unsafe { host.invoke(handle, GivesUp::ANSWER, 5_001, &ping("gave up")) };
    assert_eq!(status, MetisStatus::Pending);

    let completed = completion
        .wait_timeout(WAIT)
        .expect("the host is never parked on a completion that will not arrive");
    assert_eq!(completed.status, MetisStatus::Cancelled);
    assert!(
        !host.saw_log("completion suppressed for cancelled request 5_001"),
        "nothing is suppressed without a successful cancel"
    );
}

#[test]
fn cancelled_after_a_successful_cancel_completes_nothing() {
    let _serial = serial();
    let host = HostAdapter::new();
    let handle = instance::<GivesUpLate>(&host);
    let completion = host.register(6_001);
    let suppressed_before = accounting().suppressed;

    let status =
        unsafe { host.invoke(handle, GivesUpLate::CANCELLABLE, 6_001, &ping("giving up")) };
    assert_eq!(status, MetisStatus::Pending);
    let cancel = unsafe { host.cancel(handle, 6_001) };
    assert_eq!(cancel, MetisStatus::Ok);

    assert!(
        wait_for_log(&host, "completion suppressed", WAIT),
        "the adapter logged the suppression: {:?}",
        host.logs()
    );
    assert!(
        accounting().suppressed > suppressed_before,
        "the ledger's suppressed count moved"
    );
    // The host retired the record on the successful cancel, and the adapter
    // completed nothing, so the slot stays empty.
    assert!(
        completion
            .wait_timeout(Duration::from_millis(100))
            .is_none()
    );
}

#[test]
fn a_faulting_capability_completes_as_an_internal_error() {
    let _serial = serial();
    let host = HostAdapter::new();
    let handle = instance::<Probe>(&host);
    let completion = host.register(7_001);

    // D7: the panic happens inside the worker, after invoke has returned, so
    // the failure arrives as a completion rather than as the invoke status.
    let status = unsafe { host.invoke(handle, Probe::FAULT, 7_001, &ping("fault")) };
    assert_eq!(status, MetisStatus::Pending);
    let completed = completion
        .wait_timeout(WAIT)
        .expect("the worker caught the panic and completed the call");
    assert_eq!(completed.status, MetisStatus::Internal);
    assert!(host.saw_log("capability panicked"));
}
