//! The C ABI surface, revision 1 of the approved plugin contract.
//!
//! Everything a non-Rust implementer needs and nothing else: one versioned
//! entry point, a function table whose only call entry starts a host-polled
//! future, a host callback table, an opaque buffer, a status space, the future
//! and waker vtables, and the prefix that makes the tables append-only. This is
//! the only module `cbindgen` translates into the committed header.
//!
//! The revision is clean, not an append: `invoke` and the host `complete`
//! callback are deleted rather than deprecated, and the entry order changed
//! while the entry symbol and the ABI major stayed the same. `abi_minor`
//! therefore becomes 1 and [`metis_read_table`] compares it additively: it
//! accepts minor 1 and anything newer, whose extra entries are appended after
//! the last entry it reads, and it refuses a minor older than 1, which refuses a
//! stale pre-release plugin instead of reading its entries through the new
//! order.
//!
//! The layout rules the tables rely on are asserted at the bottom of this
//! module by `layout`, on every build, and again in C by
//! `c/layout_static_asserts.c`.

use core::ffi::c_void;
use core::mem::size_of;

mod layout;
mod version;

pub use version::{MetisAbiError, metis_read_table};

/// ABI major version. It is part of the exported symbol name.
pub const METIS_ABI_MAJOR: u32 = 1;
/// ABI minor version. It travels in the function table.
///
/// It is 1 because this revision changed the entry order under an unchanged
/// symbol and major. The minor is compared additively: a reader accepts the
/// minor it knows and any newer one, because a newer table's extra entries are
/// appended after the last entry this build reads, and it refuses an older
/// minor, because that table's entries are in the superseded order. A plugin
/// built against the superseded order is therefore refused instead of misread.
pub const METIS_ABI_MINOR: u32 = 1;

/// The name of the exported entry point for ABI major 1.
///
/// NUL-terminated, so it can be passed straight to `dlsym` and to `libloading`.
pub const METIS_PLUGIN_ENTRY_V1: &[u8] = b"metis_plugin_entry_v1\0";

// --- status ---------------------------------------------------------------

/// The status space, frozen within ABI major 1.
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
    /// Not finished yet. A poll returns it, the plugin wakes the host waker
    /// when the answer arrives, and the host polls again.
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

// --- buffers --------------------------------------------------------------

/// No owner: an unset buffer, such as a null out parameter.
pub const METIS_BUFFER_OWNER_NONE: u32 = 0;
/// Allocated by the host, freed by the host after the call returns.
pub const METIS_BUFFER_OWNER_HOST: u32 = 1;
/// Allocated by the plugin, released through the table's `release_buffer`.
pub const METIS_BUFFER_OWNER_PLUGIN: u32 = 2;

/// A byte buffer crossing the boundary.
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

// --- host log levels ------------------------------------------------------

/// Diagnostic level for the host `log` callback.
pub const METIS_LOG_DEBUG: i32 = 0;
/// Informational level for the host `log` callback.
pub const METIS_LOG_INFO: i32 = 1;
/// Warning level for the host `log` callback.
pub const METIS_LOG_WARN: i32 = 2;
/// Error level for the host `log` callback.
pub const METIS_LOG_ERROR: i32 = 3;

// --- table prefix ---------------------------------------------------------

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

// --- the future and the waker --------------------------------------------

/// Polls a future's context.
///
/// Returns `Ok` with a plugin-allocated buffer in `out_value` when the call is
/// done, `Pending` when it is not (the plugin wakes `waker` when it is), and
/// any other status when the call failed and the future is terminal. A poll
/// must not block.
pub type MetisFuturePollFn = unsafe extern "C" fn(
    ctx: *mut c_void,
    waker: MetisWaker,
    out_value: *mut MetisBuffer,
) -> MetisStatus;

/// Releases a future's context. Called by the host exactly once, and never
/// while a poll is in progress.
pub type MetisFutureDropFn = unsafe extern "C" fn(ctx: *mut c_void) -> MetisStatus;

/// The plugin's future vtable: how the host polls a plugin-owned context.
///
/// There is deliberately no `size` field. A vtable is reached through the
/// handle that carries it, one poll and one drop are the whole surface, and a
/// plugin that publishes a vtable of the wrong shape is that plugin's own bug:
/// it is not detectable at load, and a size field nobody reads would not make
/// it detectable. The residual is recorded here rather than paid for in every
/// poll.
#[repr(C)]
pub struct MetisFutureVtable {
    /// The single non-blocking poll entry.
    pub poll: MetisFuturePollFn,
    /// The exactly-once release entry.
    pub drop: MetisFutureDropFn,
}

