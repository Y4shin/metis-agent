//! The plugin side of the translation: `MetisPluginTableV1` on one side, the
//! `Plugin` trait on the other.
//!
//! Everything the ABI forces onto a plugin lives here: the payload envelope
//! (D4), buffer ownership (D6), panic catching (D7), the pending registry and
//! the worker join (D11, D12), the cancellation registry that lets `cancel`
//! reach the call a capability is parked on (D15), and the check that a
//! plugin's capability declaration lines up with the arms this module
//! dispatches on (D9).
//!
//! The trait path is always pending (D15). The ABI keeps its inline path for a
//! plugin that implements the table directly, which is a different
//! implementation of the same entries and not something this adapter reaches.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::future::Future;
use std::mem;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::slice;
use std::str;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::abi::{
    METIS_ABI_MAJOR, METIS_ABI_MINOR, METIS_BUFFER_OWNER_HOST, MetisBuffer, MetisHostApiV1,
    MetisPluginTableV1, MetisStatus,
};
use crate::authoring::alloc::{note_suppressed, plugin_alloc, release_buffer, violation};
use crate::authoring::call::{Call, CancellationToken, Host, LogLevel};
use crate::authoring::capability::{CapabilityList, Shape};
use crate::authoring::executor;
use crate::authoring::traits::{Plugin, PluginError};
use crate::payload::{metis_encode_payload, metis_payload_body};

/// One live instance.
///
/// The plugin sits behind an `Arc` so a call can move onto a worker thread
/// without borrowing the handle, and the host table is retained because
/// callbacks may come from any plugin thread (D1 as amended, D12).
struct Instance<P: Plugin> {
    plugin: Arc<P>,
    host: Host,
}

/// Request id to the call, so `cancel` reaches the same token the capability
/// method is parked on (D12, D15).
///
/// Process-wide, because the table is a `const fn` (D1, D8) with no user data
/// to key it by; request ids are host-supplied and unique, which is what keeps
/// two instances of one plugin from colliding.
fn pending() -> &'static Mutex<HashMap<u64, Call>> {
    static PENDING: OnceLock<Mutex<HashMap<u64, Call>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Worker handles, joined at `shutdown` before the host may unload the library
/// (D12).
fn workers() -> &'static Mutex<Vec<JoinHandle<()>>> {
    static WORKERS: OnceLock<Mutex<Vec<JoinHandle<()>>>> = OnceLock::new();
    WORKERS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Instance handles this plugin has already taken back, so a second release is
/// reported rather than becoming a double free.
///
/// `shutdown` releases the instance, because it has joined every worker and
/// told the plugin it is stopping; the host's `destroy_instance` after it sees
/// a handle this plugin has already collected.
fn released_instances() -> &'static Mutex<HashSet<usize>> {
    static RELEASED: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
    RELEASED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// The whole plugin side of the translation as one `const fn`, which the plugin
/// instantiates into a static and exports through `metis_plugin_entry_v1`.
///
/// That is the spike's six-line wiring, and the reason no macro is justified at
/// this size.
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

/// Every dispatch arm produces this shape, because the four capability methods
/// return four different opaque future types.
type Work = Pin<Box<dyn Future<Output = Result<Vec<u8>, PluginError>> + Send + 'static>>;

/// The status an authoring error maps onto (D7).
fn metis_status(error: &PluginError) -> MetisStatus {
    match error {
        PluginError::InvalidArgument(_) => MetisStatus::InvalidArgument,
        PluginError::NotFound(_) => MetisStatus::NotFound,
        PluginError::PermissionDenied(_) => MetisStatus::PermissionDenied,
        PluginError::Unsupported(_) => MetisStatus::Unsupported,
        PluginError::Internal(_) => MetisStatus::Internal,
        PluginError::Cancelled => MetisStatus::Cancelled,
    }
}

/// A plugin buffer holding an envelope around `body` (D4, D6).
fn plugin_buffer(body: &[u8]) -> MetisBuffer {
    plugin_alloc(&metis_encode_payload(body))
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, PluginError> {
    serde_json::to_vec(value)
        .map_err(|err| PluginError::Internal(format!("response payload: {err}")))
}

fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, MetisStatus> {
    serde_json::from_slice(body).map_err(|_| MetisStatus::InvalidArgument)
}

/// Copies a borrowed host buffer and validates its payload envelope.
///
/// The trait never sees the header, the owner tag, or the length.
///
/// # Safety
///
/// `buffer` must be the buffer the host passed to `invoke`, valid for the
/// duration of the call (D6).
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
    Ok(metis_payload_body(bytes)?.to_vec())
}

/// The arms this adapter dispatches on: the one place the ABI's `invoke` knows
/// about the trait's capability methods.
fn arm_names<P: Plugin>() -> [&'static str; 4] {
    [P::ANSWER, P::DEFERRED, P::CANCELLABLE, P::FAULT]
}

