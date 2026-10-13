//! The routed authoring trait, driven in process through the exported table.
//!
//! `crates/plugin-fixture` drives the same adapter through a real `cdylib`, so
//! these tests cover what a second fixture library would be there for: the
//! routed `call` dispatching more capabilities than the old four-arm vocabulary
//! could name, a future that resolves as `Cancelled`, a panic in a poll and a
//! panic in a destructor, a double drop, and a capability driven to completion
//! with no thread of the plugin's own.
//!
//! Request ids are namespaced per test because the adapter's cancellation
//! registry and its accounting ledger are process-wide: the exported table is a
//! `const fn` with no user data to key them by, so request ids are the only key
//! they have.

#![cfg(feature = "authoring")]

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use plugin_contract::abi::{
    METIS_BUFFER_OWNER_HOST, MetisBuffer, MetisFutureHandle, MetisHostApiV1, MetisPluginTableV1,
    MetisStatus, MetisWaker, metis_read_table,
};
use plugin_contract::authoring::{
    Call, CapabilityDescriptor, Host, LogLevel, Plugin, PluginError, Request, Response, accounting,
    schema_of, table,
};
use plugin_contract::payload::{metis_encode_payload, metis_payload_body};

/// Serialises the cases in this binary.
///
/// The adapter's cancellation registry and its ledger are process-wide, so one
/// case's shutdown could otherwise be observed by another.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn descriptor(name: &str) -> CapabilityDescriptor {
    CapabilityDescriptor::tool(name, 1, schema_of::<Body>(), schema_of::<Body>())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
struct Body {
    text: String,
}

fn request(capability: &str) -> Vec<u8> {
    serde_json::to_vec(&Body {
        text: capability.to_string(),
    })
    .expect("the request serialises")
}

// --- the host side of the boundary, in process ----------------------------

/// What the host callbacks recorded.
///
/// The host table lives inside this struct, and the struct itself is boxed, so
/// the pointer the plugin retains for its instance's lifetime stays valid: a
/// table on the stack of the call that created the instance would dangle the
/// moment that call returned.
struct HostState {
    table: *const MetisPluginTableV1,
    api: MetisHostApiV1,
    logs: Mutex<Vec<String>>,
    events: Mutex<Vec<String>>,
    services: Mutex<Vec<(String, u64, Vec<u8>)>>,
    released: AtomicUsize,
    waker_clones: AtomicUsize,
    waker_drops: AtomicUsize,
    callbacks: AtomicUsize,
}

// SAFETY: the state is reached only through the host table, whose callbacks are
// required to be thread-safe, and every field is behind a lock or an atomic.
unsafe impl Send for HostState {}
unsafe impl Sync for HostState {}

impl HostState {
    /// Boxes the host state and points the host table at it.
    fn new(table: *const MetisPluginTableV1) -> Box<HostState> {
        let mut host = Box::new(HostState {
            table,
            api: MetisHostApiV1 {
                size: size_of::<MetisHostApiV1>(),
                user_data: ptr::null_mut(),
                progress: host_progress,
                emit_event: host_emit_event,
                log: host_log,
                service_request: host_service_request,
            },
            logs: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
            services: Mutex::new(Vec::new()),
            released: AtomicUsize::new(0),
            waker_clones: AtomicUsize::new(0),
            waker_drops: AtomicUsize::new(0),
            callbacks: AtomicUsize::new(0),
        });
        // The box never moves while it is alive, so this pointer stays valid
        // for every callback the plugin makes.
        host.api.user_data = &*host as *const HostState as *mut c_void;
        host
    }

    fn api(&self) -> *const MetisHostApiV1 {
        &self.api
    }

    fn waker(&self) -> MetisWaker {
        MetisWaker {
            data: self as *const HostState as *mut c_void,
            vtable: &WAKER_VTABLE,
        }
    }

    /// Reads a plugin-owned buffer and hands it back through the table's
    /// `release_buffer`, which is the only way the host may free it.
    fn read_and_release(&self, buffer: MetisBuffer) -> Vec<u8> {
        if buffer.ptr.is_null() {
            return Vec::new();
        }
        // SAFETY: the buffer is a plugin allocation, valid until released.
        let bytes = unsafe { slice::from_raw_parts(buffer.ptr, buffer.len) }.to_vec();
        // SAFETY: the table is the live one this host was built for, and the
        // buffer is the one it just read.
        let status = unsafe { ((*self.table).release_buffer)(buffer) };
        assert_eq!(
            status,
            MetisStatus::Ok,
            "the plugin releases its own buffer"
        );
        self.released.fetch_add(1, Ordering::SeqCst);
        bytes
    }
}