/// A plugin-owned future: an opaque context plus the vtable it is polled
/// through.
///
/// The host never looks inside `ctx`; it only passes it back to `poll` and to
/// `drop`. The value is copyable because that is what a two-field C struct is,
/// and copying it does not create a second future: the host still drops the one
/// handle it was given, exactly once.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MetisFutureHandle {
    /// Plugin-owned state for one in-flight future.
    pub ctx: *mut c_void,
    /// The plugin's future vtable.
    pub vtable: *const MetisFutureVtable,
}

impl MetisFutureHandle {
    /// The unset handle a null out parameter holds.
    pub const fn null() -> Self {
        MetisFutureHandle {
            ctx: core::ptr::null_mut(),
            vtable: core::ptr::null(),
        }
    }

    /// Whether this handle names a future at all.
    pub const fn is_null(&self) -> bool {
        self.vtable.is_null()
    }
}

/// Produces a second owning waker from a borrowed one.
pub type MetisWakerCloneFn = unsafe extern "C" fn(waker: *const MetisWaker) -> MetisWaker;
/// Consumes a waker and schedules the future's task.
pub type MetisWakerWakeFn = unsafe extern "C" fn(waker: MetisWaker);
/// Schedules the future's task without consuming the waker.
pub type MetisWakerWakeByRefFn = unsafe extern "C" fn(waker: *const MetisWaker);
/// Consumes a waker without waking it.
pub type MetisWakerDropFn = unsafe extern "C" fn(waker: MetisWaker);

/// The host's waker: how a plugin asks to be polled again.
///
/// The waker the host passes to `poll` is borrowed and valid until the future
/// is dropped. The plugin may clone it, each clone is one `clone` call, and it
/// must `drop` every clone. `wake` may be called from any thread, must be cheap
/// and non-blocking, and may arrive after the future was dropped, which the
/// host records rather than crashing on.
///
/// There is deliberately no `Copy`: duplicating this value in Rust would make a
/// second owning handle without a `clone` call, and every clone has to be
/// counted and released through the vtable.
#[repr(C)]
pub struct MetisWaker {
    /// Host-owned state behind the waker, opaque to the plugin.
    pub data: *mut c_void,
    /// The host's waker vtable.
    pub vtable: *const MetisWakerVtable,
}

impl MetisWaker {
    /// The unset waker a null handle holds.
    pub const fn null() -> Self {
        MetisWaker {
            data: core::ptr::null_mut(),
            vtable: core::ptr::null(),
        }
    }

    /// Whether this waker names a host waker at all.
    pub const fn is_null(&self) -> bool {
        self.vtable.is_null()
    }
}

