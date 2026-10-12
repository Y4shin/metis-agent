//! The plugin side of the hand-written translation.
//!
//! `MetisPluginTableV1` on one side, the throwaway `Plugin` trait on the other.
//! Every ABI concern lives here: the payload header (D4), buffer ownership (D6),
//! panic catching (D7), the pending registry (D11), cancellation (D12), and the
//! worker threads that let `invoke` return pending while the trait's future is
//! still running.

use std::collections::HashMap;
use std::ffi::c_void;
use std::future::Future;
use std::mem;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::slice;
use std::str;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};
use spike_abi::*;

use crate::alloc::{accounting, plugin_alloc, release_buffer, violation};
use crate::call::{Call, SendPtr};
use crate::executor::spawn;
use crate::payload::{
    CapabilityDescriptor, CapabilityKind, CapabilityList, CompletionRequest, EchoRequest,
};
use crate::traits::{Plugin, PluginError};

/// Capability names this adapter dispatches on. They are hand-maintained
/// literals and must agree with what the same plugin reports from
/// `capabilities()`; nothing checks that. See the findings, bend 7.
pub const CAPABILITY_ECHO: &str = "tool.echo";
pub const CAPABILITY_COMPLETE: &str = "provider.complete";
pub const CAPABILITY_UNCANCELLABLE: &str = "tool.uncancellable";

/// One live instance. The plugin is behind an `Arc` so a call can be moved onto
/// a worker thread without borrowing the instance handle.
struct Instance<P> {
    plugin: Arc<P>,
    host: SendPtr<MetisHostApiV1>,
}

/// The host table the plugin was created with, for logging from anywhere.
static HOST_API: AtomicPtr<MetisHostApiV1> = AtomicPtr::new(std::ptr::null_mut());
/// Request id to call, so `cancel` can rebuild the context (D12).
static PENDING: OnceLock<Mutex<HashMap<u64, Call>>> = OnceLock::new();
/// Worker threads, joined at shutdown before the host unloads the library.
static WORKERS: OnceLock<Mutex<Vec<JoinHandle<()>>>> = OnceLock::new();
/// Completions the plugin suppressed because the call was cancelled (D12).
static SUPPRESSED: AtomicUsize = AtomicUsize::new(0);

fn pending() -> &'static Mutex<HashMap<u64, Call>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn workers() -> &'static Mutex<Vec<JoinHandle<()>>> {
    WORKERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn log(level: i32, message: &str) {
    let api = HOST_API.load(Ordering::SeqCst);
    if api.is_null() {
        eprintln!("[spike-trait:{level}] {message}");
        return;
    }
    // SAFETY: HOST_API is only set from `create_instance`, where the host
    // guarantees the table outlives the instance.
    let api = unsafe { &*api };
    let log_fn = api.log;
    unsafe { log_fn(api.user_data, level, message.as_ptr(), message.len()) }
}

impl PluginError {
    /// The ABI status this authoring error maps onto (D7).
    fn status(&self) -> MetisStatus {
        match self {
            PluginError::InvalidArgument(_) => MetisStatus::InvalidArgument,
            PluginError::NotFound(_) => MetisStatus::NotFound,
            PluginError::PermissionDenied(_) => MetisStatus::PermissionDenied,
            PluginError::Unsupported(_) => MetisStatus::Unsupported,
            PluginError::Internal(_) => MetisStatus::Internal,
            PluginError::Cancelled => MetisStatus::Cancelled,
        }
    }
}

/// The whole plugin side of the translation as one `const fn`, which the plugin
/// crate instantiates into a static.
pub const fn table<P: Plugin + Default>() -> MetisPluginTableV1 {
    MetisPluginTableV1 {
        size: mem::size_of::<MetisPluginTableV1>(),
        abi_major: METIS_ABI_MAJOR,
        abi_minor: METIS_ABI_MINOR,
        create_instance: create_instance::<P>,
        destroy_instance: destroy_instance::<P>,
        list_capabilities: list_capabilities::<P>,
        invoke: invoke::<P>,
        cancel: cancel::<P>,
        release_buffer,
        shutdown: shutdown::<P>,
    }
}

/// Every dispatch arm produces this shape, because the three capability methods
/// return futures of three different opaque types.
type Work = Pin<Box<dyn Future<Output = Result<Vec<u8>, PluginError>> + Send + 'static>>;

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, PluginError> {
    serde_json::to_vec(value).map_err(|err| PluginError::Internal(format!("response payload: {err}")))
}

fn decode<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, MetisStatus> {
    serde_json::from_slice(body).map_err(|_| MetisStatus::InvalidArgument)
}

/// Copies a borrowed host buffer and validates its payload header. The trait
/// never sees the header, the owner tag, or the length.
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
    Ok(metis_payload_body(bytes)?.to_vec())
}

