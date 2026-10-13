//! The C ABI surface (D1, D6, D7, D8).
//!
//! Everything a non-Rust implementer needs and nothing else: one versioned
//! entry point, a function table, a host callback table, an opaque buffer, a
//! status space, and the prefix that makes the table append-only. This is the
//! only module `cbindgen` translates into the committed header.
//!
//! The layout rules the tables rely on are asserted at the bottom of this
//! module by `layout`, on every build, and again in C by
//! `c/layout_static_asserts.c`.

use core::ffi::c_void;
use core::mem::size_of;

mod layout;

/// ABI major version. It is part of the exported symbol name (D8).
pub const METIS_ABI_MAJOR: u32 = 1;
/// ABI minor version. It travels in the function table (D8).
pub const METIS_ABI_MINOR: u32 = 0;

/// The name of the exported entry point for ABI major 1 (D8).
///
/// NUL-terminated, so it can be passed straight to `dlsym` and to `libloading`.
pub const METIS_PLUGIN_ENTRY_V1: &[u8] = b"metis_plugin_entry_v1\0";

// --- status (D7) -----------------------------------------------------------

/// The status space, frozen within ABI major 1 (D7).
///
/// The members are an enum because every entry of the function table returns
/// one, and because a frozen integer space is what lets a host written in
/// another language handle a plugin it was not compiled against.
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
    /// The result did not finish inline and will arrive through the host's
    /// `complete` callback (D11). This member joins the frozen list because no
    /// other member can express a deferred outcome.
    Pending = 7,
}

impl MetisStatus {
    /// The wire name of a status, for log lines and harness output.
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

// --- buffers (D6) ----------------------------------------------------------

/// No owner: an unset buffer, such as a null out parameter.
pub const METIS_BUFFER_OWNER_NONE: u32 = 0;
/// Allocated by the host, freed by the host after the call returns.
pub const METIS_BUFFER_OWNER_HOST: u32 = 1;
/// Allocated by the plugin, released through the table's `release_buffer`.
pub const METIS_BUFFER_OWNER_PLUGIN: u32 = 2;

/// A byte buffer crossing the boundary (D6).
///
/// The owner tag is what makes "each side releases only what it allocated"
/// checkable at runtime: a receiver never frees a buffer it does not own, it
/// hands a plugin-owned buffer back through `release_buffer`, and a host-owned
/// buffer is only valid for the duration of the call that received it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MetisBuffer {
    /// The first byte of the payload, or null for an empty buffer.
    pub ptr: *mut u8,
    /// Valid bytes at `ptr`.
    pub len: usize,
    /// Allocated capacity at `ptr`, which only the allocator may free with.
    pub cap: usize,
    /// One of the `METIS_BUFFER_OWNER_*` constants.
    pub owner: u32,
    /// Reserved, zero.
    pub reserved: u32,
}

impl MetisBuffer {
    /// The empty buffer, which is what a null out parameter is set to.
    pub const fn empty() -> Self {
        MetisBuffer {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
            owner: METIS_BUFFER_OWNER_NONE,
            reserved: 0,
        }
    }

    /// Whether this buffer holds no bytes.
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
            .field("reserved", &self.reserved)
            .finish()
    }
}

// --- host log levels -------------------------------------------------------

/// Diagnostic level for the host `log` callback.
pub const METIS_LOG_DEBUG: i32 = 0;
/// Informational level for the host `log` callback.
pub const METIS_LOG_INFO: i32 = 1;
/// Warning level for the host `log` callback.
pub const METIS_LOG_WARN: i32 = 2;
/// Error level for the host `log` callback.
pub const METIS_LOG_ERROR: i32 = 3;

// --- table prefix (D8) -----------------------------------------------------

/// The smallest prefix of any table: the size and the version fields.
///
/// A consumer that knows only this much reads only this much, which is what
/// makes the table append-only. `size` is the first field so it is readable
/// before anything else is touched, and a table whose `size` does not cover
/// this prefix is rejected rather than read.
#[repr(C)]
pub struct MetisTablePrefix {
    /// Size in bytes of the table the plugin returned.
    pub size: usize,
    /// ABI major version of the table.
    pub abi_major: u32,
    /// ABI minor version of the table.
    pub abi_minor: u32,
}

// --- plugin table (D1, D8) -------------------------------------------------

/// Creates an instance and hands it the host table it will call back through.
pub type MetisCreateInstanceFn = unsafe extern "C" fn(
    host: *const MetisHostApiV1,
    out_instance: *mut *mut c_void,
) -> MetisStatus;

/// Destroys an instance. A destroyed handle is not used again.
pub type MetisDestroyInstanceFn = unsafe extern "C" fn(instance: *mut c_void) -> MetisStatus;

/// Returns a plugin-allocated payload holding the capability descriptors (D9).
pub type MetisListCapabilitiesFn =
    unsafe extern "C" fn(instance: *mut c_void, out_payload: *mut MetisBuffer) -> MetisStatus;

/// One generic call, addressed by capability name (D1, D9, D11).
///
/// Returns `Ok` with a plugin-allocated payload for an inline result, `Pending`
/// when the result will arrive through the host's `complete` callback, or a
/// failure status. `host` is passed per call as well as retained by the
/// instance, so an implementation can use either without a cost.
pub type MetisInvokeFn = unsafe extern "C" fn(
    instance: *mut c_void,
    capability: *const u8,
    capability_len: usize,
    request_id: u64,
    payload: MetisBuffer,
    host: *const MetisHostApiV1,
    out_payload: *mut MetisBuffer,
) -> MetisStatus;