/// The host's waker vtable.
///
/// There is deliberately no `size` field, for the same reason as
/// [`MetisFutureVtable`]: the handle is the only route to it, the four entries
/// below are the whole surface, and a wrong-shape vtable is its author's bug.
#[repr(C)]
pub struct MetisWakerVtable {
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

// --- plugin table ---------------------------------------------------------

/// Creates an instance and hands it the host table it will call back through.
pub type MetisCreateInstanceFn = unsafe extern "C" fn(
    host: *const MetisHostApiV1,
    out_instance: *mut *mut c_void,
) -> MetisStatus;

/// Destroys an instance. A destroyed handle is not used again.
pub type MetisDestroyInstanceFn = unsafe extern "C" fn(instance: *mut c_void) -> MetisStatus;

/// Returns a plugin-allocated payload holding the capability descriptors.
pub type MetisListCapabilitiesFn =
    unsafe extern "C" fn(instance: *mut c_void, out_payload: *mut MetisBuffer) -> MetisStatus;

/// The only call entry: starts a host-polled future for a capability.
///
/// On success `*out` holds a plugin-owned future. Nothing is called back for
/// the call's result: the host polls the future it was handed. The request
/// payload is host-owned and valid for the call only.
pub type MetisInvokeFutureFn = unsafe extern "C" fn(
    instance: *mut c_void,
    capability: *const u8,
    capability_len: usize,
    request_id: u64,
    payload: MetisBuffer,
    host: *const MetisHostApiV1,
    out: *mut MetisFutureHandle,
) -> MetisStatus;

/// Marks a call's cancellation token.
///
/// The plugin's future for that request is expected to resolve as `Cancelled`
/// rather than be dropped mid-flight.
pub type MetisCancelFn =
    unsafe extern "C" fn(instance: *mut c_void, request_id: u64) -> MetisStatus;

/// Releases a buffer this plugin allocated.
///
/// The host calls it for every plugin-owned buffer it received, including a
/// payload it decided to drop and one it is handed from inside
/// `service_request`, where this entry is reentrant.
pub type MetisReleaseBufferFn = unsafe extern "C" fn(buffer: MetisBuffer) -> MetisStatus;

/// Quiesces an instance.
///
/// The host drops every future before calling it, so nothing may arrive after
/// it returns and the instance may then be destroyed.
pub type MetisShutdownFn = unsafe extern "C" fn(instance: *mut c_void) -> MetisStatus;

/// Delivers the host's answer to an earlier `service_request`.
///
/// The plugin wakes its future's waker from here. The payload is host-owned and
/// borrowed for the call.
pub type MetisServiceCompleteFn = unsafe extern "C" fn(
    instance: *mut c_void,
    request_id: u64,
    status: MetisStatus,
    payload: MetisBuffer,
) -> MetisStatus;

/// The version 1 function table, revision 1 of the contract.
///
/// Append-only evolution from here: a new entry is appended after
/// `service_complete` and `size` grows to cover it. A reader compares `size`
/// against the prefix it knows before touching a field, and never decides
/// presence from a null pointer: the fields are plain function pointers,
/// because `Option<NamedFnAlias>` has no C rendering cbindgen can emit
/// (measured).
#[repr(C)]
pub struct MetisPluginTableV1 {
    /// Size in bytes of this table. First field, always.
    pub size: usize,
    /// ABI major version, which must match the entry symbol's naming.
    pub abi_major: u32,
    /// ABI minor version. A reader accepts the one it speaks and any newer one,
    /// and refuses an older one.
    pub abi_minor: u32,
    /// Instance lifecycle.
    pub create_instance: MetisCreateInstanceFn,
    /// Instance lifecycle.
    pub destroy_instance: MetisDestroyInstanceFn,
    /// Capability discovery.
    pub list_capabilities: MetisListCapabilitiesFn,
    /// The only call entry.
    pub invoke_future: MetisInvokeFutureFn,
    /// Cancellation.
    pub cancel: MetisCancelFn,
    /// Buffer release.
    pub release_buffer: MetisReleaseBufferFn,
    /// Quiescence.
    pub shutdown: MetisShutdownFn,
    /// Service completion.
    pub service_complete: MetisServiceCompleteFn,
}

// --- host table -----------------------------------------------------------

/// Progress for an in-flight call, carrying a plugin-allocated payload.
///
/// The receiver releases the payload through `release_buffer`.
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

/// The plugin asks the host for a service it can await: today a timer, later
/// IO.
///
/// The host answers later through the plugin's `service_complete`, with the
/// same `request_id`, and the plugin wakes its future's waker when the answer
/// arrives. The payload is plugin-owned and the host releases it, including
/// from inside this callback, where `release_buffer` is reentrant.
pub type MetisServiceRequestFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    service: *const u8,
    service_len: usize,
    request_id: u64,
    payload: MetisBuffer,
) -> MetisStatus;

/// The version 1 host callback table, revision 1 of the contract.
///
/// Same rules as the plugin table: append-only, presence decided by `size`, and
/// entries are plain function pointers. `user_data` is what makes the callbacks
/// usable: without it a host has nowhere to keep the state the callback needs.
/// A host that does not support a callback installs a no-op stub rather than a
/// null pointer.
///
/// There is deliberately no `complete` entry. A call completes when a poll of
/// its future returns `Ok`, and a callback that duplicated that would be a
/// second protocol for the same event.
///
/// Each callback is an `extern "C"` frame reached from plugin code, so a panic
/// inside one unwinds into plugin frames and aborts across the boundary. The
/// host must wrap every callback body in `catch_unwind` and turn a panic into a
/// status the plugin sees, exactly as the plugin side wraps its exported
/// adapters; only the callback's own frame can do this, because the plugin that
/// called it catches nothing.
#[repr(C)]
pub struct MetisHostApiV1 {
    /// Size in bytes of this table. First field, always.
    pub size: usize,
    /// Opaque host state handed back to every callback.
    pub user_data: *mut c_void,
    /// Progress of an in-flight call.
    pub progress: MetisProgressFn,
    /// A plugin-originated event.
    pub emit_event: MetisEmitEventFn,
    /// A log line.
    pub log: MetisLogFn,
    /// A host service the plugin awaits.
    pub service_request: MetisServiceRequestFn,
}

// --- presence decided by size --------------------------------------------
//
// Every table is size-prefixed, so presence is a comparison against `size` and
// never a look at the bytes in a slot: an older table simply has fewer bytes,
// and what sits past its `size` is whatever that build had there.
//
// The predicates below are the contract for a Rust reader. The `*_SLOT`
// constants are what a non-Rust reader needs, and `cbindgen` emits them into
// the committed header as `#define`s (it cannot emit a `const fn`, and a
// computed `size_of` expression is not rendered, both measured). Each one names
// the entry's zero-based pointer slot after the table's version prefix, so the
// C form of the same predicate is:
//
//   size >= sizeof(MetisTablePrefix)
//           + (METIS_PLUGIN_TABLE_V1_<ENTRY>_SLOT + 1) * sizeof(void *)
//
// for the plugin table, and the same expression with the host table's own
// prefix width for the host table.

