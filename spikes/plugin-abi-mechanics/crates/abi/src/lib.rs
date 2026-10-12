//! Spike-only rendering of the plugin C ABI primitives (D1, D4, D6, D7, D8).
//!
//! This is throwaway prototype code. It is not `crates/plugin-contract` and it
//! must not be treated as the contract; it exists so the decisions in
//! `docs/tasks/metis-foundations/tasks/plugin-abi-layout/task.md` can be
//! compiled and loaded once. See the findings beside this spike's task.
//!
//! Everything in this crate is part of the C ABI surface, so `cbindgen` sees
//! only this crate. Payload types (JSON shapes) live in `spike-payload` and are
//! deliberately outside the C ABI surface.

#![allow(clippy::missing_safety_doc)]

mod layout;

use core::ffi::c_void;

/// ABI major version. It is part of the exported symbol name (D8).
pub const METIS_ABI_MAJOR: u32 = 1;
/// ABI minor version. It travels in the function table (D8).
pub const METIS_ABI_MINOR: u32 = 0;

/// The name of the exported entry point for ABI major 1 (D8).
pub const METIS_PLUGIN_ENTRY_V1: &[u8] = b"metis_plugin_entry_v1\0";

// --- payload header (D4) ---------------------------------------------------

/// Mask for the low four bits: payload format version.
pub const METIS_PAYLOAD_VERSION_MASK: u8 = 0x0f;
/// Bit 4: codec selector.
pub const METIS_PAYLOAD_CODEC_BIT: u8 = 0x10;
/// Bits 5 to 7: reserved, must be zero.
pub const METIS_PAYLOAD_RESERVED_MASK: u8 = 0xe0;
/// The only payload format version defined at first.
pub const METIS_PAYLOAD_FORMAT_VERSION: u8 = 1;
/// Codec 0: UTF-8 JSON.
pub const METIS_CODEC_JSON: u8 = 0;
/// Codec 1: MessagePack, defined but unused (D4).
pub const METIS_CODEC_MESSAGEPACK: u8 = 1;

/// Builds the one-byte payload header.
pub const fn metis_payload_header(format_version: u8, codec: u8) -> u8 {
    (format_version & METIS_PAYLOAD_VERSION_MASK) | ((codec & 1) << 4)
}

/// Low four bits: payload format version.
pub const fn metis_payload_format_version(header: u8) -> u8 {
    header & METIS_PAYLOAD_VERSION_MASK
}

/// Bit 4: codec selector.
pub const fn metis_payload_codec(header: u8) -> u8 {
    (header & METIS_PAYLOAD_CODEC_BIT) >> 4
}

/// Bits 5 to 7: reserved.
pub const fn metis_payload_reserved(header: u8) -> u8 {
    header & METIS_PAYLOAD_RESERVED_MASK
}

/// Validates a payload header. This is the single implementation of the D4
/// rule, shared by host and plugin.
pub fn metis_validate_payload_header(header: u8) -> Result<(), MetisStatus> {
    if metis_payload_reserved(header) != 0 {
        return Err(MetisStatus::InvalidArgument);
    }
    if metis_payload_format_version(header) != METIS_PAYLOAD_FORMAT_VERSION {
        return Err(MetisStatus::InvalidArgument);
    }
    if metis_payload_codec(header) != METIS_CODEC_JSON {
        return Err(MetisStatus::Unsupported);
    }
    Ok(())
}

/// Encodes a body with a version 1 JSON header.
pub fn metis_encode_payload(body: &[u8]) -> Vec<u8> {
    metis_encode_payload_with(metis_payload_header(METIS_PAYLOAD_FORMAT_VERSION, METIS_CODEC_JSON), body)
}

/// Encodes a body with an arbitrary header byte, for negative tests.
pub fn metis_encode_payload_with(header: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 1);
    out.push(header);
    out.extend_from_slice(body);
    out
}

/// Strips and validates the payload header, returning the body.
pub fn metis_payload_body(bytes: &[u8]) -> Result<&[u8], MetisStatus> {
    match bytes.split_first() {
        Some((&header, body)) => {
            metis_validate_payload_header(header)?;
            Ok(body)
        }
        None => Err(MetisStatus::InvalidArgument),
    }
}

// --- status (D7) -----------------------------------------------------------

/// Frozen status enum (D7).
///
/// Spike revision: `Pending` is appended. The D7 list of seven members has no
/// way to express "the result arrives through `complete`", which D11 requires.
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
    /// Spike addition, see the module docs and the findings.
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

// --- buffers (D6) ----------------------------------------------------------

/// No owner: an unset buffer.
pub const METIS_BUFFER_OWNER_NONE: u32 = 0;
/// Allocated by the host, freed by the host.
pub const METIS_BUFFER_OWNER_HOST: u32 = 1;
/// Allocated by the plugin, released through `release_buffer`.
pub const METIS_BUFFER_OWNER_PLUGIN: u32 = 2;