unsafe extern "C" fn host_progress(user_data: *mut c_void, request_id: u64, payload: MetisBuffer) {
    let state = unsafe { &*(user_data as *const HostState) };
    state.callbacks.fetch_add(1, Ordering::SeqCst);
    let body = state.read_and_release(payload);
    state
        .logs
        .lock()
        .unwrap()
        .push(format!("progress {request_id} {}", body.len()));
}

unsafe extern "C" fn host_emit_event(
    user_data: *mut c_void,
    topic: *const u8,
    topic_len: usize,
    payload: MetisBuffer,
) {
    let state = unsafe { &*(user_data as *const HostState) };
    state.callbacks.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the topic is borrowed for the call.
    let topic = unsafe { slice::from_raw_parts(topic, topic_len) };
    let topic = String::from_utf8_lossy(topic).to_string();
    let _ = state.read_and_release(payload);
    state.events.lock().unwrap().push(topic);
}

unsafe extern "C" fn host_log(
    user_data: *mut c_void,
    level: i32,
    message: *const u8,
    message_len: usize,
) {
    let state = unsafe { &*(user_data as *const HostState) };
    state.callbacks.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the message is borrowed for the call.
    let message = unsafe { slice::from_raw_parts(message, message_len) };
    state
        .logs
        .lock()
        .unwrap()
        .push(format!("[{level}] {}", String::from_utf8_lossy(message)));
}

unsafe extern "C" fn host_service_request(
    user_data: *mut c_void,
    service: *const u8,
    service_len: usize,
    request_id: u64,
    payload: MetisBuffer,
) -> MetisStatus {
    let state = unsafe { &*(user_data as *const HostState) };
    state.callbacks.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the service name is borrowed for the call.
    let service = unsafe { slice::from_raw_parts(service, service_len) };
    let service = String::from_utf8_lossy(service).to_string();
    // The service payload is plugin-owned and released here, from inside the
    // callback, which is the reentrancy the contract allows.
    let body = state.read_and_release(payload);
    state
        .services
        .lock()
        .unwrap()
        .push((service, request_id, body));
    MetisStatus::Ok
}

// --- the host waker -------------------------------------------------------

unsafe extern "C" fn waker_clone(waker: *const MetisWaker) -> MetisWaker {
    // The fields are read one at a time: a clone must go through the vtable's
    // `clone` entry, which is what this is, rather than through a Rust copy.
    // SAFETY: the waker is the live one the host passed to the poll.
    let cloned = MetisWaker {
        data: unsafe { (*waker).data },
        vtable: unsafe { (*waker).vtable },
    };
    // SAFETY: the data is this host's state.
    let state = unsafe { &*(cloned.data as *const HostState) };
    state.waker_clones.fetch_add(1, Ordering::SeqCst);
    cloned
}

