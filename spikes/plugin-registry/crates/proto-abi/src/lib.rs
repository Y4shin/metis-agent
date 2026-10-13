//! PROTOTYPE (throwaway). Not product code.
//!
//! The C ABI, copied down from the landed `plugin-contract` surface
//! (`crates/plugin-contract/src/abi/mod.rs`) and cut to what this prototype
//! needs: the version 1 function table, the host callback table, the buffer,
//! the status space, the table prefix, and the payload envelope.
//!
//! It exists as its own crate so the cdylib and the host share one definition
//! of the layout, exactly as `plugin-contract` is shared today.
//!
//! ROUND THREE (lane 3, versioning and expressibility). This crate is now the
//! *current* version of the ABI: the landed tables are kept byte-for-byte, and
//! the host-polled future seam is appended to them. Appending only ever adds
//! fields after the landed tail and grows `size`, which is the landed
//! append-only rule. The generated header at `include/metis_plugin_abi.h` is
//! the same surface expressed in C, and `src/layout.rs` plus
//! `c/layout_static_asserts.c` hold the layout agreement.
//!
//! The types below `// --- host-polled future seam (appended) ---` are the new
//! ones the spec adds. They are all `repr(C)`, all pointer-or-integer-or-fnptr
//! on the wire, and each table keeps `size` first.

use core::ffi::c_void;
use core::mem::{offset_of, size_of};

mod layout;

/// ABI major version. Part of the exported symbol name.
pub const METIS_ABI_MAJOR: u32 = 1;
/// ABI minor version. Travels in the function table.
pub const METIS_ABI_MINOR: u32 = 0;

/// The exported entry point for ABI major 1, NUL-terminated for `libloading`.
pub const METIS_PLUGIN_ENTRY_V1: &[u8] = b"metis_plugin_entry_v1\0";

// --- status ----------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MetisStatus {
    Ok = 0,
    InvalidArgument = 1,
    NotFound = 2,
    PermissionDenied = 3,
    Cancelled = 4,
    Internal = 5,
    Unsupported = 6,
    Pending = 7,
}

impl MetisStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            MetisStatus::Ok => "ok",
            MetisStatus::InvalidArgument => "invalid_argument",
            MetisStatus::NotFound => "not_found",
            MetisStatus::PermissionDenied => "permission_denied",
            MetisStatus::Cancelled => "cancelled",
            MetisStatus::Internal => "internal",
            MetisStatus::Unsupported => "unsupported",
            MetisStatus::Pending => "pending",
        }
    }
}

// --- buffers ---------------------------------------------------------------

pub const METIS_BUFFER_OWNER_NONE: u32 = 0;
pub const METIS_BUFFER_OWNER_HOST: u32 = 1;
pub const METIS_BUFFER_OWNER_PLUGIN: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MetisBuffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
    pub owner: u32,
    pub reserved: u32,
}

impl MetisBuffer {
    pub const fn empty() -> Self {
        MetisBuffer {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
            owner: METIS_BUFFER_OWNER_NONE,
            reserved: 0,
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.ptr.is_null()
    }
}

impl core::fmt::Debug for MetisBuffer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MetisBuffer")
            .field("ptr", &self.ptr)
            .field("len", &self.len)
            .field("cap", &self.cap)
            .field("owner", &self.owner)
            .finish()
    }
}

// --- payload envelope ------------------------------------------------------

/// The envelope is one header byte: low nibble is the payload version, bit 4 is
/// the codec bit, bits 5 to 7 are reserved and must be zero.
pub const METIS_PAYLOAD_VERSION: u8 = 1;
pub const METIS_PAYLOAD_CODEC_JSON: u8 = 0;
pub const METIS_PAYLOAD_CODEC_BIT: u8 = 0x10;
pub const METIS_PAYLOAD_RESERVED_MASK: u8 = 0xe0;

/// Wraps `body` in the payload envelope.
pub fn metis_encode_payload(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 1);
    out.push(METIS_PAYLOAD_VERSION | (METIS_PAYLOAD_CODEC_JSON << 4));
    out.extend_from_slice(body);
    out
}

