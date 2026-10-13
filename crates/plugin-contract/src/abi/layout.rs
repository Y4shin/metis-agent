//! Compile-time layout assertions for every type that crosses the ABI (D13).
//!
//! These run on every build of this crate, not only under `cargo test`, and
//! they are the Rust half of the agreement: `c/layout_static_asserts.c`
//! re-asserts the same numbers in C, compiled by `build.rs` against the
//! committed generated header, and `tests/c_layout.rs` compares the values the
//! C compiler measured with the values asserted here.
//!
//! Every number is written through a size-of expression rather than as a
//! 64-bit literal, so the assertions hold per target instead of only on the
//! machine that wrote them (D13 amendment).

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

// A `repr(C)` fieldless enum is a C `int`, and the members are frozen (D7), so
// the numeric values are part of the ABI, not an implementation detail.
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
// the reason `size` is the first field (D8).
const _: () = assert!(size_of::<MetisTablePrefix>() == SIZE_T + 2 * U32);
const _: () = assert!(align_of::<MetisTablePrefix>() == align_of::<usize>());
const _: () = assert!(offset_of!(MetisTablePrefix, size) == 0);
const _: () = assert!(offset_of!(MetisTablePrefix, abi_major) == SIZE_T);
const _: () = assert!(offset_of!(MetisTablePrefix, abi_minor) == SIZE_T + U32);

// --- MetisPluginTableV1 ----------------------------------------------------

// The prefix, then seven function pointer slots. Append-only works because the
// entry a host knows lands exactly on a pointer boundary, first at
// `size_of::<MetisTablePrefix>()` and then one pointer at a time.
const _: () = assert!(size_of::<MetisPluginTableV1>() == size_of::<MetisTablePrefix>() + 7 * PTR);
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
// An appended entry must land exactly after the current tail, otherwise
// "append-only" is not a layout guarantee.
const _: () =
    assert!(offset_of!(MetisPluginTableV1, shutdown) + PTR == size_of::<MetisPluginTableV1>());

// --- MetisHostApiV1 --------------------------------------------------------

// `size`, `user_data`, and the four callbacks.
const _: () = assert!(size_of::<MetisHostApiV1>() == SIZE_T + 5 * PTR);
const _: () = assert!(align_of::<MetisHostApiV1>() == align_of::<*const core::ffi::c_void>());
const _: () = assert!(offset_of!(MetisHostApiV1, size) == 0);
const _: () = assert!(offset_of!(MetisHostApiV1, user_data) == SIZE_T);
const _: () = assert!(offset_of!(MetisHostApiV1, complete) == SIZE_T + PTR);
const _: () = assert!(offset_of!(MetisHostApiV1, progress) == SIZE_T + 2 * PTR);
const _: () = assert!(offset_of!(MetisHostApiV1, emit_event) == SIZE_T + 3 * PTR);
const _: () = assert!(offset_of!(MetisHostApiV1, log) == SIZE_T + 4 * PTR);
const _: () = assert!(offset_of!(MetisHostApiV1, log) + PTR == size_of::<MetisHostApiV1>());