/// Requests cancellation of a pending request (D11, D12).
pub type MetisCancelFn =
    unsafe extern "C" fn(instance: *mut c_void, request_id: u64) -> MetisStatus;

/// Releases a buffer this plugin allocated (D6).
///
/// The host calls it for every plugin-owned buffer it received, including a
/// payload it decided to drop.
pub type MetisReleaseBufferFn = unsafe extern "C" fn(buffer: MetisBuffer) -> MetisStatus;

/// Shuts an instance down: no callback may arrive after it returns (D12).
pub type MetisShutdownFn = unsafe extern "C" fn(instance: *mut c_void) -> MetisStatus;

/// The version 1 function table (D1, D8).
///
/// Append-only evolution: a new entry is appended after `shutdown` and `size`
/// grows to cover it. A reader compares `size` against the prefix it knows
/// before touching a field, and never decides presence from a null pointer:
/// the fields are plain function pointers, because `Option<NamedFnAlias>` has
/// no C rendering cbindgen can emit (measured).
#[repr(C)]
pub struct MetisPluginTableV1 {
    /// Size in bytes of this table. First field, always.
    pub size: usize,
    /// ABI major version, which must match the entry symbol's naming.
    pub abi_major: u32,
    /// ABI minor version, which may exceed the one a host was built with.
    pub abi_minor: u32,
    /// Instance lifecycle.
    pub create_instance: MetisCreateInstanceFn,
    /// Instance lifecycle.
    pub destroy_instance: MetisDestroyInstanceFn,
    /// Capability discovery (D9).
    pub list_capabilities: MetisListCapabilitiesFn,
    /// The single generic call (D1).
    pub invoke: MetisInvokeFn,
    /// Cancellation (D11).
    pub cancel: MetisCancelFn,
    /// Buffer release (D6).
    pub release_buffer: MetisReleaseBufferFn,
    /// Shutdown (D12).
    pub shutdown: MetisShutdownFn,
}

/// Reads the prefix fields of a table after checking that the reported `size`
/// covers them.
///
/// Returns `None` for a null table and for a table that claims to be smaller
/// than the prefix, so a caller never reads a field past the size the plugin
/// promised. A table larger than this build's layout is fine: the prefix is by
/// construction the part every reader agrees on.
///
/// # Safety
///
/// `table` must point at a `MetisPluginTableV1` (or at anything whose first
/// three fields have this layout) that stays alive, and stays immutable, for
/// the lifetime `'a` the caller picks.
pub unsafe fn metis_read_prefix<'a>(
    table: *const MetisPluginTableV1,
) -> Option<&'a MetisTablePrefix> {
    if table.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees a live, immutable table. The `size` field
    // is the first field of both types, so reading it is in bounds whenever the
    // pointer itself is valid; only the fields after it need the size check.
    let prefix = unsafe { &*(table as *const MetisTablePrefix) };
    let needed = core::mem::offset_of!(MetisTablePrefix, abi_minor) + size_of::<u32>();
    if prefix.size < needed {
        return None;
    }
    Some(prefix)
}

// --- host table (D1 plus the user_data amendment) --------------------------

/// A completion, inline or late, carrying a plugin-allocated payload (D11).
///
/// The receiver releases the payload through `release_buffer` even when it
/// discards it, for example a completion that arrived after a successful
/// cancel.
pub type MetisCompleteFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    request_id: u64,
    status: MetisStatus,
    payload: MetisBuffer,
);

/// Progress for a pending request, carrying a plugin-allocated payload.
pub type MetisProgressFn =
    unsafe extern "C" fn(user_data: *mut c_void, request_id: u64, payload: MetisBuffer);

/// A plugin-originated event, owned by the plugin until it is released.
pub type MetisEmitEventFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    topic: *const u8,
    topic_len: usize,
    payload: MetisBuffer,
);

/// A log line borrowed from the plugin for the duration of the call.
pub type MetisLogFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    level: i32,
    message: *const u8,
    message_len: usize,
);

/// The version 1 host callback table (D1).
///
/// Same rules as the plugin table: append-only, presence decided by `size`, and
/// entries are plain function pointers. `user_data` is what makes the callbacks
/// usable: without it a host has nowhere to keep the state the callback needs.
/// A host that does not support a callback installs a no-op stub rather than a
/// null pointer.
#[repr(C)]
pub struct MetisHostApiV1 {
    /// Size in bytes of this table. First field, always.
    pub size: usize,
    /// Opaque host state handed back to every callback.
    pub user_data: *mut c_void,
    /// Completion of a pending request (D11).
    pub complete: MetisCompleteFn,
    /// Progress of a pending request.
    pub progress: MetisProgressFn,
    /// A plugin-originated event.
    pub emit_event: MetisEmitEventFn,
    /// A log line.
    pub log: MetisLogFn,
}

// --- entry point (D8) ------------------------------------------------------

/// The signature of `metis_plugin_entry_v1`.
///
/// Argument-free and frozen within a major version: the symbol name carries
/// only the major, so a signature that changed inside major 1 could not be
/// detected before the call. No entry point takes `MetisPluginTableV1` in a
/// parameter either, because cbindgen then emits the typedef before the struct
/// and C scopes the struct tag to the prototype (measured).
pub type MetisPluginEntryV1Fn = unsafe extern "C" fn() -> *const MetisPluginTableV1;