/// A borrowed or owned byte buffer crossing the boundary (D6).
///
/// The host-allocated variant is only valid for the duration of the call that
/// received it. The plugin-allocated variant is released by the plugin table's
/// `release_buffer`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MetisBuffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
    /// One of the `METIS_BUFFER_OWNER_*` constants.
    pub owner: u32,
    /// Reserved, zero.
    pub reserved: u32,
}

impl MetisBuffer {
    pub const fn empty() -> Self {
        MetisBuffer { ptr: core::ptr::null_mut(), len: 0, cap: 0, owner: METIS_BUFFER_OWNER_NONE, reserved: 0 }
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
            .field("reserved", &self.reserved)
            .finish()
    }
}

// --- log levels ------------------------------------------------------------

pub const METIS_LOG_DEBUG: i32 = 0;
pub const METIS_LOG_INFO: i32 = 1;
pub const METIS_LOG_WARN: i32 = 2;
pub const METIS_LOG_ERROR: i32 = 3;

// --- plugin table (D1, D8) -------------------------------------------------

/// The smallest prefix of any table: enough to read the size and the version
/// fields without assuming anything about the rest (D8).
#[repr(C)]
pub struct MetisTablePrefix {
    pub size: usize,
    pub abi_major: u32,
    pub abi_minor: u32,
}

pub type MetisCreateInstanceFn =
    unsafe extern "C" fn(host: *const MetisHostApiV1, out_instance: *mut *mut c_void) -> MetisStatus;

pub type MetisDestroyInstanceFn = unsafe extern "C" fn(instance: *mut c_void) -> MetisStatus;

/// Returns a plugin-allocated payload holding the capability descriptors.
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

pub type MetisCancelFn = unsafe extern "C" fn(instance: *mut c_void, request_id: u64) -> MetisStatus;

pub type MetisReleaseBufferFn = unsafe extern "C" fn(buffer: MetisBuffer) -> MetisStatus;

pub type MetisShutdownFn = unsafe extern "C" fn(instance: *mut c_void) -> MetisStatus;

/// The version 1 function table (D1, D8).
///
/// Append-only evolution: new entries are appended after `shutdown` and the
/// `size` field grows. A reader must compare `size` against
/// `size_of::<MetisPluginTableV1>()` (or the prefix it knows) before touching a
/// field.
///
/// Presence of an entry is decided by `size`, not by a null pointer. The fields
/// are plain function pointers on purpose: cbindgen 0.29 renders
/// `Option<NamedFnAlias>` as an opaque struct, which produces a header that a C
/// compiler rejects. See the findings for the measured behaviour.
#[repr(C)]
pub struct MetisPluginTableV1 {
    /// Size in bytes of the table the plugin returned.
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
}

// --- host table (D1, D11, D12) ---------------------------------------------

/// A completion arrives with a plugin-allocated payload that the receiver of
/// the callback still has to release through `release_buffer`, even when the
/// receiver discards the payload.
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

pub type MetisLogFn =
    unsafe extern "C" fn(user_data: *mut c_void, level: i32, message: *const u8, message_len: usize);

/// The version 1 host table (D1).
///
/// Same rule as the plugin table: entries are plain function pointers and a
/// host that does not support a callback installs a no-op stub. A nullable
/// entry would have to be written inline as
/// `Option<unsafe extern "C" fn(...)>`, which cbindgen renders correctly but
/// without the named typedef.
#[repr(C)]
pub struct MetisHostApiV1 {
    /// Size in bytes of the host table.
    pub size: usize,
    pub user_data: *mut c_void,
    pub complete: MetisCompleteFn,
    pub progress: MetisProgressFn,
    pub emit_event: MetisEmitEventFn,
    pub log: MetisLogFn,
}

// --- entry point (D8) ------------------------------------------------------

/// Signature of the exported entry point, `metis_plugin_entry_v1`.
///
/// No entry point mentions `MetisPluginTableV1` in its parameters: letting one
/// do so makes cbindgen emit the function typedef before the struct, and C then
/// scopes the struct tag to the prototype. See the findings.
///
/// The entry point takes no arguments on purpose: the symbol name already
/// carries the ABI major version, and a signature that changed within a major
/// version could not be detected before the call.
pub type MetisPluginEntryV1Fn = unsafe extern "C" fn() -> *const MetisPluginTableV1;

/// Reads the prefix fields of a table after checking that the reported size
/// covers them.
///
/// # Safety
///
/// `table` must point at a `MetisPluginTableV1` (or anything whose first three
/// fields have this layout) that stays alive for the returned reference.
pub unsafe fn metis_read_prefix<'a>(table: *const MetisPluginTableV1) -> Option<&'a MetisTablePrefix> {
    if table.is_null() {
        return None;
    }
    let prefix = unsafe { &*(table as *const MetisTablePrefix) };
    // The `size` field is itself the first field, so it is readable whenever
    // the pointer is valid; only the fields after it need the check.
    let needed = core::mem::offset_of!(MetisTablePrefix, abi_minor) + core::mem::size_of::<u32>();
    if prefix.size < needed {
        return None;
    }
    Some(prefix)
}