unsafe extern "C" fn waker_wake(waker: MetisWaker) {
    // SAFETY: the data is this host's state.
    let state = unsafe { &*(waker.data as *const HostState) };
    state.waker_drops.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn waker_wake_by_ref(waker: *const MetisWaker) {
    let _ = waker;
}

unsafe extern "C" fn waker_drop(waker: MetisWaker) {
    // SAFETY: the data is this host's state.
    let state = unsafe { &*(waker.data as *const HostState) };
    state.waker_drops.fetch_add(1, Ordering::SeqCst);
}

static WAKER_VTABLE: plugin_contract::abi::MetisWakerVtable =
    plugin_contract::abi::MetisWakerVtable {
        clone: waker_clone,
        wake: waker_wake,
        wake_by_ref: waker_wake_by_ref,
        drop: waker_drop,
    };

/// Polls one future once with this host's waker.
fn poll(state: &HostState, handle: MetisFutureHandle) -> (MetisStatus, MetisBuffer) {
    let mut out = MetisBuffer::empty();
    // SAFETY: the handle came from this table's `invoke_future`, and the waker
    // is this host's.
    let status = unsafe {
        let vtable = &*handle.vtable;
        (vtable.poll)(handle.ctx, state.waker(), &mut out)
    };
    (status, out)
}

fn drop_future(handle: MetisFutureHandle) -> MetisStatus {
    // SAFETY: the handle came from this table's `invoke_future`.
    unsafe {
        let vtable = &*handle.vtable;
        (vtable.drop)(handle.ctx)
    }
}

/// Drives one capability to its terminal status, releasing every ready buffer.
fn drive(
    table: &MetisPluginTableV1,
    state: &HostState,
    instance: *mut c_void,
    request_id: u64,
    capability: &str,
) -> MetisStatus {
    let body = request(capability);
    let encoded = metis_encode_payload(&body);
    let mut handle = MetisFutureHandle::null();
    // SAFETY: the instance is live, the request buffer is host-owned and valid
    // for this call, and the out parameter is a live local.
    let status = unsafe {
        (table.invoke_future)(
            instance,
            capability.as_ptr(),
            capability.len(),
            request_id,
            MetisBuffer {
                ptr: encoded.as_ptr() as *mut u8,
                len: encoded.len(),
                cap: encoded.len(),
                owner: METIS_BUFFER_OWNER_HOST,
                reserved: 0,
            },
            state.api(),
            &mut handle,
        )
    };
    assert_eq!(status, MetisStatus::Ok, "invoke_future starts a future");
    let mut polls = 0;
    let terminal = loop {
        let (status, out) = poll(state, handle);
        polls += 1;
        match status {
            MetisStatus::Pending => {
                assert!(polls < 8, "a ready capability must not stay pending");
            }
            MetisStatus::Ok => {
                let out = state.read_and_release(out);
                let _ = metis_payload_body(&out).expect("the response carries an envelope");
                break MetisStatus::Ok;
            }
            terminal => break terminal,
        }
    };
    assert_eq!(
        drop_future(handle),
        MetisStatus::Ok,
        "the future drops once"
    );
    terminal
}

// --- probe plugins --------------------------------------------------------

/// The six capabilities the routed trait dispatches by name. Six is the point:
/// the four-arm vocabulary could not name a fifth.
#[derive(Default)]
struct Many;

const CAPABILITIES: [&str; 6] = [
    "tool.one",
    "tool.two",
    "tool.three",
    "tool.four",
    "tool.five",
    "tool.six",
];

impl Plugin for Many {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        CAPABILITIES.iter().map(|name| descriptor(name)).collect()
    }

    async fn call(&self, _call: Call, request: Request<'_>) -> Result<Response, PluginError> {
        let body: Body = request.json()?;
        if !CAPABILITIES.contains(&request.capability) {
            return Err(PluginError::NotFound(request.capability.to_string()));
        }
        Response::json(&Body {
            text: format!("{}:{}", request.capability, body.text),
        })
    }
}

/// A capability that waits for a host service before answering.
#[derive(Default)]
struct Timer;

#[derive(Debug, serde::Deserialize)]
struct TimerAnswer {
    delay_ms: u64,
}

impl Plugin for Timer {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![descriptor("tool.timer")]
    }

    async fn call(&self, call: Call, _request: Request<'_>) -> Result<Response, PluginError> {
        call.progress(&Body {
            text: "waiting".to_string(),
        })?;
        let answer: TimerAnswer = call
            .service_request("timer", &serde_json::json!({ "delay_ms": 7 }))?
            .await?;
        Response::json(&Body {
            text: format!("waited {}", answer.delay_ms),
        })
    }
}

/// A capability that parks on its cancellation token.
#[derive(Default)]
struct Cancellable;

impl Plugin for Cancellable {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![descriptor("tool.cancellable")]
    }

    async fn call(&self, call: Call, _request: Request<'_>) -> Result<Response, PluginError> {
        call.cancellation().cancelled().await;
        Err(PluginError::Cancelled)
    }
}

/// A capability that panics in its poll.
#[derive(Default)]
struct Panics;

impl Plugin for Panics {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![descriptor("tool.panic")]
    }

    async fn call(&self, _call: Call, _request: Request<'_>) -> Result<Response, PluginError> {
        panic!("probe: a deliberate panic inside a poll")
    }
}

