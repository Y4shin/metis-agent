//! Compile-time layout assertions for every type that crosses the ABI.
//!
//! These run on every build of this crate, not only under `cargo test`, and
//! they are the Rust half of the agreement: `c/layout_static_asserts.c`
//! re-asserts the same numbers in C, compiled by `build.rs` against the
//! committed generated header, and `tests/c_layout.rs` compares the values the
//! C compiler measured with the values asserted here.
//!
//! Every number is written through a size-of expression rather than as a
//! 64-bit literal, so the assertions hold per target instead of only on the
//! machine that wrote them.

use core::mem::{align_of, offset_of, size_of};

use super::*;

/// A pointer slot. `ptr` and `cap` in the generated header are `void *` and
/// `size_t`, which share a size on the supported targets.
const PTR: usize = size_of::<*const core::ffi::c_void>();
/// The `size` fields, which are `size_t` in the generated header.
const SIZE_T: usize = size_of::<usize>();
/// The version fields, and the buffer's owner and reserved tags.
const U32: usize = size_of::<u32>();

// --- MetisStatus -----------------------------------------------------------

// A `repr(C)` fieldless enum is a C `int`, and the members are frozen, so the
// numeric values are part of the ABI, not an implementation detail.
const _: () = assert!(size_of::<MetisStatus>() == U32);
const _: () = assert!(align_of::<MetisStatus>() == align_of::<u32>());
const _: () = assert!(MetisStatus::Ok as i32 == 0);
const _: () = assert!(MetisStatus::InvalidArgument as i32 == 1);
const _: () = assert!(MetisStatus::NotFound as i32 == 2);
const _: () = assert!(MetisStatus::PermissionDenied as i32 == 3);
const _: () = assert!(MetisStatus::Cancelled as i32 == 4);
const _: () = assert!(MetisStatus::Internal as i32 == 5);
const _: () = assert!(MetisStatus::Unsupported as i32 == 6);
const _: () = assert!(MetisStatus::Pending as i32 == 7);

// --- MetisBuffer -----------------------------------------------------------

// `ptr`, `len`, `cap`, `owner`, `reserved`: three pointer-sized fields and two
// `uint32_t` tags, with no padding between them.
const _: () = assert!(size_of::<MetisBuffer>() == 3 * PTR + 2 * U32);
const _: () = assert!(align_of::<MetisBuffer>() == align_of::<*mut u8>());
const _: () = assert!(offset_of!(MetisBuffer, ptr) == 0);
const _: () = assert!(offset_of!(MetisBuffer, len) == PTR);
const _: () = assert!(offset_of!(MetisBuffer, cap) == 2 * PTR);
const _: () = assert!(offset_of!(MetisBuffer, owner) == 3 * PTR);
const _: () = assert!(offset_of!(MetisBuffer, reserved) == 3 * PTR + U32);

// --- MetisTablePrefix ------------------------------------------------------

// `size`, `abi_major`, `abi_minor`: the shortest thing any reader touches, and
// the reason `size` is the first field.
const _: () = assert!(size_of::<MetisTablePrefix>() == SIZE_T + 2 * U32);
const _: () = assert!(align_of::<MetisTablePrefix>() == align_of::<usize>());
const _: () = assert!(offset_of!(MetisTablePrefix, size) == 0);
const _: () = assert!(offset_of!(MetisTablePrefix, abi_major) == SIZE_T);
const _: () = assert!(offset_of!(MetisTablePrefix, abi_minor) == SIZE_T + U32);

// --- MetisFutureVtable and MetisFutureHandle ------------------------------

// Two function pointer slots and no `size` field, which is the revision the
// contract made: the vtable carries only the entries the host calls.
const _: () = assert!(size_of::<MetisFutureVtable>() == 2 * PTR);
const _: () = assert!(align_of::<MetisFutureVtable>() == align_of::<*const core::ffi::c_void>());
const _: () = assert!(offset_of!(MetisFutureVtable, poll) == 0);
const _: () = assert!(offset_of!(MetisFutureVtable, drop) == PTR);
const _: () = assert!(offset_of!(MetisFutureVtable, drop) + PTR == size_of::<MetisFutureVtable>());

// A context pointer and a vtable pointer, in that order.
const _: () = assert!(size_of::<MetisFutureHandle>() == 2 * PTR);
const _: () = assert!(align_of::<MetisFutureHandle>() == align_of::<*mut core::ffi::c_void>());
const _: () = assert!(offset_of!(MetisFutureHandle, ctx) == 0);
const _: () = assert!(offset_of!(MetisFutureHandle, vtable) == PTR);
const _: () =
    assert!(offset_of!(MetisFutureHandle, vtable) + PTR == size_of::<MetisFutureHandle>());

// --- MetisWakerVtable and MetisWaker --------------------------------------

// Four function pointer slots and no `size` field.
const _: () = assert!(size_of::<MetisWakerVtable>() == 4 * PTR);
const _: () = assert!(align_of::<MetisWakerVtable>() == align_of::<*const core::ffi::c_void>());
const _: () = assert!(offset_of!(MetisWakerVtable, clone) == 0);
const _: () = assert!(offset_of!(MetisWakerVtable, wake) == PTR);
const _: () = assert!(offset_of!(MetisWakerVtable, wake_by_ref) == 2 * PTR);
const _: () = assert!(offset_of!(MetisWakerVtable, drop) == 3 * PTR);
const _: () = assert!(offset_of!(MetisWakerVtable, drop) + PTR == size_of::<MetisWakerVtable>());