/// Whether a plugin table that claims `size` bytes covers `invoke_future`.
pub const fn metis_plugin_table_has_invoke_future(size: usize) -> bool {
    size >= METIS_PLUGIN_TABLE_V1_INVOKE_FUTURE_THRESHOLD
}

/// Whether a plugin table that claims `size` bytes covers `service_complete`.
pub const fn metis_plugin_table_has_service_complete(size: usize) -> bool {
    size >= METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_THRESHOLD
}

/// Whether a host table that claims `size` bytes covers `service_request`.
pub const fn metis_host_api_has_service_request(size: usize) -> bool {
    size >= METIS_HOST_API_V1_SERVICE_REQUEST_THRESHOLD
}

/// The `create_instance` pointer slot of the plugin table, after its prefix.
pub const METIS_PLUGIN_TABLE_V1_CREATE_INSTANCE_SLOT: usize = 0;
/// The `destroy_instance` pointer slot of the plugin table, after its prefix.
pub const METIS_PLUGIN_TABLE_V1_DESTROY_INSTANCE_SLOT: usize = 1;
/// The `list_capabilities` pointer slot of the plugin table, after its prefix.
pub const METIS_PLUGIN_TABLE_V1_LIST_CAPABILITIES_SLOT: usize = 2;
/// The `invoke_future` pointer slot of the plugin table, after its prefix.
pub const METIS_PLUGIN_TABLE_V1_INVOKE_FUTURE_SLOT: usize = 3;
/// The `cancel` pointer slot of the plugin table, after its prefix.
pub const METIS_PLUGIN_TABLE_V1_CANCEL_SLOT: usize = 4;
/// The `release_buffer` pointer slot of the plugin table, after its prefix.
pub const METIS_PLUGIN_TABLE_V1_RELEASE_BUFFER_SLOT: usize = 5;
/// The `shutdown` pointer slot of the plugin table, after its prefix.
pub const METIS_PLUGIN_TABLE_V1_SHUTDOWN_SLOT: usize = 6;
/// The `service_complete` pointer slot of the plugin table, after its prefix.
pub const METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_SLOT: usize = 7;
/// The number of pointer slots in the plugin table, after its prefix.
pub const METIS_PLUGIN_TABLE_V1_SLOTS: usize = 8;

/// The `user_data` pointer slot of the host table, after its `size` field.
pub const METIS_HOST_API_V1_USER_DATA_SLOT: usize = 0;
/// The `progress` pointer slot of the host table, after its `size` field.
pub const METIS_HOST_API_V1_PROGRESS_SLOT: usize = 1;
/// The `emit_event` pointer slot of the host table, after its `size` field.
pub const METIS_HOST_API_V1_EMIT_EVENT_SLOT: usize = 2;
/// The `log` pointer slot of the host table, after its `size` field.
pub const METIS_HOST_API_V1_LOG_SLOT: usize = 3;
/// The `service_request` pointer slot of the host table, after its `size`
/// field.
pub const METIS_HOST_API_V1_SERVICE_REQUEST_SLOT: usize = 4;
/// The number of pointer slots in the host table, after its `size` field.
pub const METIS_HOST_API_V1_SLOTS: usize = 5;

/// One pointer slot, which is also one `size_t` on the supported targets (the C
/// translation unit asserts the equality), so the thresholds below are written
/// in one number.
const SLOT: usize = size_of::<*const c_void>();

/// The byte size a plugin table needs before `invoke_future` may be read.
///
/// This is the prefix plus the slots before it plus that slot: the number a
/// reader compares `size` against.
pub const METIS_PLUGIN_TABLE_V1_INVOKE_FUTURE_THRESHOLD: usize =
    size_of::<MetisTablePrefix>() + (METIS_PLUGIN_TABLE_V1_INVOKE_FUTURE_SLOT + 1) * SLOT;

/// The byte size a plugin table needs before `service_complete` may be read.
pub const METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_THRESHOLD: usize =
    size_of::<MetisTablePrefix>() + (METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_SLOT + 1) * SLOT;

/// The byte size a host table needs before `service_request` may be called.
pub const METIS_HOST_API_V1_SERVICE_REQUEST_THRESHOLD: usize =
    SLOT + (METIS_HOST_API_V1_SERVICE_REQUEST_SLOT + 1) * SLOT;

// --- entry point ----------------------------------------------------------

/// The signature of `metis_plugin_entry_v1`.
///
/// Argument-free and frozen within a major version: the symbol name carries
/// only the major, so a signature that changed inside major 1 could not be
/// detected before the call. No entry point takes `MetisPluginTableV1` in a
/// parameter either, because cbindgen then emits the typedef before the struct
/// and C scopes the struct tag to the prototype (measured).
pub type MetisPluginEntryV1Fn = unsafe extern "C" fn() -> *const MetisPluginTableV1;
