//! Spike-only cdylib plugin.
//!
//! Exports exactly one symbol, `metis_plugin_entry_v1`, returning a version 1
//! function table (D1, D8). Everything else in here is throwaway.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Mutex, Once, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use spike_abi::*;
use spike_payload::{CapabilityDescriptor, CapabilityKind, CapabilityList, EchoRequest, EchoResponse};

// --- process-wide plugin state ---------------------------------------------

/// The host table handed over at `create_instance`, used by worker threads.
static HOST_API: AtomicPtr<MetisHostApiV1> = AtomicPtr::new(std::ptr::null_mut());

/// Buffers this plugin allocated, keyed by pointer, valued by capacity.
fn live_buffers() -> &'static Mutex<HashMap<usize, usize>> {
    static LIVE: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static VIOLATIONS: AtomicUsize = AtomicUsize::new(0);
/// Instances already destroyed, so a double destroy is reported, not a double free.
static DESTROYED: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
static CANCELLED: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();
static PENDING: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();
static WORKERS: OnceLock<Mutex<Vec<JoinHandle<()>>>> = OnceLock::new();

fn destroyed() -> &'static Mutex<HashSet<usize>> {
    DESTROYED.get_or_init(|| Mutex::new(HashSet::new()))
}
fn cancelled() -> &'static Mutex<HashSet<u64>> {
    CANCELLED.get_or_init(|| Mutex::new(HashSet::new()))
}
fn pending() -> &'static Mutex<HashSet<u64>> {
    PENDING.get_or_init(|| Mutex::new(HashSet::new()))
}
fn workers() -> &'static Mutex<Vec<JoinHandle<()>>> {
    WORKERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn violation(what: &str) {
    VIOLATIONS.fetch_add(1, Ordering::SeqCst);
    log(METIS_LOG_ERROR, &format!("spike: protocol violation: {what}"));
}

fn log(level: i32, message: &str) {
    let api = HOST_API.load(Ordering::SeqCst);
    if api.is_null() {
        eprintln!("[plugin:{level}] {message}");
        return;
    }
    // SAFETY: HOST_API is only ever set from `create_instance`, where the host
    // guarantees the table outlives the instance.
    let api = unsafe { &*api };
    let log_fn = api.log;
    // SAFETY: the message slice is valid for the call, and the callback
    // contract is "borrowed for the duration of the call" (D6).
    unsafe { log_fn(api.user_data, level, message.as_ptr(), message.len()) }
}

// --- allocation (D6) -------------------------------------------------------

fn plugin_alloc(bytes: &[u8]) -> MetisBuffer {
    let mut vec: Vec<u8> = Vec::with_capacity(bytes.len());
    vec.extend_from_slice(bytes);
    let ptr = vec.as_mut_ptr();
    let len = vec.len();
    let cap = vec.capacity();
    mem::forget(vec);
    live_buffers().lock().unwrap().insert(ptr as usize, cap);
    ALLOCS.fetch_add(1, Ordering::SeqCst);
    MetisBuffer { ptr, len, cap, owner: METIS_BUFFER_OWNER_PLUGIN, reserved: 0 }
}