// The data pointer and the vtable pointer the four entries above are reached
// through.
const _: () = assert!(size_of::<MetisWaker>() == 2 * PTR);
const _: () = assert!(align_of::<MetisWaker>() == align_of::<*mut core::ffi::c_void>());
const _: () = assert!(offset_of!(MetisWaker, data) == 0);
const _: () = assert!(offset_of!(MetisWaker, vtable) == PTR);
const _: () = assert!(offset_of!(MetisWaker, vtable) + PTR == size_of::<MetisWaker>());

// --- MetisPluginTableV1 ----------------------------------------------------

// The prefix, then eight function pointer slots. Append-only works because the
// entry a host knows lands exactly on a pointer boundary, first at
// `size_of::<MetisTablePrefix>()` and then one pointer at a time.
const _: () = assert!(size_of::<MetisPluginTableV1>() == size_of::<MetisTablePrefix>() + 8 * PTR);
const _: () = assert!(align_of::<MetisPluginTableV1>() == align_of::<*const core::ffi::c_void>());
const _: () = assert!(offset_of!(MetisPluginTableV1, size) == 0);
const _: () = assert!(offset_of!(MetisPluginTableV1, abi_major) == SIZE_T);
const _: () = assert!(offset_of!(MetisPluginTableV1, abi_minor) == SIZE_T + U32);
const _: () =
    assert!(offset_of!(MetisPluginTableV1, create_instance) == size_of::<MetisTablePrefix>());
const _: () = assert!(
    offset_of!(MetisPluginTableV1, destroy_instance) == size_of::<MetisTablePrefix>() + PTR
);
const _: () = assert!(
    offset_of!(MetisPluginTableV1, list_capabilities) == size_of::<MetisTablePrefix>() + 2 * PTR
);
const _: () = assert!(
    offset_of!(MetisPluginTableV1, invoke_future) == size_of::<MetisTablePrefix>() + 3 * PTR
);
const _: () =
    assert!(offset_of!(MetisPluginTableV1, cancel) == size_of::<MetisTablePrefix>() + 4 * PTR);
const _: () = assert!(
    offset_of!(MetisPluginTableV1, release_buffer) == size_of::<MetisTablePrefix>() + 5 * PTR
);
const _: () =
    assert!(offset_of!(MetisPluginTableV1, shutdown) == size_of::<MetisTablePrefix>() + 6 * PTR);
const _: () = assert!(
    offset_of!(MetisPluginTableV1, service_complete) == size_of::<MetisTablePrefix>() + 7 * PTR
);
// An appended entry must land exactly after the current tail, otherwise
// "append-only" is not a layout guarantee.
const _: () = assert!(
    offset_of!(MetisPluginTableV1, service_complete) + PTR == size_of::<MetisPluginTableV1>()
);

// The slot constants and the presence thresholds are the same numbers, written
// for a reader that has only a `size` to compare against.
const _: () = assert!(
    METIS_PLUGIN_TABLE_V1_INVOKE_FUTURE_SLOT * PTR + size_of::<MetisTablePrefix>()
        == offset_of!(MetisPluginTableV1, invoke_future)
);
const _: () = assert!(
    METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_SLOT * PTR + size_of::<MetisTablePrefix>()
        == offset_of!(MetisPluginTableV1, service_complete)
);
const _: () =
    assert!(METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_SLOT + 1 == METIS_PLUGIN_TABLE_V1_SLOTS);
const _: () = assert!(
    METIS_PLUGIN_TABLE_V1_INVOKE_FUTURE_THRESHOLD
        == offset_of!(MetisPluginTableV1, invoke_future) + PTR
);
const _: () = assert!(
    METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_THRESHOLD
        == offset_of!(MetisPluginTableV1, service_complete) + PTR
);

// --- MetisHostApiV1 --------------------------------------------------------

// `size`, `user_data`, and the four callbacks.
const _: () = assert!(size_of::<MetisHostApiV1>() == SIZE_T + 5 * PTR);
const _: () = assert!(align_of::<MetisHostApiV1>() == align_of::<*const core::ffi::c_void>());
const _: () = assert!(offset_of!(MetisHostApiV1, size) == 0);
const _: () = assert!(offset_of!(MetisHostApiV1, user_data) == SIZE_T);
const _: () = assert!(offset_of!(MetisHostApiV1, progress) == SIZE_T + PTR);
const _: () = assert!(offset_of!(MetisHostApiV1, emit_event) == SIZE_T + 2 * PTR);
const _: () = assert!(offset_of!(MetisHostApiV1, log) == SIZE_T + 3 * PTR);
const _: () = assert!(offset_of!(MetisHostApiV1, service_request) == SIZE_T + 4 * PTR);
const _: () =
    assert!(offset_of!(MetisHostApiV1, service_request) + PTR == size_of::<MetisHostApiV1>());

const _: () = assert!(
    METIS_HOST_API_V1_SERVICE_REQUEST_SLOT * PTR + SIZE_T
        == offset_of!(MetisHostApiV1, service_request)
);
const _: () = assert!(METIS_HOST_API_V1_SERVICE_REQUEST_SLOT + 1 == METIS_HOST_API_V1_SLOTS);
const _: () = assert!(
    METIS_HOST_API_V1_SERVICE_REQUEST_THRESHOLD
        == offset_of!(MetisHostApiV1, service_request) + PTR
);
