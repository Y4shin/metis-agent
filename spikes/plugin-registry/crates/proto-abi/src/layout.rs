//! Compile-time layout assertions for every type that crosses the ABI, the new
//! host-polled future types included.
//!
//! These run on every build, not only under `cargo test`, and they are the Rust
//! half of the agreement: `c/layout_static_asserts.c` re-asserts the same
//! numbers in pointer-size terms, is compiled by `build.rs` against the
//! committed generated header, and `tests/layout.rs` compares the values the C
//! compiler measured with the values asserted here.
//!
//! Every number is a size-of expression rather than a 64-bit literal, so the
//! assertions hold per target.

use core::mem::{align_of, offset_of, size_of};

use super::*;

/// A pointer slot, which is what every function-pointer field is.
const PTR: usize = size_of::<*const core::ffi::c_void>();
/// The `size` fields, which are `size_t` in the generated header.
const SIZE_T: usize = size_of::<usize>();
/// The version fields and the buffer's owner and reserved tags.
const U32: usize = size_of::<u32>();

// --- the landed prefix -----------------------------------------------------

const _: () = assert!(size_of::<MetisBuffer>() == 3 * PTR + 2 * U32);
const _: () = assert!(align_of::<MetisBuffer>() == align_of::<*mut u8>());
const _: () = assert!(offset_of!(MetisBuffer, ptr) == 0);
const _: () = assert!(offset_of!(MetisBuffer, len) == PTR);
const _: () = assert!(offset_of!(MetisBuffer, cap) == 2 * PTR);
const _: () = assert!(offset_of!(MetisBuffer, owner) == 3 * PTR);
const _: () = assert!(offset_of!(MetisBuffer, reserved) == 3 * PTR + U32);

const _: () = assert!(size_of::<MetisTablePrefix>() == SIZE_T + 2 * U32);
const _: () = assert!(align_of::<MetisTablePrefix>() == align_of::<usize>());
const _: () = assert!(offset_of!(MetisTablePrefix, size) == 0);
const _: () = assert!(offset_of!(MetisTablePrefix, abi_major) == SIZE_T);
const _: () = assert!(offset_of!(MetisTablePrefix, abi_minor) == SIZE_T + U32);

// --- the plugin table with the two appended entries ------------------------

// Nine function-pointer slots: the seven landed entries, then `invoke_future`
// and `service_complete`. Append-only holds because the appended entries land
// exactly on pointer boundaries after `shutdown`.
const _: () = assert!(size_of::<MetisPluginTableV1>() == size_of::<MetisTablePrefix>() + 9 * PTR);
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
const _: () =
    assert!(offset_of!(MetisPluginTableV1, invoke) == size_of::<MetisTablePrefix>() + 3 * PTR);
const _: () =
    assert!(offset_of!(MetisPluginTableV1, cancel) == size_of::<MetisTablePrefix>() + 4 * PTR);
const _: () = assert!(
    offset_of!(MetisPluginTableV1, release_buffer) == size_of::<MetisTablePrefix>() + 5 * PTR
);
const _: () =
    assert!(offset_of!(MetisPluginTableV1, shutdown) == size_of::<MetisTablePrefix>() + 6 * PTR);
// The landed table stops exactly at the end of `shutdown`; the first appended
// entry must start there, or "appended" is not a layout guarantee.
const _: () =
    assert!(offset_of!(MetisPluginTableV1, invoke_future) == size_of::<MetisTablePrefix>() + 7 * PTR);
const _: () = assert!(
    offset_of!(MetisPluginTableV1, service_complete) == size_of::<MetisTablePrefix>() + 8 * PTR
);
const _: () = assert!(
    offset_of!(MetisPluginTableV1, service_complete) + PTR == size_of::<MetisPluginTableV1>()
);
// The landed prefix size is the size an older reader knows, and it is exactly
// the offset of the first appended entry.
const _: () = assert!(METIS_PLUGIN_TABLE_V1_LANDED_SIZE == size_of::<MetisTablePrefix>() + 7 * PTR);
const _: () = assert!(METIS_PLUGIN_TABLE_V1_LANDED_SIZE == offset_of!(MetisPluginTableV1, invoke_future));
const _: () = assert!(metis_plugin_table_has_invoke_future(METIS_PLUGIN_TABLE_V1_LANDED_SIZE) == false);
const _: () = assert!(metis_plugin_table_has_invoke_future(METIS_PLUGIN_TABLE_V1_LANDED_SIZE + PTR) == true);
const _: () = assert!(metis_plugin_table_has_service_complete(METIS_PLUGIN_TABLE_V1_LANDED_SIZE + PTR) == false);