/// Strips and checks the payload envelope.
pub fn metis_payload_body(bytes: &[u8]) -> Result<&[u8], MetisStatus> {
    let (header, body) = bytes.split_first().ok_or(MetisStatus::InvalidArgument)?;
    if header & METIS_PAYLOAD_RESERVED_MASK != 0 {
        return Err(MetisStatus::InvalidArgument);
    }
    if header & METIS_PAYLOAD_CODEC_BIT != 0 {
        return Err(MetisStatus::Unsupported);
    }
    if header & 0x0f != METIS_PAYLOAD_VERSION {
        return Err(MetisStatus::InvalidArgument);
    }
    Ok(body)
}

// --- tables ----------------------------------------------------------------

#[repr(C)]
pub struct MetisTablePrefix {
    pub size: usize,
    pub abi_major: u32,
    pub abi_minor: u32,
}

pub type MetisCreateInstanceFn = unsafe extern "C" fn(
    host: *const MetisHostApiV1,
    out_instance: *mut *mut c_void,
) -> MetisStatus;
pub type MetisDestroyInstanceFn = unsafe extern "C" fn(instance: *mut c_void) -> MetisStatus;
pub type MetisListCapabilitiesFn =
    unsafe extern "C" fn(instance: *mut c_void, out_payload: *mut MetisBuffer) -> MetisStatus;
pub type MetisInvokeFn = unsafe extern "C" fn(
    instance: *mut c_void,
    capability: *const u8,
    capability_len: usize,
    request_id: u64,
    payload: MetisBuffer,
    host: *const MetisHostApiV1,
    out_payload: *mut MetisBuffer,
) -> MetisStatus;
pub type MetisCancelFn =
    unsafe extern "C" fn(instance: *mut c_void, request_id: u64) -> MetisStatus;
pub type MetisReleaseBufferFn = unsafe extern "C" fn(buffer: MetisBuffer) -> MetisStatus;
pub type MetisShutdownFn = unsafe extern "C" fn(instance: *mut c_void) -> MetisStatus;

/// Appended (lane 3): delivers a host-owned service the plugin awaited.
///
/// The host calls it after answering a `service_request`, carrying the answer's
/// status and payload. The plugin wakes the future's waker from this entry, so
/// the host never blocks on the service.
pub type MetisServiceCompleteFn = unsafe extern "C" fn(
    instance: *mut c_void,
    request_id: u64,
    status: MetisStatus,
    payload: MetisBuffer,
) -> MetisStatus;

/// The version 1 function table: `size` first, then seven function pointers,
/// then the two appended host-polled future entries (lane 3).
///
/// The first seven entries and the prefix are byte-for-byte the landed table;
/// the two appended entries land exactly after `shutdown`, so a reader that
/// stops at the prefix it knows reads a valid table and only needs its `size`
/// check to decide whether the appended slots may be touched.
#[repr(C)]
pub struct MetisPluginTableV1 {
    pub size: usize,
    pub abi_major: u32,
    pub abi_minor: u32,
    pub create_instance: MetisCreateInstanceFn,
    pub destroy_instance: MetisDestroyInstanceFn,
    pub list_capabilities: MetisListCapabilitiesFn,
    pub invoke: MetisInvokeFn,
    pub cancel: MetisCancelFn,
    pub release_buffer: MetisReleaseBufferFn,
    pub shutdown: MetisShutdownFn,
    /// Appended (lane 3): starts a host-polled future instead of a callback.
    /// Present only when `size` covers it.
    pub invoke_future: MetisInvokeFutureFn,
    /// Appended (lane 3): delivers a host-owned service the plugin awaited.
    /// Present only when `size` covers it.
    pub service_complete: MetisServiceCompleteFn,
}

/// Reads the prefix fields after checking the reported `size` covers them.
///
/// # Safety
///
/// `table` must point at a live, immutable `MetisPluginTableV1` for `'a`.
pub unsafe fn metis_read_prefix<'a>(
    table: *const MetisPluginTableV1,
) -> Option<&'a MetisTablePrefix> {
    if table.is_null() {
        return None;
    }
    let prefix = unsafe { &*(table as *const MetisTablePrefix) };
    let needed = core::mem::offset_of!(MetisTablePrefix, abi_minor) + size_of::<u32>();
    if prefix.size < needed {
        return None;
    }
    Some(prefix)
}