/// Checks the plugin's capability declaration against the dispatch arms.
///
/// The two are separate declarations: `capabilities()` is what the host is
/// told, and the arm names are what the adapter answers. A descriptor whose
/// name no arm answers would otherwise surface as `not_found` at call time, so
/// it is a creation failure instead (D9).
fn check_declaration<P: Plugin>(plugin: &P) -> Result<(), String> {
    let arms = arm_names::<P>();
    for (index, name) in arms.iter().enumerate() {
        if name.is_empty() {
            // An arm this plugin does not implement. Nothing advertises it, so
            // no host reaches it.
            continue;
        }
        if arms[..index].contains(name) {
            return Err(format!("two capability arms claim the name {name}"));
        }
    }
    for descriptor in plugin.capabilities() {
        if !arms.contains(&descriptor.name.as_str()) {
            return Err(format!(
                "capability {} has a descriptor but no dispatch arm (arms: {arms:?})",
                descriptor.name
            ));
        }
    }
    Ok(())
}

/// The one place the ABI's `invoke` reaches a trait method.
///
/// Dispatch is by capability name (D9). A name no arm answers is `NotFound`,
/// which the creation-time check makes unreachable for a declared capability.
fn dispatch<P: Plugin>(
    plugin: Arc<P>,
    call: Call,
    capability: &str,
    body: &[u8],
) -> Result<Work, MetisStatus> {
    if capability == P::ANSWER {
        let request: <P::Answer as Shape>::Request = decode(body)?;
        return Ok(Box::pin(async move {
            encode(&plugin.answer(call, request).await?)
        }));
    }
    if capability == P::DEFERRED {
        let request: <P::Deferred as Shape>::Request = decode(body)?;
        return Ok(Box::pin(async move {
            encode(&plugin.deferred(call, request).await?)
        }));
    }
    if capability == P::CANCELLABLE {
        let request: <P::Cancellable as Shape>::Request = decode(body)?;
        return Ok(Box::pin(async move {
            encode(&plugin.cancellable(call, request).await?)
        }));
    }
    if capability == P::FAULT {
        let request: <P::Fault as Shape>::Request = decode(body)?;
        return Ok(Box::pin(async move {
            encode(&plugin.fault(call, request).await?)
        }));
    }
    Err(MetisStatus::NotFound)
}

// --- exported adapters -----------------------------------------------------
//
// Every exported function catches panics (D7), so a panic in a plugin becomes a
// status instead of unwinding across the boundary.

unsafe extern "C" fn create_instance<P: Plugin + Default>(
    host: *const MetisHostApiV1,
    out_instance: *mut *mut c_void,
) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if host.is_null() || out_instance.is_null() {
            return MetisStatus::InvalidArgument;
        }
        let host = Host::from_raw(host);
        let plugin = P::default();
        // D9: the declaration and the dispatch arms are separate declarations,
        // and this is where they are made to agree.
        if let Err(mismatch) = check_declaration::<P>(&plugin) {
            host.log(LogLevel::Error, &format!("authoring: {mismatch}"));
            return MetisStatus::InvalidArgument;
        }
        let instance = Box::new(Instance {
            plugin: Arc::new(plugin),
            host,
        });
        // `Default` construction cannot fail or take configuration, so this is
        // where initialisation reports failure (spike finding 8).
        if let Err(error) = instance.plugin.started(&instance.host) {
            instance.host.log(
                LogLevel::Error,
                &format!("authoring: instance start failed: {error}"),
            );
            drop(instance);
            return metis_status(&error);
        }
        let raw = Box::into_raw(instance);
        // A released handle's address can be handed to the next instance by the
        // allocator; creating an instance there makes it live again.
        released_instances().lock().unwrap().remove(&(raw as usize));
        // SAFETY: the host owns the out parameter and gets the only handle to
        // this box, which `destroy_instance` takes back exactly once.
        unsafe { *out_instance = raw as *mut c_void };
        instance_log(&host, "authoring: instance created");
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

fn instance_log(host: &Host, message: &str) {
    host.log(LogLevel::Info, message);
}