/// The one place the ABI's `invoke` knows about the trait's methods. Dispatch
/// is by capability name (D9); a per-method function table would put one arm
/// here into the ABI table instead.
fn dispatch<P: Plugin>(
    plugin: Arc<P>,
    call: Call,
    capability: &str,
    body: &[u8],
) -> Result<Work, MetisStatus> {
    match capability {
        "tool.echo" => {
            let request: EchoRequest = decode(body)?;
            Ok(Box::pin(async move { encode(&plugin.echo(call, request).await?) }))
        }
        "provider.complete" => {
            let request: CompletionRequest = decode(body)?;
            Ok(Box::pin(async move { encode(&plugin.complete(call, request).await?) }))
        }
        "tool.uncancellable" => {
            let request: EchoRequest = decode(body)?;
            Ok(Box::pin(async move {
                let response = plugin.uncancellable(call, request).await?;
                encode(&response)
            }))
        }
        _ => Err(MetisStatus::NotFound),
    }
}

/// The progress route handed to `Call`: serialised body in, plugin-allocated
/// buffer out, host callback.
fn progress_route(host: SendPtr<MetisHostApiV1>, request_id: u64) -> impl Fn(&[u8]) + Send + Sync {
    move |body: &[u8]| {
        let buffer = plugin_alloc(&metis_encode_payload(body));
        // SAFETY: the host table outlives the instance and callbacks may come
        // from any plugin thread (D12).
        let api = unsafe { &*host.get() };
        let progress = api.progress;
        unsafe { progress(api.user_data, request_id, buffer) };
    }
}

// --- exported adapters -----------------------------------------------------
//
// D7: every exported function wraps its body in `catch_unwind`, so a panic in
// the plugin becomes an internal-error status instead of unwinding across the
// boundary.