/// The guard whose destructor panics, so a panic lands in the future's drop.
struct PanicOnDrop;

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("probe: a deliberate panic inside a future destructor")
    }
}

/// A capability whose future is never ready and whose destructor panics.
#[derive(Default)]
struct DropPanics;

impl Plugin for DropPanics {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![descriptor("tool.panic_drop")]
    }

    async fn call(&self, _call: Call, _request: Request<'_>) -> Result<Response, PluginError> {
        let _guard = PanicOnDrop;
        std::future::pending::<()>().await;
        unreachable!("the guard is never dropped on this path")
    }
}

/// A plugin with two capabilities of the same name, which a routed `call`
/// cannot disambiguate.
#[derive(Default)]
struct Duplicate;

impl Plugin for Duplicate {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![descriptor("tool.same"), descriptor("tool.same")]
    }

    async fn call(&self, _call: Call, _request: Request<'_>) -> Result<Response, PluginError> {
        Response::json(&Body {
            text: "same".to_string(),
        })
    }
}

// --- helpers --------------------------------------------------------------

fn create(table: &MetisPluginTableV1, state: &HostState) -> (MetisStatus, *mut c_void) {
    let mut instance = ptr::null_mut();
    // SAFETY: the host table lives inside `state` and stays valid for the
    // instance's lifetime, and the out parameter is a live local.
    let status = unsafe { (table.create_instance)(state.api(), &mut instance) };
    (status, instance)
}

// --- the cases ------------------------------------------------------------

static MANY: MetisPluginTableV1 = table::<Many>();
static TIMER: MetisPluginTableV1 = table::<Timer>();
static CANCELLABLE: MetisPluginTableV1 = table::<Cancellable>();
static PANICS: MetisPluginTableV1 = table::<Panics>();
static DROP_PANICS: MetisPluginTableV1 = table::<DropPanics>();
static DUPLICATE: MetisPluginTableV1 = table::<Duplicate>();

#[test]
fn the_exported_table_is_the_revision_and_reads_back() {
    let _guard = serial();
    // SAFETY: the table is a static and never mutated.
    let read = unsafe { metis_read_table(&raw const MANY) }.expect("a current table is accepted");
    assert_eq!(read.abi_major, 1);
    assert_eq!(read.abi_minor, 1);
    assert_eq!(read.size, size_of::<MetisPluginTableV1>());
}

#[test]
fn more_than_four_capabilities_dispatch_by_name() {
    let _guard = serial();
    let state = HostState::new(&raw const MANY);
    let (status, instance) = create(&MANY, &state);
    assert_eq!(status, MetisStatus::Ok);
    // The ledger is process-wide, so this case compares against its own
    // starting point rather than against zero.
    let violations = accounting().violations;

    let mut listed = MetisBuffer::empty();
    // SAFETY: the instance is live and the out parameter is a live local.
    let status = unsafe { (MANY.list_capabilities)(instance, &mut listed) };
    assert_eq!(status, MetisStatus::Ok);
    let body = state.read_and_release(listed);
    let list: plugin_contract::authoring::CapabilityList =
        serde_json::from_slice(metis_payload_body(&body).expect("the list carries an envelope"))
            .expect("the list is JSON");
    assert_eq!(list.capabilities.len(), 6, "six capabilities, not four");

    for (index, capability) in CAPABILITIES.iter().enumerate() {
        let status = drive(&MANY, &state, instance, 1_001 + index as u64, capability);
        assert_eq!(status, MetisStatus::Ok, "{capability} answered");
    }

    // A name no capability declares is the plugin's own `not_found`, which is
    // what routing by name means.
    let status = drive(&MANY, &state, instance, 1_099, "tool.absent");
    assert_eq!(status, MetisStatus::NotFound);

    // SAFETY: the instance came from `create_instance`.
    let status = unsafe { (MANY.shutdown)(instance) };
    assert_eq!(status, MetisStatus::Ok);
    // SAFETY: the instance came from `create_instance` and has been shut down.
    let status = unsafe { (MANY.destroy_instance)(instance) };
    assert_eq!(status, MetisStatus::Ok);
    assert_eq!(accounting().live, 0, "every plugin buffer was released");
    assert_eq!(
        accounting().violations,
        violations,
        "routing by name adds no ownership violation"
    );
}

