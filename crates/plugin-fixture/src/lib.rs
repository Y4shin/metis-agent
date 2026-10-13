//! The fixture plugin: a real cdylib that implements ABI major 1.
//!
//! It is test support, not product code. `crates/plugin-fixture/src/bin/abi-harness.rs`
//! loads it as a shared library and drives it, and nothing outside this crate
//! depends on it. It exists so the plugin boundary is proven by a real library
//! loaded through `plugin-contract`, not by a mock or by a second copy of the
//! ABI.
//!
//! What it implements, and why each piece is here:
//!
//! - `tool.echo`, an inline result: the synchronous half of D11.
//! - `tool.pending`, a result that arrives on a plugin worker thread through the
//!   host's `complete` callback, with a `progress` callback first: the deferred
//!   half of D11. It does not honour cancellation, which is what makes the host
//!   drop a completion that arrives after a cancel (D12).
//! - `tool.cancellable`, which suppresses its completion after a successful
//!   cancel: the other half of the drop path (D12).
//! - `tool.panic`, a deliberate panic inside an exported adapter, which becomes
//!   `internal` instead of unwinding across the boundary (D7).
//! - an allocation ledger, reported through the host `log` callback at shutdown:
//!   `allocs`, `frees`, `live`, `violations`, `suppressed`. Every allocation
//!   goes out to the host, and every release goes through this plugin's
//!   `release_buffer`, so the two ledgers can be compared (D6).
//!
//! The payload shapes live in `payload`, deliberately outside the C ABI
//! surface. Nothing in this file touches the boundary with a Rust `String`,
//! `Vec`, trait object or future.

pub mod payload;

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Mutex, Once, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use plugin_contract::abi::*;
use plugin_contract::payload::{metis_encode_payload, metis_payload_body};

use crate::payload::{
    CapabilityDescriptor, CapabilityKind, CapabilityList, EchoRequest, EchoResponse,
};

// --- process-wide plugin state ---------------------------------------------

/// The host table handed over at `create_instance`, used by worker threads.
static HOST_API: AtomicPtr<MetisHostApiV1> = AtomicPtr::new(std::ptr::null_mut());

/// Buffers this plugin allocated, keyed by pointer and valued by capacity.
fn live_buffers() -> &'static Mutex<HashMap<usize, usize>> {
    static LIVE: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The ledger the host compares its own counters against.
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static SUPPRESSED: AtomicUsize = AtomicUsize::new(0);
static VIOLATIONS: AtomicUsize = AtomicUsize::new(0);

/// Instances already destroyed, so a second destroy is reported rather than
/// becoming a double free.
fn destroyed() -> &'static Mutex<HashSet<usize>> {
    static DESTROYED: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
    DESTROYED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn cancelled() -> &'static Mutex<HashSet<u64>> {
    static CANCELLED: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();
    CANCELLED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn pending() -> &'static Mutex<HashSet<u64>> {
    static PENDING: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashSet::new()))
}

fn workers() -> &'static Mutex<Vec<JoinHandle<()>>> {
    static WORKERS: OnceLock<Mutex<Vec<JoinHandle<()>>>> = OnceLock::new();
    WORKERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn violation(what: &str) {
    VIOLATIONS.fetch_add(1, Ordering::SeqCst);
    log(
        METIS_LOG_ERROR,
        &format!("fixture: protocol violation: {what}"),
    );
}

fn log(level: i32, message: &str) {
    let api = HOST_API.load(Ordering::SeqCst);
    if api.is_null() {
        eprintln!("[fixture:{level}] {message}");
        return;
    }
    // SAFETY: HOST_API is only set from `create_instance`, where the host
    // guarantees the table outlives the instance.
    let api = unsafe { &*api };
    let log_fn = api.log;
    // SAFETY: the message slice is valid for the call, and the callback
    // contract is "borrowed for the duration of the call" (D6).
    unsafe { log_fn(api.user_data, level, message.as_ptr(), message.len()) }
}

/// Turns the body of an exported adapter into a status.
///
/// A panic inside the body is the designed failure mode (D7): it becomes
/// `internal` and the panic hook logs it, so it never unwinds across the
/// boundary.
fn guard(body: impl FnOnce() -> MetisStatus) -> MetisStatus {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(status) => status,
        Err(_) => MetisStatus::Internal,
    }
}

/// Logs a panic instead of letting the default hook print it.
///
/// The exported adapters catch panics (D7), so a panic inside this library is a
/// designed failure mode rather than a crash, and it belongs in the same log as
/// every other diagnostic. The hook is global to this library's copy of `std`,
/// which the dynamic loader gives the library its own instance of, so it does
/// not disturb the host's hook.
fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let message = match info.payload().downcast_ref::<&str>() {
                Some(message) => (*message).to_string(),
                None => match info.payload().downcast_ref::<String>() {
                    Some(message) => message.clone(),
                    None => "<non-string panic payload>".to_string(),
                },
            };
            log(
                METIS_LOG_ERROR,
                &format!("fixture: panic caught at the boundary: {message}"),
            );
        }));
    });
}