// --- the host table with the appended service entry ------------------------

const _: () = assert!(size_of::<MetisHostApiV1>() == SIZE_T + 6 * PTR);
const _: () = assert!(align_of::<MetisHostApiV1>() == align_of::<*const core::ffi::c_void>());
const _: () = assert!(offset_of!(MetisHostApiV1, size) == 0);
const _: () = assert!(offset_of!(MetisHostApiV1, user_data) == SIZE_T);
const _: () = assert!(offset_of!(MetisHostApiV1, complete) == SIZE_T + PTR);
const _: () = assert!(offset_of!(MetisHostApiV1, progress) == SIZE_T + 2 * PTR);
const _: () = assert!(offset_of!(MetisHostApiV1, emit_event) == SIZE_T + 3 * PTR);
const _: () = assert!(offset_of!(MetisHostApiV1, log) == SIZE_T + 4 * PTR);
const _: () = assert!(offset_of!(MetisHostApiV1, service_request) == SIZE_T + 5 * PTR);
const _: () = assert!(offset_of!(MetisHostApiV1, service_request) + PTR == size_of::<MetisHostApiV1>());
const _: () = assert!(METIS_HOST_API_V1_LANDED_SIZE == SIZE_T + 5 * PTR);
const _: () = assert!(METIS_HOST_API_V1_LANDED_SIZE == offset_of!(MetisHostApiV1, service_request));
const _: () = assert!(metis_host_api_has_service_request(METIS_HOST_API_V1_LANDED_SIZE) == false);
const _: () = assert!(metis_host_api_has_service_request(METIS_HOST_API_V1_LANDED_SIZE + PTR) == true);

// --- the new host-polled future types --------------------------------------

// A handle is a context pointer and a vtable pointer.
const _: () = assert!(size_of::<MetisFutureHandle>() == 2 * PTR);
const _: () = assert!(align_of::<MetisFutureHandle>() == align_of::<*mut core::ffi::c_void>());
const _: () = assert!(offset_of!(MetisFutureHandle, ctx) == 0);
const _: () = assert!(offset_of!(MetisFutureHandle, vtable) == PTR);

// The future vtable is `size` plus two function pointers, size first.
const _: () = assert!(size_of::<MetisFutureVtable>() == SIZE_T + 2 * PTR);
const _: () = assert!(align_of::<MetisFutureVtable>() == align_of::<*const core::ffi::c_void>());
const _: () = assert!(offset_of!(MetisFutureVtable, size) == 0);
const _: () = assert!(offset_of!(MetisFutureVtable, poll) == SIZE_T);
const _: () = assert!(offset_of!(MetisFutureVtable, drop) == SIZE_T + PTR);
const _: () = assert!(offset_of!(MetisFutureVtable, drop) + PTR == size_of::<MetisFutureVtable>());

// A waker is a data pointer and a vtable pointer.
const _: () = assert!(size_of::<MetisWaker>() == 2 * PTR);
const _: () = assert!(align_of::<MetisWaker>() == align_of::<*mut core::ffi::c_void>());
const _: () = assert!(offset_of!(MetisWaker, data) == 0);
const _: () = assert!(offset_of!(MetisWaker, vtable) == PTR);

// The waker vtable is `size` plus clone, wake, wake_by_ref and drop, size first.
const _: () = assert!(size_of::<MetisWakerVtable>() == SIZE_T + 4 * PTR);
const _: () = assert!(align_of::<MetisWakerVtable>() == align_of::<*const core::ffi::c_void>());
const _: () = assert!(offset_of!(MetisWakerVtable, size) == 0);
const _: () = assert!(offset_of!(MetisWakerVtable, clone) == SIZE_T);
const _: () = assert!(offset_of!(MetisWakerVtable, wake) == SIZE_T + PTR);
const _: () = assert!(offset_of!(MetisWakerVtable, wake_by_ref) == SIZE_T + 2 * PTR);
const _: () = assert!(offset_of!(MetisWakerVtable, drop) == SIZE_T + 3 * PTR);
const _: () = assert!(offset_of!(MetisWakerVtable, drop) + PTR == size_of::<MetisWakerVtable>());

// The argument-free entry point keeps its signature: changing it would change
// the symbol's meaning inside major 1.
const _: () = assert!(size_of::<MetisPluginEntryV1Fn>() == PTR);