#[test]
fn a_host_service_round_trips_through_the_future() {
    let _guard = serial();
    let state = HostState::new(&raw const TIMER);
    let (status, instance) = create(&TIMER, &state);
    assert_eq!(status, MetisStatus::Ok);

    let body = request("tool.timer");
    let encoded = metis_encode_payload(&body);
    let mut handle = MetisFutureHandle::null();
    // SAFETY: the instance is live, the request buffer is host-owned and valid
    // for the call.
    let status = unsafe {
        (TIMER.invoke_future)(
            instance,
            b"tool.timer".as_ptr(),
            "tool.timer".len(),
            1_201,
            MetisBuffer {
                ptr: encoded.as_ptr() as *mut u8,
                len: encoded.len(),
                cap: encoded.len(),
                owner: METIS_BUFFER_OWNER_HOST,
                reserved: 0,
            },
            state.api(),
            &mut handle,
        )
    };
    assert_eq!(status, MetisStatus::Ok);

    // The first poll issues the service request and parks.
    let (status, _) = poll(&state, handle);
    assert_eq!(status, MetisStatus::Pending);
    let services = state.services.lock().unwrap().clone();
    assert_eq!(services.len(), 1, "the plugin asked for the timer");
    let (service, request_id, service_body) = &services[0];
    assert_eq!(service, "timer");
    assert_eq!(*request_id, 1_201);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            metis_payload_body(service_body).expect("the request carries an envelope")
        )
        .expect("the timer request is JSON")["delay_ms"],
        7
    );

    // The host answers with a host-owned, borrowed payload; the plugin copies
    // it, so the answer buffer can go out of scope immediately.
    let answer =
        metis_encode_payload(&serde_json::to_vec(&serde_json::json!({ "delay_ms": 7 })).unwrap());
    // SAFETY: the instance is live and the answer buffer is host-owned and
    // valid for the call.
    let status = unsafe {
        (TIMER.service_complete)(
            instance,
            1_201,
            MetisStatus::Ok,
            MetisBuffer {
                ptr: answer.as_ptr() as *mut u8,
                len: answer.len(),
                cap: answer.len(),
                owner: METIS_BUFFER_OWNER_HOST,
                reserved: 0,
            },
        )
    };
    assert_eq!(status, MetisStatus::Ok);

    let (status, out) = poll(&state, handle);
    assert_eq!(status, MetisStatus::Ok, "the woken future answers");
    let response = state.read_and_release(out);
    let body: Body = serde_json::from_slice(
        metis_payload_body(&response).expect("the response carries an envelope"),
    )
    .expect("the response is JSON");
    assert_eq!(body.text, "waited 7");
    assert_eq!(drop_future(handle), MetisStatus::Ok);
    assert_eq!(
        state.waker_clones.load(Ordering::SeqCst),
        state.waker_drops.load(Ordering::SeqCst),
        "every waker clone the plugin made was released through the host's own entry"
    );

    // SAFETY: the instance is live.
    let _ = unsafe { (TIMER.shutdown)(instance) };
    // SAFETY: the instance is live.
    let _ = unsafe { (TIMER.destroy_instance)(instance) };
    assert_eq!(accounting().live, 0);
}