// --- allocation (D6) -------------------------------------------------------

/// Copies bytes into a buffer this plugin owns.
///
/// The capacity is recorded so `release_buffer` can rebuild the exact `Vec`,
/// and the allocation is counted so the ledger at shutdown is final.
fn plugin_alloc(bytes: &[u8]) -> MetisBuffer {
    let mut vec: Vec<u8> = Vec::with_capacity(bytes.len());
    vec.extend_from_slice(bytes);
    let ptr = vec.as_mut_ptr();
    let len = vec.len();
    let cap = vec.capacity();
    mem::forget(vec);
    live_buffers().lock().unwrap().insert(ptr as usize, cap);
    ALLOCS.fetch_add(1, Ordering::SeqCst);
    MetisBuffer {
        ptr,
        len,
        cap,
        owner: METIS_BUFFER_OWNER_PLUGIN,
        reserved: 0,
    }
}

/// Copies a borrowed host buffer and validates its payload header.
unsafe fn read_host_payload(buffer: MetisBuffer) -> Result<Vec<u8>, MetisStatus> {
    if buffer.owner != METIS_BUFFER_OWNER_HOST {
        violation("the plugin received a buffer it does not believe the host allocated");
        return Err(MetisStatus::InvalidArgument);
    }
    if buffer.ptr.is_null() {
        return Err(MetisStatus::InvalidArgument);
    }
    // SAFETY: the host owns the buffer and keeps it alive for the call (D6).
    let bytes = unsafe { slice::from_raw_parts(buffer.ptr, buffer.len) };
    let body = metis_payload_body(bytes)?;
    Ok(body.to_vec())
}

// --- instances -------------------------------------------------------------

struct PluginState {
    host: *const MetisHostApiV1,
}

// --- capability listing (D9) ----------------------------------------------

const CAPABILITIES: [(&str, CapabilityKind, u32); 4] = [
    ("tool.echo", CapabilityKind::Tool, 1),
    ("tool.pending", CapabilityKind::Tool, 1),
    ("tool.cancellable", CapabilityKind::Tool, 1),
    ("tool.panic", CapabilityKind::Tool, 1),
];

// --- invoke (D11) ----------------------------------------------------------

enum Outcome {
    Inline(Vec<u8>),
    Pending,
    Fail(MetisStatus),
}

fn finish_pending(request_id: u64) {
    pending().lock().unwrap().remove(&request_id);
}

/// Finishes a pending request on a worker thread.
///
/// `honour_cancel` decides which half of the drop path this capability
/// demonstrates: a capability that honours a cancel suppresses its completion,
/// and one that does not makes the host drop the completion that arrives anyway
/// (D12).
fn spawn_worker(delay: Duration, request_id: u64, host_ptr: usize, honour_cancel: bool) {
    let handle = thread::spawn(move || {
        thread::sleep(delay);
        // SAFETY: the host table outlives the instance, and `shutdown` joins
        // every worker before it returns, so the table is still alive here.
        let host = unsafe { &*(host_ptr as *const MetisHostApiV1) };

        if honour_cancel && cancelled().lock().unwrap().contains(&request_id) {
            finish_pending(request_id);
            SUPPRESSED.fetch_add(1, Ordering::SeqCst);
            log(
                METIS_LOG_INFO,
                &format!("fixture: suppressed completion for cancelled request {request_id}"),
            );
            return;
        }

        let stage = serde_json::to_vec(&serde_json::json!({ "stage": "working" }))
            .expect("the fixture payload serialises");
        let progress_buffer = plugin_alloc(&metis_encode_payload(&stage));
        let progress = host.progress;
        unsafe { progress(host.user_data, request_id, progress_buffer) };

        let body = serde_json::to_vec(&EchoResponse {
            echo: "async".to_string(),
            bytes: 5,
        })
        .expect("the fixture payload serialises");
        let buffer = plugin_alloc(&metis_encode_payload(&body));
        finish_pending(request_id);
        let complete = host.complete;
        unsafe { complete(host.user_data, request_id, MetisStatus::Ok, buffer) };
    });
    workers().lock().unwrap().push(handle);
}

