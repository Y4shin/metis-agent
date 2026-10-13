//! The plugin side of the translation: `MetisPluginTableV1` on one side, the
//! `Plugin` trait on the other.
//!
//! Everything the ABI forces onto a plugin lives here: the payload envelope,
//! buffer ownership, panic catching at every exported entry, the cancellation
//! registry that lets `cancel` reach the call a future is parked on, the
//! service completion the host answers an awaited service through, and the
//! check that a plugin's capability declaration has no duplicate names.
//!
//! There is no executor here and no worker thread. The host polls the future
//! this adapter hands out, on the host's own runtime, so a plugin needs no
//! runtime, no pool, and no thread per call, which is also what makes unloading
//! safe: plugin code only runs inside a call the host made.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::str;
use std::sync::{Arc, Mutex, OnceLock};

use crate::abi::{
    METIS_ABI_MAJOR, METIS_ABI_MINOR, METIS_BUFFER_OWNER_HOST, MetisBuffer, MetisFutureHandle,
    MetisHostApiV1, MetisPluginTableV1, MetisStatus,
};
use crate::authoring::alloc::{plugin_alloc, release_buffer, violation};
use crate::authoring::call::{Call, CancellationToken, Host, LogLevel, resolve_service};
use crate::authoring::capability::CapabilityList;
use crate::authoring::future::{ErasedFuture, new_handle};
use crate::authoring::traits::{Plugin, PluginError, Request};
use crate::payload::{metis_encode_payload, metis_payload_body};

/// One live instance.
///
/// The plugin sits behind an `Arc` so a future can own a handle to it, and the
/// host table is retained because a future may be polled from any host thread.
struct Instance<P: Plugin> {
    plugin: Arc<P>,
    host: Host,
}

/// Request id to the call, so `cancel` reaches the same token a future is
/// parked on.
///
/// Process-wide, because the table is a `const fn` with no user data to key it
/// by; request ids are host-supplied and unique, which is what keeps two
/// instances of one plugin from colliding.
fn calls() -> &'static Mutex<HashMap<u64, Call>> {
    static CALLS: OnceLock<Mutex<HashMap<u64, Call>>> = OnceLock::new();
    CALLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Removes a call from the registry, which the future's own release does, so
/// the registry cannot outlive the future it describes.
fn release_call(request_id: u64) {
    calls().lock().unwrap().remove(&request_id);
}

/// Instance handles this plugin has already taken back, so a second release is
/// reported rather than becoming a double free.
fn released_instances() -> &'static Mutex<HashSet<usize>> {
    static RELEASED: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
    RELEASED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// The whole plugin side of the translation as one `const fn`, which the plugin
/// instantiates into a static and exports through `metis_plugin_entry_v1`.
pub const fn table<P: Plugin + Default>() -> MetisPluginTableV1 {
    MetisPluginTableV1 {
        size: mem::size_of::<MetisPluginTableV1>(),
        abi_major: METIS_ABI_MAJOR,
        abi_minor: METIS_ABI_MINOR,
        create_instance: create_instance::<P>,
        destroy_instance: destroy_instance::<P>,
        list_capabilities: list_capabilities::<P>,
        invoke_future: invoke_future::<P>,
        cancel: cancel::<P>,
        release_buffer,
        shutdown: shutdown::<P>,
        service_complete: service_complete::<P>,
    }
}