#[test]
fn a_cancel_resolves_as_cancelled() {
    let _guard = serial();
    let state = HostState::new(&raw const CANCELLABLE);
    let (status, instance) = create(&CANCELLABLE, &state);
    assert_eq!(status, MetisStatus::Ok);

    let body = request("tool.cancellable");
    let encoded = metis_encode_payload(&body);
    let mut handle = MetisFutureHandle::null();
    // SAFETY: the instance is live and the buffer is host-owned.
    let status = unsafe {
        (CANCELLABLE.invoke_future)(
            instance,
            b"tool.cancellable".as_ptr(),
            "tool.cancellable".len(),
            1_301,
            MetisBuffer {
                ptr: encoded.as_ptr() as *mut u8,
                len: encoded.len(),
                cap: encoded.len(),
                owner: METIS_BUFFER_OWNER_HOST,
                reserved: 0,
            },
            state.api(),
            &mut handle,
        )
    };
    assert_eq!(status, MetisStatus::Ok);
    assert_eq!(poll(&state, handle).0, MetisStatus::Pending);

    // SAFETY: the instance is live and the request id is outstanding.
    let status = unsafe { (CANCELLABLE.cancel)(instance, 1_301) };
    assert_eq!(status, MetisStatus::Ok);
    let (status, _) = poll(&state, handle);
    assert_eq!(
        status,
        MetisStatus::Cancelled,
        "the future resolves as cancelled rather than being dropped mid-flight"
    );
    assert_eq!(drop_future(handle), MetisStatus::Ok);
    // A cancel for a request the plugin no longer holds is not found.
    // SAFETY: the instance is live.
    assert_eq!(
        unsafe { (CANCELLABLE.cancel)(instance, 1_301) },
        MetisStatus::NotFound
    );

    // SAFETY: the instance is live.
    let _ = unsafe { (CANCELLABLE.shutdown)(instance) };
    // SAFETY: the instance is live.
    let _ = unsafe { (CANCELLABLE.destroy_instance)(instance) };
}

#[test]
fn a_panic_in_a_poll_is_terminal_and_the_instance_keeps_serving() {
    let _guard = serial();
    let state = HostState::new(&raw const PANICS);
    let (status, instance) = create(&PANICS, &state);
    assert_eq!(status, MetisStatus::Ok);

    let before = accounting().violations;
    let status = drive(&PANICS, &state, instance, 1_401, "tool.panic");
    assert_eq!(
        status,
        MetisStatus::Internal,
        "a panic in a poll becomes a terminal internal status"
    );
    assert!(
        state.callbacks.load(Ordering::SeqCst) > 0,
        "the boundary logs the catch through the host"
    );
    assert_eq!(
        accounting().violations,
        before,
        "a panic is not a violation"
    );

    // SAFETY: the instance is live.
    let _ = unsafe { (PANICS.shutdown)(instance) };
    // SAFETY: the instance is live.
    let _ = unsafe { (PANICS.destroy_instance)(instance) };
}

#[test]
fn a_panic_in_a_future_destructor_is_contained() {
    let _guard = serial();
    let state = HostState::new(&raw const DROP_PANICS);
    let (status, instance) = create(&DROP_PANICS, &state);
    assert_eq!(status, MetisStatus::Ok);

    let body = request("tool.panic_drop");
    let encoded = metis_encode_payload(&body);
    let mut handle = MetisFutureHandle::null();
    // SAFETY: the instance is live and the buffer is host-owned.
    let status = unsafe {
        (DROP_PANICS.invoke_future)(
            instance,
            b"tool.panic_drop".as_ptr(),
            "tool.panic_drop".len(),
            1_501,
            MetisBuffer {
                ptr: encoded.as_ptr() as *mut u8,
                len: encoded.len(),
                cap: encoded.len(),
                owner: METIS_BUFFER_OWNER_HOST,
                reserved: 0,
            },
            state.api(),
            &mut handle,
        )
    };
    assert_eq!(status, MetisStatus::Ok);
    assert_eq!(poll(&state, handle).0, MetisStatus::Pending);

    // The destructor panics, and the shim contains it: the process is still
    // here, which is the whole point against `async-ffi`.
    let status = drop_future(handle);
    assert_eq!(status, MetisStatus::Internal);
    assert!(
        state
            .logs
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("authoring") || line.contains("[1]")),
        "the boundary keeps logging after the catch"
    );

    // SAFETY: the instance is live.
    let _ = unsafe { (DROP_PANICS.shutdown)(instance) };
    // SAFETY: the instance is live.
    let _ = unsafe { (DROP_PANICS.destroy_instance)(instance) };
}