unsafe fn invoke_inner(
    capability: &str,
    request_id: u64,
    payload: MetisBuffer,
    host: *const MetisHostApiV1,
) -> Outcome {
    let body = match unsafe { read_host_payload(payload) } {
        Ok(body) => body,
        Err(status) => return Outcome::Fail(status),
    };

    match capability {
        "tool.echo" => {
            let request: EchoRequest = match serde_json::from_slice(&body) {
                Ok(request) => request,
                Err(err) => {
                    log(METIS_LOG_WARN, &format!("fixture: bad echo request: {err}"));
                    return Outcome::Fail(MetisStatus::InvalidArgument);
                }
            };
            let echo = request.echo.repeat(request.repeat as usize);
            let response = EchoResponse {
                bytes: echo.len(),
                echo,
            };
            Outcome::Inline(serde_json::to_vec(&response).expect("the fixture payload serialises"))
        }
        "tool.pending" => {
            pending().lock().unwrap().insert(request_id);
            // Cancellation is not honoured here on purpose: the completion
            // arrives anyway, and the host has to drop it (D12).
            spawn_worker(Duration::from_millis(120), request_id, host as usize, false);
            Outcome::Pending
        }
        "tool.cancellable" => {
            pending().lock().unwrap().insert(request_id);
            // Long enough that a cancel arriving right after the pending return
            // is always observed before the worker wants to complete.
            spawn_worker(Duration::from_millis(300), request_id, host as usize, true);
            Outcome::Pending
        }
        "tool.panic" => panic!("fixture: deliberate panic inside an exported adapter"),
        other => {
            log(
                METIS_LOG_WARN,
                &format!("fixture: unknown capability {other}"),
            );
            Outcome::Fail(MetisStatus::NotFound)
        }
    }
}

// --- exported adapters (D7: every one wraps its body in catch_unwind) ------

#[unsafe(no_mangle)]
pub extern "C" fn metis_plugin_entry_v1() -> *const MetisPluginTableV1 {
    install_panic_hook();
    catch_unwind(AssertUnwindSafe(|| &raw const PLUGIN_TABLE_V1)).unwrap_or(std::ptr::null())
}

static PLUGIN_TABLE_V1: MetisPluginTableV1 = MetisPluginTableV1 {
    size: mem::size_of::<MetisPluginTableV1>(),
    abi_major: METIS_ABI_MAJOR,
    abi_minor: METIS_ABI_MINOR,
    create_instance: plugin_create_instance,
    destroy_instance: plugin_destroy_instance,
    list_capabilities: plugin_list_capabilities,
    invoke: plugin_invoke,
    cancel: plugin_cancel,
    release_buffer: plugin_release_buffer,
    shutdown: plugin_shutdown,
};

unsafe extern "C" fn plugin_create_instance(
    host: *const MetisHostApiV1,
    out_instance: *mut *mut c_void,
) -> MetisStatus {
    guard(|| {
        if out_instance.is_null() || host.is_null() {
            return MetisStatus::InvalidArgument;
        }
        HOST_API.store(host as *mut MetisHostApiV1, Ordering::SeqCst);
        let state = Box::new(PluginState { host });
        // SAFETY: the host owns the out parameter and gets the only handle to
        // this box, which `destroy_instance` takes back exactly once.
        unsafe { *out_instance = Box::into_raw(state) as *mut c_void };
        log(METIS_LOG_INFO, "fixture: instance created");
        // An event at creation time is how `emit_event` is exercised: the
        // callback belongs to the host table, not to a request (D1).
        let body = serde_json::to_vec(&serde_json::json!({ "ready": true }))
            .expect("the fixture payload serialises");
        let buffer = plugin_alloc(&metis_encode_payload(&body));
        // SAFETY: `host` is non-null and outlives the instance.
        let api = unsafe { &*host };
        let emit_event = api.emit_event;
        let topic = b"fixture.ready";
        unsafe { emit_event(api.user_data, topic.as_ptr(), topic.len(), buffer) };
        MetisStatus::Ok
    })
}

unsafe extern "C" fn plugin_destroy_instance(instance: *mut c_void) -> MetisStatus {
    guard(|| unsafe { destroy_instance(instance) })
}

unsafe fn destroy_instance(instance: *mut c_void) -> MetisStatus {
    if instance.is_null() {
        return MetisStatus::InvalidArgument;
    }
    if !destroyed().lock().unwrap().insert(instance as usize) {
        log(METIS_LOG_WARN, "fixture: instance handle already destroyed");
        return MetisStatus::NotFound;
    }
    // SAFETY: the handle came from Box::into_raw in `create_instance`, and the
    // set above makes this path run exactly once per handle.
    let state = unsafe { Box::from_raw(instance as *mut PluginState) };
    // Log through the instance's own host table rather than the process-wide
    // pointer, which is what a real plugin does when it says goodbye.
    let api = state.host;
    if !api.is_null() {
        let api = unsafe { &*api };
        let log_fn = api.log;
        let message = "fixture: instance destroyed";
        unsafe {
            log_fn(
                api.user_data,
                METIS_LOG_INFO,
                message.as_ptr(),
                message.len(),
            )
        };
    }
    MetisStatus::Ok
}