unsafe extern "C" fn destroy_instance<P: Plugin>(instance: *mut c_void) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() {
            return MetisStatus::InvalidArgument;
        }
        if !released_instances()
            .lock()
            .unwrap()
            .insert(instance as usize)
        {
            return MetisStatus::NotFound;
        }
        // SAFETY: the handle came from `create_instance`, and the insert above
        // makes this path run exactly once per handle.
        let state = unsafe { Box::from_raw(instance as *mut Instance<P>) };
        instance_log(&state.host, "authoring: instance released");
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
        // SAFETY: the handle is the live instance for this call.
        let state = unsafe { &*(instance as *const Instance<P>) };
        let list = CapabilityList {
            capabilities: state.plugin.capabilities(),
        };
        let body = match encode(&list) {
            Ok(body) => body,
            Err(error) => {
                state.host.log(
                    LogLevel::Error,
                    "authoring: the capability list could not be encoded",
                );
                return metis_status(&error);
            }
        };
        // SAFETY: the host owns the out parameter and releases the buffer.
        unsafe { *out_payload = plugin_buffer(&body) };
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
        let host = Host::from_raw(host);
        // SAFETY: the host passes a UTF-8 capability name it owns for the call.
        let capability =
            match str::from_utf8(unsafe { slice::from_raw_parts(capability, capability_len) }) {
                Ok(name) => name,
                Err(_) => {
                    host.log(
                        LogLevel::Warn,
                        "authoring: a capability name arrived that is not UTF-8",
                    );
                    return MetisStatus::InvalidArgument;
                }
            };
        let body = match unsafe { read_host_payload(payload) } {
            Ok(body) => body,
            Err(status) => return status,
        };
        // SAFETY: the handle is the live instance this capability belongs to.
        let state = unsafe { &*(instance as *const Instance<P>) };

        // The request is decoded, and the arm resolved, before anything is
        // registered or spawned: a malformed request or an unknown capability
        // is an ordinary error return that leaves nothing pending.
        let call = Call::new(request_id, host, CancellationToken::new());
        let work = match dispatch::<P>(Arc::clone(&state.plugin), call.clone(), capability, &body) {
            Ok(work) => work,
            Err(status) => {
                host.log(
                    LogLevel::Warn,
                    &format!(
                        "authoring: invoke for {capability} returned {}",
                        status.as_str()
                    ),
                );
                return status;
            }
        };

        // D11: the pending record exists before the future can run, so a cancel
        // that races the completion still finds the call.
        pending().lock().unwrap().insert(request_id, call.clone());
        let handle = executor::spawn(async move {
            // D7: the worker is a boundary. A panic inside the capability
            // becomes an internal-error completion instead of a worker that
            // dies without answering the host.
            let outcome = match executor::catch_panic(work).await {
                Ok(outcome) => outcome,
                Err(panic) => {
                    let detail = panic_message(&panic);
                    host.log(
                        LogLevel::Error,
                        &format!("authoring: the {capability} capability panicked: {detail}"),
                    );
                    pending().lock().unwrap().remove(&request_id);
                    host.complete(request_id, MetisStatus::Internal, MetisBuffer::empty());
                    return;
                }
            };
            pending().lock().unwrap().remove(&request_id);
            let (status, payload) = match outcome {
                // D12: the plugin gave up on a call the host cancelled, so its
                // completion is suppressed. Nothing was allocated, so there is
                // nothing to release either. The same value without a
                // successful cancel still completes, so a host is never parked
                // on a completion that will not arrive.
                Err(PluginError::Cancelled) if call.is_cancelled() => {
                    note_suppressed();
                    host.log(
                        LogLevel::Info,
                        &format!(
                            "authoring: completion suppressed for cancelled request {request_id}"
                        ),
                    );
                    return;
                }
                Ok(body) => (MetisStatus::Ok, plugin_buffer(&body)),
                Err(error) => (metis_status(&error), MetisBuffer::empty()),
            };
            host.complete(request_id, status, payload);
        });
        workers().lock().unwrap().push(handle);

        // D15: the trait is async, so there is never a synchronous result to
        // hand back. The ABI keeps its inline path for a plugin that implements
        // the table directly.
        MetisStatus::Pending
    }));
    match result {
        Ok(status) => status,
        Err(_) => {
            // Nothing here has the host pointer in hand any more, and the panic
            // is a plugin bug rather than a protocol event, so it goes to the
            // process's own diagnostics.
            eprintln!("plugin-contract: a panic escaped an authoring adapter");
            MetisStatus::Internal
        }
    }
}

unsafe extern "C" fn cancel<P: Plugin>(instance: *mut c_void, request_id: u64) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() {
            return MetisStatus::InvalidArgument;
        }
        // SAFETY: the handle is the live instance for this call.
        let state = unsafe { &*(instance as *const Instance<P>) };
        let call = match pending().lock().unwrap().get(&request_id) {
            Some(call) => call.clone(),
            None => return MetisStatus::NotFound,
        };
        // The token fires before the plugin's own cancel method runs, so an arm
        // parked on it is woken whether or not the method says anything (D15).
        call.mark_cancelled();
        match state.plugin.cancel(call) {
            Ok(()) => {
                state.host.log(
                    LogLevel::Info,
                    &format!("authoring: cancel accepted for request {request_id}"),
                );
                MetisStatus::Ok
            }
            Err(error) => metis_status(&error),
        }
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn shutdown<P: Plugin>(instance: *mut c_void) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() {
            return MetisStatus::InvalidArgument;
        }
        if released_instances()
            .lock()
            .unwrap()
            .contains(&(instance as usize))
        {
            return MetisStatus::NotFound;
        }
        // D12: join every worker first, so no callback can arrive after this
        // returns and the ledger the plugin reports is final.
        let handles: Vec<JoinHandle<()>> = mem::take(&mut *workers().lock().unwrap());
        for handle in handles {
            let _ = handle.join();
        }
        // SAFETY: the handle is live, and the check above plus the insert in
        // `destroy_instance` make this run once per handle.
        let state = unsafe { &*(instance as *const Instance<P>) };
        state.plugin.stopped(&state.host);
        // SAFETY: as above; `shutdown` collects the handle it was given.
        unsafe { destroy_instance::<P>(instance) }
    }));
    result.unwrap_or(MetisStatus::Internal)
}

/// The message inside a caught panic payload, for the host's log.
fn panic_message(panic: &executor::Panic) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    "<non-string panic payload>".to_string()
}