/// Copies a borrowed host buffer and validates its payload header.
unsafe fn read_host_payload(buffer: MetisBuffer) -> Result<Vec<u8>, MetisStatus> {
    if buffer.owner != METIS_BUFFER_OWNER_HOST {
        violation("plugin received a buffer it does not believe the host allocated");
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

const CAPABILITY_VERSIONS: [(&str, CapabilityKind, u32); 6] = [
    ("tool.echo", CapabilityKind::Tool, 1),
    ("tool.slow", CapabilityKind::Tool, 1),
    ("tool.cancelme", CapabilityKind::Tool, 1),
    ("tool.cancelrace", CapabilityKind::Tool, 1),
    ("tool.panic", CapabilityKind::Tool, 1),
    // Deliberately newer than anything the host supports, to exercise the
    // per-capability refusal of D5/D10.
    ("tool.future", CapabilityKind::Tool, 99),
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

fn spawn_worker(delay: Duration, request_id: u64, host_ptr: usize, honour_cancel: bool) {
    let handle = thread::spawn(move || {
        thread::sleep(delay);
        let host = unsafe { &*(host_ptr as *const MetisHostApiV1) };
        if honour_cancel && cancelled().lock().unwrap().contains(&request_id) {
            finish_pending(request_id);
            log(METIS_LOG_INFO, &format!("spike: dropped completion for cancelled request {request_id}"));
            return;
        }
        let stage = serde_json::to_vec(&serde_json::json!({ "stage": "working" })).expect("spike payload");
        let progress_buffer = plugin_alloc(&metis_encode_payload(&stage));
        let progress = host.progress;
        unsafe { progress(host.user_data, request_id, progress_buffer) };

        let body = serde_json::to_vec(&EchoResponse { echo: "async".to_string(), bytes: 5 })
            .expect("serialising a spike response cannot fail");
        let buffer = plugin_alloc(&metis_encode_payload(&body));
        finish_pending(request_id);
        let complete = host.complete;
        unsafe { complete(host.user_data, request_id, MetisStatus::Ok, buffer) }
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
                    log(METIS_LOG_WARN, &format!("spike: bad echo request: {err}"));
                    return Outcome::Fail(MetisStatus::InvalidArgument);
                }
            };
            let echo = request.echo.repeat(request.repeat as usize);
            let response = EchoResponse { bytes: echo.len(), echo };
            Outcome::Inline(serde_json::to_vec(&response).expect("serialising a spike response cannot fail"))
        }
        "tool.slow" => {
            pending().lock().unwrap().insert(request_id);
            spawn_worker(Duration::from_millis(150), request_id, host as usize, false);
            Outcome::Pending
        }
        "tool.cancelme" => {
            pending().lock().unwrap().insert(request_id);
            // Long enough that a cancel arriving right after the pending return
            // is always observed before the worker wants to complete.
            spawn_worker(Duration::from_millis(500), request_id, host as usize, true);
            Outcome::Pending
        }
        "tool.cancelrace" => {
            pending().lock().unwrap().insert(request_id);
            // Never honours cancellation: the completion arrives after a
            // successful cancel, so the host's own rule has to drop it (D12).
            spawn_worker(Duration::from_millis(120), request_id, host as usize, false);
            Outcome::Pending
        }
        "tool.panic" => panic!("spike: deliberate panic inside exported invoke"),
        other => {
            log(METIS_LOG_WARN, &format!("spike: unknown capability {other}"));
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
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out_instance.is_null() || host.is_null() {
            return MetisStatus::InvalidArgument;
        }
        HOST_API.store(host as *mut MetisHostApiV1, Ordering::SeqCst);
        let state = Box::new(PluginState { host });
        unsafe { *out_instance = Box::into_raw(state) as *mut c_void };
        log(METIS_LOG_INFO, "spike: instance created");
        let api = unsafe { &*host };
        let body = serde_json::to_vec(&serde_json::json!({ "ready": true })).expect("spike payload");
        let buffer = plugin_alloc(&metis_encode_payload(&body));
        let topic = "spike.ready";
        let emit_event = api.emit_event;
        unsafe { emit_event(api.user_data, topic.as_ptr(), topic.len(), buffer) };
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn plugin_destroy_instance(instance: *mut c_void) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| { unsafe { destroy_instance(instance) } }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe fn destroy_instance(instance: *mut c_void) -> MetisStatus {
    if instance.is_null() {
        return MetisStatus::InvalidArgument;
    }
    let mut set = destroyed().lock().unwrap();
    if !set.insert(instance as usize) {
        log(METIS_LOG_WARN, "spike: instance handle already destroyed");
        return MetisStatus::NotFound;
    }
    drop(set);
    let state = unsafe { Box::from_raw(instance as *mut PluginState) };
    // The instance carries the host table it was created with; logging through
    // it (rather than the process-wide pointer) is what a real plugin would do.
    let api = state.host;
    if !api.is_null() {
        let api = unsafe { &*api };
        let log_fn = api.log;
        let message = "spike: instance destroyed";
        unsafe { log_fn(api.user_data, METIS_LOG_INFO, message.as_ptr(), message.len()) };
    }
    MetisStatus::Ok
}

unsafe extern "C" fn plugin_list_capabilities(
    _instance: *mut c_void,
    out_payload: *mut MetisBuffer,
) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out_payload.is_null() {
            return MetisStatus::InvalidArgument;
        }
        let list = CapabilityList {
            capabilities: CAPABILITY_VERSIONS
                .iter()
                .map(|(name, kind, version)| CapabilityDescriptor {
                    name: (*name).to_string(),
                    kind: *kind,
                    version: *version,
                    schema: format!("{name}/v{version}"),
                })
                .collect(),
        };
        let body = serde_json::to_vec(&list).expect("serialising the capability list cannot fail");
        unsafe { *out_payload = plugin_alloc(&metis_encode_payload(&body)) };
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
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
    let result = catch_unwind(AssertUnwindSafe(|| {
        if capability.is_null() || out_payload.is_null() || host.is_null() {
            return MetisStatus::InvalidArgument;
        }
        // SAFETY: the host passes a UTF-8 capability name it owns for the call.
        let capability = match std::str::from_utf8(unsafe { slice::from_raw_parts(capability, capability_len) })
        {
            Ok(capability) => capability,
            Err(_) => return MetisStatus::InvalidArgument,
        };
        match unsafe { invoke_inner(capability, request_id, payload, host) } {
            Outcome::Inline(body) => {
                unsafe { *out_payload = plugin_alloc(&metis_encode_payload(&body)) };
                MetisStatus::Ok
            }
            Outcome::Pending => MetisStatus::Pending,
            Outcome::Fail(status) => status,
        }
    }));
    match result {
        Ok(status) => status,
        Err(_) => {
            // The panic hook has already logged the payload. Catching here is
            // the designed behaviour (D7), so this is not a protocol violation.
            log(METIS_LOG_WARN, "spike: panic turned into an internal-error status");
            MetisStatus::Internal
        }
    }
}

unsafe extern "C" fn plugin_cancel(_instance: *mut c_void, request_id: u64) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if !pending().lock().unwrap().contains(&request_id) {
            return MetisStatus::NotFound;
        }
        cancelled().lock().unwrap().insert(request_id);
        log(METIS_LOG_INFO, &format!("spike: cancel accepted for request {request_id}"));
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn plugin_release_buffer(buffer: MetisBuffer) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if buffer.owner != METIS_BUFFER_OWNER_PLUGIN {
            violation("release_buffer called with a buffer this plugin did not allocate");
            return MetisStatus::InvalidArgument;
        }
        if buffer.ptr.is_null() {
            // A zero-length buffer is the null buffer; nothing was allocated.
            return MetisStatus::Ok;
        }
        let cap = match live_buffers().lock().unwrap().remove(&(buffer.ptr as usize)) {
            Some(cap) => cap,
            None => {
                violation("release_buffer called with an unknown or already released pointer");
                return MetisStatus::NotFound;
            }
        };
        if cap != buffer.cap || buffer.len > buffer.cap {
            violation("release_buffer called with a buffer whose length or capacity changed");
        }
        // SAFETY: the pointer came from plugin_alloc, the capacity is the one
        // recorded at allocation time, and the map no longer holds it, so this
        // drop happens exactly once.
        drop(unsafe { Vec::from_raw_parts(buffer.ptr, buffer.len, cap) });
        FREES.fetch_add(1, Ordering::SeqCst);
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn plugin_shutdown(instance: *mut c_void) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // Join every worker before the reporting line, so the accounting below
        // is final and so the host may unload the library afterwards.
        let handles: Vec<JoinHandle<()>> = mem::take(&mut *workers().lock().unwrap());
        for handle in handles {
            let _ = handle.join();
        }
        let allocs = ALLOCS.load(Ordering::SeqCst);
        let frees = FREES.load(Ordering::SeqCst);
        let live = live_buffers().lock().unwrap().len();
        let violations = VIOLATIONS.load(Ordering::SeqCst);
        log(
            METIS_LOG_INFO,
            &format!(
                "SPIKE_ACCOUNT plugin allocs={allocs} frees={frees} live={live} violations={violations}"
            ),
        );
        unsafe { destroy_instance(instance) }
    }));
    result.unwrap_or(MetisStatus::Internal)
}

// --- panic hook ------------------------------------------------------------

fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let message = if let Some(text) = info.payload().downcast_ref::<&str>() {
                (*text).to_string()
            } else if let Some(text) = info.payload().downcast_ref::<String>() {
                text.clone()
            } else {
                "<non-string panic payload>".to_string()
            };
            log(METIS_LOG_ERROR, &format!("spike: panic caught at the boundary: {message}"));
            if std::env::var_os("SPIKE_SHOW_PANIC_BACKTRACE").is_some() {
                previous(info);
            }
        }));
    });
}