unsafe extern "C" fn plugin_list_capabilities(
    _instance: *mut c_void,
    out_payload: *mut MetisBuffer,
) -> MetisStatus {
    guard(|| {
        if out_payload.is_null() {
            return MetisStatus::InvalidArgument;
        }
        let list = CapabilityList {
            capabilities: CAPABILITIES
                .iter()
                .map(|(name, kind, version)| CapabilityDescriptor {
                    name: (*name).to_string(),
                    kind: *kind,
                    version: *version,
                    schema: format!("{name}/v{version}"),
                })
                .collect(),
        };
        let body = serde_json::to_vec(&list).expect("the fixture payload serialises");
        // SAFETY: the host owns the out parameter and the buffer is this
        // plugin's allocation, which the host releases (D6).
        unsafe { *out_payload = plugin_alloc(&metis_encode_payload(&body)) };
        MetisStatus::Ok
    })
}

unsafe extern "C" fn plugin_invoke(
    _instance: *mut c_void,
    capability: *const u8,
    capability_len: usize,
    request_id: u64,
    payload: MetisBuffer,
    host: *const MetisHostApiV1,
    out_payload: *mut MetisBuffer,
) -> MetisStatus {
    guard(|| {
        if capability.is_null() || out_payload.is_null() || host.is_null() {
            return MetisStatus::InvalidArgument;
        }
        // SAFETY: the host passes a UTF-8 capability name it owns for the
        // duration of the call.
        let capability =
            match std::str::from_utf8(unsafe { slice::from_raw_parts(capability, capability_len) })
            {
                Ok(capability) => capability,
                Err(_) => return MetisStatus::InvalidArgument,
            };
        match unsafe { invoke_inner(capability, request_id, payload, host) } {
            Outcome::Inline(body) => {
                // SAFETY: the host owns the out parameter.
                unsafe { *out_payload = plugin_alloc(&metis_encode_payload(&body)) };
                MetisStatus::Ok
            }
            // A pending result leaves the out buffer null: "ok with no result"
            // would be ambiguous (D11).
            Outcome::Pending => MetisStatus::Pending,
            Outcome::Fail(status) => status,
        }
    })
}

unsafe extern "C" fn plugin_cancel(_instance: *mut c_void, request_id: u64) -> MetisStatus {
    guard(|| {
        if !pending().lock().unwrap().contains(&request_id) {
            return MetisStatus::NotFound;
        }
        cancelled().lock().unwrap().insert(request_id);
        log(
            METIS_LOG_INFO,
            &format!("fixture: cancel accepted for request {request_id}"),
        );
        MetisStatus::Ok
    })
}

unsafe extern "C" fn plugin_release_buffer(buffer: MetisBuffer) -> MetisStatus {
    guard(|| {
        if buffer.owner != METIS_BUFFER_OWNER_PLUGIN {
            violation("release_buffer was called with a buffer this plugin did not allocate");
            return MetisStatus::InvalidArgument;
        }
        if buffer.ptr.is_null() {
            // An empty buffer allocated nothing, so there is nothing to free.
            return MetisStatus::Ok;
        }
        let cap = match live_buffers()
            .lock()
            .unwrap()
            .remove(&(buffer.ptr as usize))
        {
            Some(cap) => cap,
            None => {
                violation("release_buffer was called with an unknown or already released pointer");
                return MetisStatus::NotFound;
            }
        };
        if cap != buffer.cap || buffer.len > buffer.cap {
            violation("release_buffer was called with a buffer whose length or capacity changed");
        }
        // SAFETY: the pointer came from `plugin_alloc`, the capacity is the one
        // recorded at allocation time, and the map no longer holds it, so this
        // drop happens exactly once.
        drop(unsafe { Vec::from_raw_parts(buffer.ptr, buffer.len, cap) });
        FREES.fetch_add(1, Ordering::SeqCst);
        MetisStatus::Ok
    })
}

unsafe extern "C" fn plugin_shutdown(instance: *mut c_void) -> MetisStatus {
    guard(|| {
        // Join every worker before the ledger line, so the accounting is final
        // and the host may unload the library afterwards (D12).
        let handles: Vec<JoinHandle<()>> = mem::take(&mut *workers().lock().unwrap());
        for handle in handles {
            let _ = handle.join();
        }
        let allocs = ALLOCS.load(Ordering::SeqCst);
        let frees = FREES.load(Ordering::SeqCst);
        let live = live_buffers().lock().unwrap().len();
        let violations = VIOLATIONS.load(Ordering::SeqCst);
        let suppressed = SUPPRESSED.load(Ordering::SeqCst);
        log(
            METIS_LOG_INFO,
            &format!(
                "FIXTURE_ACCOUNT allocs={allocs} frees={frees} live={live} violations={violations} suppressed={suppressed}"
            ),
        );
        unsafe { destroy_instance(instance) }
    })
}