// --- host table ------------------------------------------------------------

pub type MetisCompleteFn =
    unsafe extern "C" fn(user_data: *mut c_void, request_id: u64, status: MetisStatus, payload: MetisBuffer);
pub type MetisProgressFn =
    unsafe extern "C" fn(user_data: *mut c_void, request_id: u64, payload: MetisBuffer);
pub type MetisEmitEventFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    topic: *const u8,
    topic_len: usize,
    payload: MetisBuffer,
);
pub type MetisLogFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    level: i32,
    message: *const u8,
    message_len: usize,
);

/// Appended (lane 3): the plugin asks the host for a timer or an IO operation.
/// The host answers later through the plugin's `service_complete`, and the
/// plugin wakes the future's waker when that answer arrives.
///
/// Present only when the host table's `size` covers it; a host built before the
/// seam installs the old four callbacks and the plugin must not call this.
pub type MetisServiceRequestFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    service: *const u8,
    service_len: usize,
    request_id: u64,
    payload: MetisBuffer,
) -> MetisStatus;

pub const METIS_LOG_DEBUG: i32 = 0;
pub const METIS_LOG_INFO: i32 = 1;
pub const METIS_LOG_WARN: i32 = 2;
pub const METIS_LOG_ERROR: i32 = 3;

#[repr(C)]
pub struct MetisHostApiV1 {
    pub size: usize,
    pub user_data: *mut c_void,
    pub complete: MetisCompleteFn,
    pub progress: MetisProgressFn,
    pub emit_event: MetisEmitEventFn,
    pub log: MetisLogFn,
    /// Appended (lane 3): host-owned services the plugin awaits. Present only
    /// when `size` covers it.
    pub service_request: MetisServiceRequestFn,
}

// --- host-polled future seam (appended, lane 3) ----------------------------
//
// The boundary types of the host-polled future. `async-ffi`'s `FfiFuture` never
// crosses: the boundary is an opaque `ctx` the plugin owns plus two C vtables
// that carry their own `size`. Every field is a pointer, an integer, or a
// function pointer.

/// A plugin-owned future context plus the vtable the host polls it through.
///
/// The plugin allocates `ctx` and owns it; the host never dereferences it, it
/// only passes it back to `poll` and calls `drop` on it exactly once.
#[repr(C)]
pub struct MetisFutureHandle {
    /// Opaque to the host: plugin-owned state for one in-flight future.
    pub ctx: *mut c_void,
    /// The plugin's future vtable.
    pub vtable: *const MetisFutureVtable,
}

/// Polls a future's `ctx`. Returns `Ok` with a plugin-allocated buffer in
/// `out_value` when ready, `Pending` when not ready (the plugin will call
/// `waker.wake` when it is), and any other status when the call failed.
///
/// `waker` is valid until the future is dropped. The plugin may clone it (each
/// clone is a `clone` call) and must `drop` every clone. `poll` must not block.
pub type MetisFuturePollFn = unsafe extern "C" fn(
    ctx: *mut c_void,
    waker: MetisWaker,
    out_value: *mut MetisBuffer,
) -> MetisStatus;

/// Releases a future's `ctx`. Called exactly once by the host, and it must
/// release every plugin-owned buffer the future still holds.
pub type MetisFutureDropFn = unsafe extern "C" fn(ctx: *mut c_void) -> MetisStatus;

impl MetisFutureHandle {
    /// The unset handle a null out parameter holds.
    pub const fn null() -> Self {
        MetisFutureHandle {
            ctx: core::ptr::null_mut(),
            vtable: core::ptr::null(),
        }
    }
}

/// The plugin's future vtable. `size` is first, like every table.
#[repr(C)]
pub struct MetisFutureVtable {
    /// Size in bytes of this table. First field, always.
    pub size: usize,
    /// The single non-blocking poll entry.
    pub poll: MetisFuturePollFn,
    /// The exactly-once release entry.
    pub drop: MetisFutureDropFn,
}