#[test]
fn a_double_drop_is_reported_rather_than_a_second_free() {
    let _guard = serial();
    let state = HostState::new(&raw const PANICS);
    let (status, instance) = create(&PANICS, &state);
    assert_eq!(status, MetisStatus::Ok);
    let body = request("tool.panic");
    let encoded = metis_encode_payload(&body);
    let mut handle = MetisFutureHandle::null();
    // SAFETY: the instance is live and the buffer is host-owned.
    let status = unsafe {
        (PANICS.invoke_future)(
            instance,
            b"tool.panic".as_ptr(),
            "tool.panic".len(),
            1_601,
            MetisBuffer {
                ptr: encoded.as_ptr() as *mut u8,
                len: encoded.len(),
                cap: encoded.len(),
                owner: METIS_BUFFER_OWNER_HOST,
                reserved: 0,
            },
            state.api(),
            &mut handle,
        )
    };
    assert_eq!(status, MetisStatus::Ok);
    let _ = poll(&state, handle);
    assert_eq!(
        drop_future(handle),
        MetisStatus::Ok,
        "the panic was in the poll, so the drop itself is clean"
    );
    let violations = accounting().violations;
    assert_eq!(
        drop_future(handle),
        MetisStatus::NotFound,
        "a second drop is refused"
    );
    assert!(accounting().violations > violations, "and it is recorded");

    // SAFETY: the instance is live.
    let _ = unsafe { (PANICS.shutdown)(instance) };
    // SAFETY: the instance is live.
    let _ = unsafe { (PANICS.destroy_instance)(instance) };
}

#[test]
fn a_duplicate_capability_name_fails_at_creation() {
    let _guard = serial();
    let state = HostState::new(&raw const DUPLICATE);
    let (status, instance) = create(&DUPLICATE, &state);
    assert_eq!(status, MetisStatus::InvalidArgument);
    assert!(instance.is_null(), "a refused creation yields no instance");
}

#[test]
fn a_plugin_starts_and_stops_through_the_lifecycle_pair() {
    let _guard = serial();
    let state = HostState::new(&raw const MANY);
    let (status, instance) = create(&MANY, &state);
    assert_eq!(status, MetisStatus::Ok);
    // SAFETY: the instance is live.
    assert_eq!(unsafe { (MANY.shutdown)(instance) }, MetisStatus::Ok);
    // SAFETY: the instance is live.
    assert_eq!(
        unsafe { (MANY.destroy_instance)(instance) },
        MetisStatus::Ok
    );
    // A second destroy of the same handle is refused rather than freed twice.
    // SAFETY: the handle has already been released.
    assert_eq!(
        unsafe { (MANY.destroy_instance)(instance) },
        MetisStatus::NotFound
    );
}

/// A plugin whose `started` fails yields no instance.
#[derive(Default)]
struct StartsBadly;

impl Plugin for StartsBadly {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![descriptor("tool.never")]
    }

    async fn call(&self, _call: Call, _request: Request<'_>) -> Result<Response, PluginError> {
        Ok(Response::new(Vec::new()))
    }

    fn started(&self, _host: &Host) -> Result<(), PluginError> {
        Err(PluginError::PermissionDenied("probe: no".to_string()))
    }
}

static STARTS_BADLY: MetisPluginTableV1 = table::<StartsBadly>();

#[test]
fn a_failing_start_yields_no_instance() {
    let _guard = serial();
    let state = HostState::new(&raw const STARTS_BADLY);
    let (status, instance) = create(&STARTS_BADLY, &state);
    assert_eq!(status, MetisStatus::PermissionDenied);
    assert!(instance.is_null());
}

/// A plugin with nothing but the routed call, to prove the trait's shape is
/// still usable when an author overrides nothing else.
#[derive(Default)]
struct Minimal;

impl Plugin for Minimal {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![descriptor("tool.nothing")]
    }

    async fn call(&self, call: Call, _request: Request<'_>) -> Result<Response, PluginError> {
        call.log(LogLevel::Debug, "minimal");
        Ok(Response::new(Vec::new()))
    }
}

static MINIMAL: MetisPluginTableV1 = table::<Minimal>();

#[test]
fn the_default_hooks_are_optional() {
    let _guard = serial();
    let state = HostState::new(&raw const MINIMAL);
    let (status, instance) = create(&MINIMAL, &state);
    assert_eq!(status, MetisStatus::Ok);
    assert_eq!(
        drive(&MINIMAL, &state, instance, 1_701, "tool.nothing"),
        MetisStatus::Ok
    );
    // SAFETY: the instance is live.
    assert_eq!(unsafe { (MINIMAL.shutdown)(instance) }, MetisStatus::Ok);
    // SAFETY: the instance is live.
    assert_eq!(
        unsafe { (MINIMAL.destroy_instance)(instance) },
        MetisStatus::Ok
    );
}