/// The status an authoring error maps onto.
pub(crate) fn metis_status(error: &PluginError) -> MetisStatus {
    match error {
        PluginError::InvalidArgument(_) => MetisStatus::InvalidArgument,
        PluginError::NotFound(_) => MetisStatus::NotFound,
        PluginError::PermissionDenied(_) => MetisStatus::PermissionDenied,
        PluginError::Unsupported(_) => MetisStatus::Unsupported,
        PluginError::Internal(_) => MetisStatus::Internal,
        PluginError::Cancelled => MetisStatus::Cancelled,
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, PluginError> {
    serde_json::to_vec(value)
        .map_err(|err| PluginError::Internal(format!("response payload: {err}")))
}

/// Copies a borrowed host buffer and validates its payload envelope.
///
/// The trait never sees the header, the owner tag, or the length.
///
/// # Safety
///
/// `buffer` must be the buffer the host passed, valid for the duration of the
/// call.
unsafe fn read_host_payload(buffer: MetisBuffer) -> Result<Vec<u8>, MetisStatus> {
    if buffer.owner != METIS_BUFFER_OWNER_HOST {
        violation("the plugin received a buffer it does not believe the host allocated");
        return Err(MetisStatus::InvalidArgument);
    }
    if buffer.ptr.is_null() {
        return Err(MetisStatus::InvalidArgument);
    }
    // SAFETY: the host owns the buffer and keeps it alive for the call.
    let bytes = unsafe { slice::from_raw_parts(buffer.ptr, buffer.len) };
    Ok(metis_payload_body(bytes)?.to_vec())
}

/// Checks that a plugin's capability declaration is usable.
///
/// There is one declaration now, not a declaration plus a set of dispatch arms,
/// so the only way it can be wrong is to name the same capability twice, which
/// would make the routed `call` ambiguous for a host that routes by name. It is
/// a creation failure rather than a call-time surprise.
fn check_declaration<P: Plugin>(plugin: &P) -> Result<(), String> {
    let mut seen = HashSet::new();
    for descriptor in plugin.capabilities() {
        if !seen.insert(descriptor.name.clone()) {
            return Err(format!("two capabilities are named {}", descriptor.name));
        }
    }
    Ok(())
}

// --- exported adapters -----------------------------------------------------
//
// Every exported function catches panics, so a panic in a plugin becomes a
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
        if let Err(mismatch) = check_declaration::<P>(&plugin) {
            host.log(LogLevel::Error, &format!("authoring: {mismatch}"));
            return MetisStatus::InvalidArgument;
        }
        let instance = Box::new(Instance {
            plugin: Arc::new(plugin),
            host,
        });
        // `Default` construction cannot fail or take configuration, so this is
        // where initialisation reports failure. A plugin whose `started` fails
        // yields no instance.
        if let Err(error) = instance.plugin.started(&instance.host) {
            instance.host.log(
                LogLevel::Error,
                &format!("authoring: instance start failed: {error}"),
            );
            drop(instance);
            return metis_status(&error);
        }
        instance
            .host
            .log(LogLevel::Info, "authoring: instance created");
        let raw = Box::into_raw(instance);
        // A released handle's address can be handed to the next instance by the
        // allocator; creating an instance there makes it live again.
        released_instances().lock().unwrap().remove(&(raw as usize));
        // SAFETY: the host owns the out parameter and gets the only handle to
        // this box, which `destroy_instance` takes back exactly once.
        unsafe { *out_instance = raw as *mut c_void };
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
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
        state
            .host
            .log(LogLevel::Info, "authoring: instance released");
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
        unsafe { *out_payload = plugin_alloc(&metis_encode_payload(&body)) };
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn invoke_future<P: Plugin>(
    instance: *mut c_void,
    capability: *const u8,
    capability_len: usize,
    request_id: u64,
    payload: MetisBuffer,
    host: *const MetisHostApiV1,
    out: *mut MetisFutureHandle,
) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() || capability.is_null() || host.is_null() || out.is_null() {
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
        let call = Call::new(request_id, host, CancellationToken::new());

        // The registry entry exists before the future can be polled, so a
        // cancel that races the first poll still finds the call. The registry
        // and the future share one cancellation token, so marking either marks
        // the one the author parked on.
        calls().lock().unwrap().insert(request_id, call.clone());

        // The future owns everything it needs, so it is `'static` and the host
        // may poll it from any thread. The request borrows the owned name and
        // body inside the async block, which is why the block exists at all.
        let plugin = Arc::clone(&state.plugin);
        let capability_name = capability.to_string();
        let future: ErasedFuture = Box::pin(async move {
            let request = Request::new(&capability_name, &body);
            plugin.call(call, request).await
        });

        // SAFETY: the host owns the out parameter, and the handle it receives
        // is dropped exactly once through the vtable it carries.
        unsafe { *out = new_handle(future, request_id, release_call) };
        MetisStatus::Ok
    }));
    match result {
        Ok(status) => status,
        Err(_) => {
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
        let call = match calls().lock().unwrap().get(&request_id) {
            Some(call) => call.clone(),
            None => return MetisStatus::NotFound,
        };
        // The token fires before the plugin's own cancel method runs, so a
        // future parked on it is woken whether or not the method says anything.
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
        // SAFETY: the handle is live, and the host drops every future before
        // calling this, so nothing is in flight here.
        let state = unsafe { &*(instance as *const Instance<P>) };
        state.plugin.stopped(&state.host);
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn service_complete<P: Plugin>(
    instance: *mut c_void,
    request_id: u64,
    status: MetisStatus,
    payload: MetisBuffer,
) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() {
            return MetisStatus::InvalidArgument;
        }
        // The payload is host-owned and borrowed for this call: the plugin
        // copies what it needs and never frees it. An error status may carry no
        // payload at all.
        let body = if status == MetisStatus::Ok {
            match unsafe { read_host_payload(payload) } {
                Ok(body) => body,
                Err(status) => return status,
            }
        } else {
            Vec::new()
        };
        if resolve_service(request_id, status, body) {
            MetisStatus::Ok
        } else {
            // A completion for a call the plugin no longer holds. That is a
            // recorded event rather than a crash: the host may have given up.
            // SAFETY: the handle is the live instance.
            let state = unsafe { &*(instance as *const Instance<P>) };
            state.host.log(
                LogLevel::Warn,
                &format!("authoring: no awaited service for request {request_id}"),
            );
            MetisStatus::NotFound
        }
    }));
    result.unwrap_or(MetisStatus::Internal)
}