/// A waker the host hands to `poll`. The plugin may call `wake` from any
/// thread, and every clone it makes must be `drop`ped.
///
/// `wake` and `drop` consume the waker they are given; `clone` and `wake_by_ref`
/// borrow it.
#[repr(C)]
pub struct MetisWaker {
    /// Opaque to the plugin: host-owned state behind the waker.
    pub data: *mut c_void,
    /// The host's waker vtable.
    pub vtable: *const MetisWakerVtable,
}

/// Produces a second owning waker from a borrowed one.
pub type MetisWakerCloneFn = unsafe extern "C" fn(waker: *const MetisWaker) -> MetisWaker;
/// Consumes a waker and schedules the future's task.
pub type MetisWakerWakeFn = unsafe extern "C" fn(waker: MetisWaker);
/// Schedules the future's task without consuming the waker.
pub type MetisWakerWakeByRefFn = unsafe extern "C" fn(waker: *const MetisWaker);
/// Consumes a waker without waking it.
pub type MetisWakerDropFn = unsafe extern "C" fn(waker: MetisWaker);

impl MetisWaker {
    /// The unset waker a null handle holds.
    pub const fn null() -> Self {
        MetisWaker {
            data: core::ptr::null_mut(),
            vtable: core::ptr::null(),
        }
    }
}

/// The host's waker vtable. `size` is first, like every table.
#[repr(C)]
pub struct MetisWakerVtable {
    /// Size in bytes of this table. First field, always.
    pub size: usize,
    /// Produces a second owning waker from a borrowed one.
    pub clone: MetisWakerCloneFn,
    /// Consumes a waker and schedules the future's task. May be called from any
    /// thread; must be cheap and non-blocking.
    pub wake: MetisWakerWakeFn,
    /// Schedules the future's task without consuming the waker.
    pub wake_by_ref: MetisWakerWakeByRefFn,
    /// Consumes a waker without waking it.
    pub drop: MetisWakerDropFn,
}

/// Appended (lane 3): starts a host-polled future for `capability`.
///
/// On success `*out` holds the plugin-allocated future handle. The host is the
/// only poller, and it must call the handle's `drop` exactly once.
pub type MetisInvokeFutureFn = unsafe extern "C" fn(
    instance: *mut c_void,
    capability: *const u8,
    capability_len: usize,
    request_id: u64,
    payload: MetisBuffer,
    host: *const MetisHostApiV1,
    out: *mut MetisFutureHandle,
) -> MetisStatus;

// --- presence by size (the append-only rule) -------------------------------

/// The byte size of the landed plugin table: prefix plus the seven landed
/// entries. A table smaller than this is the *older* plugin, and the host must
/// not touch anything past it.
pub const METIS_PLUGIN_TABLE_V1_LANDED_SIZE: usize =
    size_of::<MetisTablePrefix>() + 7 * size_of::<*const c_void>();

/// The byte size of the landed host table: `size`, `user_data`, four callbacks.
pub const METIS_HOST_API_V1_LANDED_SIZE: usize =
    size_of::<usize>() + 5 * size_of::<*const c_void>();

/// Whether a plugin table that claims `size` bytes covers `invoke_future`.
///
/// Presence is decided by `size`, never by the pointer value at the slot: an
/// older plugin's table has no such slot, and the bytes after it are whatever
/// the older build had there.
pub const fn metis_plugin_table_has_invoke_future(size: usize) -> bool {
    size >= offset_of!(MetisPluginTableV1, invoke_future) + size_of::<MetisInvokeFutureFn>()
}

/// Whether a plugin table that claims `size` bytes covers `service_complete`.
pub const fn metis_plugin_table_has_service_complete(size: usize) -> bool {
    size >= offset_of!(MetisPluginTableV1, service_complete) + size_of::<MetisServiceCompleteFn>()
}

/// Whether a host table that claims `size` bytes covers `service_request`.
pub const fn metis_host_api_has_service_request(size: usize) -> bool {
    size >= offset_of!(MetisHostApiV1, service_request) + size_of::<MetisServiceRequestFn>()
}

pub type MetisPluginEntryV1Fn = unsafe extern "C" fn() -> *const MetisPluginTableV1;