unsafe extern "C" fn create_instance<P: Plugin + Default>(
    host: *const MetisHostApiV1,
    out_instance: *mut *mut c_void,
) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if host.is_null() || out_instance.is_null() {
            return MetisStatus::InvalidArgument;
        }
        HOST_API.store(host as *mut MetisHostApiV1, Ordering::SeqCst);
        let state = Box::new(Instance { plugin: Arc::new(P::default()), host: SendPtr::new(host) });
        unsafe { *out_instance = Box::into_raw(state) as *mut c_void };
        log(METIS_LOG_INFO, "adapter: instance created");
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn destroy_instance<P: Plugin>(instance: *mut c_void) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() {
            return MetisStatus::InvalidArgument;
        }
        // SAFETY: the handle came from `create_instance` and D7's typed
        // instance handle is never dereferenced by the host.
        let state = unsafe { Box::from_raw(instance as *mut Instance<P>) };
        // The instance retains the host table it was created with (D1 as
        // amended); logging through it is what a real plugin would do.
        let api = unsafe { &*state.host.get() };
        let message = "adapter: instance destroyed";
        unsafe { (api.log)(api.user_data, METIS_LOG_INFO, message.as_ptr(), message.len()) };
        drop(state);
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn list_capabilities<P: Plugin>(
    instance: *mut c_void,
    out_payload: *mut MetisBuffer,
) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() || out_payload.is_null() {
            return MetisStatus::InvalidArgument;
        }
        let instance = unsafe { &*(instance as *const Instance<P>) };
        let list = CapabilityList { capabilities: instance.plugin.capabilities() };
        let body = match encode(&list) {
            Ok(body) => body,
            Err(error) => {
                log(METIS_LOG_ERROR, "adapter: capability list could not be encoded");
                return error.status();
            }
        };
        unsafe { *out_payload = plugin_alloc(&metis_encode_payload(&body)) };
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn invoke<P: Plugin>(
    instance: *mut c_void,
    capability: *const u8,
    capability_len: usize,
    request_id: u64,
    payload: MetisBuffer,
    host: *const MetisHostApiV1,
    out_payload: *mut MetisBuffer,
) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() || capability.is_null() || out_payload.is_null() || host.is_null() {
            return MetisStatus::InvalidArgument;
        }
        let instance = unsafe { &*(instance as *const Instance<P>) };
        // SAFETY: the host passes a UTF-8 capability name it owns for the call.
        let capability =
            match str::from_utf8(unsafe { slice::from_raw_parts(capability, capability_len) }) {
                Ok(name) => name,
                Err(_) => return MetisStatus::InvalidArgument,
            };
        let body = match unsafe { read_host_payload(payload) } {
            Ok(body) => body,
            Err(status) => return status,
        };

        let host = SendPtr::new(host);
        let call = Call::new(request_id, progress_route(host, request_id));
        // D11: the pending record exists before the trait is called, so an
        // immediate completion still finds it.
        pending().lock().unwrap().insert(request_id, call.clone());
        let work = match dispatch(Arc::clone(&instance.plugin), call.clone(), capability, &body) {
            Ok(work) => work,
            Err(status) => {
                // Nothing was started, so nothing is pending.
                pending().lock().unwrap().remove(&request_id);
                return status;
            }
        };

        let handle = spawn(async move {
            let outcome = work.await;
            pending().lock().unwrap().remove(&request_id);
            let (status, buffer) = match outcome {
                // The plugin gave up on a cancelled request, so the adapter
                // suppresses its completion (D12). No buffer was allocated, so
                // there is nothing to release. A plugin that answers
                // `Cancelled` without a cancel still gets a completion, so the
                // host can never be left waiting on a status it will not see.
                Err(PluginError::Cancelled) if call.is_cancelled() => {
                    SUPPRESSED.fetch_add(1, Ordering::SeqCst);
                    log(
                        METIS_LOG_INFO,
                        &format!("adapter: completion suppressed for cancelled request {request_id}"),
                    );
                    return;
                }
                Ok(body) => (MetisStatus::Ok, plugin_alloc(&metis_encode_payload(&body))),
                Err(error) => (error.status(), MetisBuffer::empty()),
            };
            // SAFETY: the host table outlives the instance.
            let api = unsafe { &*host.get() };
            let complete = api.complete;
            unsafe { complete(api.user_data, request_id, status, buffer) };
        });
        workers().lock().unwrap().push(handle);

        // The trait is async, so there is never a synchronous result to hand
        // back: this spike is always pending, and D11's inline shape is
        // deliberately left to the mechanics spike.
        MetisStatus::Pending
    }));
    match result {
        Ok(status) => status,
        Err(_) => {
            log(METIS_LOG_WARN, "adapter: panic turned into an internal-error status");
            MetisStatus::Internal
        }
    }
}

unsafe extern "C" fn cancel<P: Plugin>(instance: *mut c_void, request_id: u64) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() {
            return MetisStatus::InvalidArgument;
        }
        let instance = unsafe { &*(instance as *const Instance<P>) };
        let call = match pending().lock().unwrap().get(&request_id) {
            Some(call) => call.clone(),
            None => return MetisStatus::NotFound,
        };
        call.mark_cancelled();
        let status = match instance.plugin.cancel(call) {
            Ok(()) => {
                log(METIS_LOG_INFO, &format!("adapter: cancel accepted for request {request_id}"));
                MetisStatus::Ok
            }
            Err(error) => error.status(),
        };
        status
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn shutdown<P: Plugin>(_instance: *mut c_void) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // Join every worker first: the accounting below is then final, and the
        // host may unload the library afterwards.
        let handles: Vec<JoinHandle<()>> = mem::take(&mut *workers().lock().unwrap());
        for handle in handles {
            let _ = handle.join();
        }
        let (allocs, frees, live, violations) = accounting();
        let suppressed = SUPPRESSED.load(Ordering::SeqCst);
        log(
            METIS_LOG_INFO,
            &format!(
                "SPIKE_TRAIT_ACCOUNT allocs={allocs} frees={frees} live={live} violations={violations} suppressed={suppressed}"
            ),
        );
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

/// The adapter's capability names, so a plugin can describe the same set it
/// dispatches on without repeating the literals.
pub fn capability_descriptor(name: &str, kind: CapabilityKind, version: u32) -> CapabilityDescriptor {
    CapabilityDescriptor {
        name: name.to_string(),
        kind,
        version,
        schema: format!("{name}/v{version}"),
    }
}
