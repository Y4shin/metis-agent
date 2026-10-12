//! Compile-time layout assertions for every type that crosses the ABI (D13).
//!
//! These run on every build of the crate, not only under `cargo test`. The
//! numbers are duplicated deliberately in `include/metis_plugin_abi.h`'s
//! consumers, specifically `tests/c/layout_static_asserts.c`.

use core::mem::{align_of, offset_of, size_of};

use crate::*;

// MetisStatus: a `repr(C)` fieldless enum is an `int` in C.
const _: () = assert!(size_of::<MetisStatus>() == 4);
const _: () = assert!(align_of::<MetisStatus>() == 4);

// MetisBuffer.
const _: () = assert!(size_of::<MetisBuffer>() == 32);
const _: () = assert!(align_of::<MetisBuffer>() == 8);
const _: () = assert!(offset_of!(MetisBuffer, ptr) == 0);
const _: () = assert!(offset_of!(MetisBuffer, len) == 8);
const _: () = assert!(offset_of!(MetisBuffer, cap) == 16);
const _: () = assert!(offset_of!(MetisBuffer, owner) == 24);
const _: () = assert!(offset_of!(MetisBuffer, reserved) == 28);

// MetisTablePrefix.
const _: () = assert!(size_of::<MetisTablePrefix>() == 16);
const _: () = assert!(align_of::<MetisTablePrefix>() == 8);
const _: () = assert!(offset_of!(MetisTablePrefix, size) == 0);
const _: () = assert!(offset_of!(MetisTablePrefix, abi_major) == 8);
const _: () = assert!(offset_of!(MetisTablePrefix, abi_minor) == 12);

// MetisPluginTableV1: 2 + 7 pointer-sized slots.
const _: () = assert!(size_of::<MetisPluginTableV1>() == 72);
const _: () = assert!(align_of::<MetisPluginTableV1>() == 8);
const _: () = assert!(offset_of!(MetisPluginTableV1, size) == 0);
const _: () = assert!(offset_of!(MetisPluginTableV1, abi_major) == 8);
const _: () = assert!(offset_of!(MetisPluginTableV1, abi_minor) == 12);
const _: () = assert!(offset_of!(MetisPluginTableV1, create_instance) == 16);
const _: () = assert!(offset_of!(MetisPluginTableV1, destroy_instance) == 24);
const _: () = assert!(offset_of!(MetisPluginTableV1, list_capabilities) == 32);
const _: () = assert!(offset_of!(MetisPluginTableV1, invoke) == 40);
const _: () = assert!(offset_of!(MetisPluginTableV1, cancel) == 48);
const _: () = assert!(offset_of!(MetisPluginTableV1, release_buffer) == 56);
const _: () = assert!(offset_of!(MetisPluginTableV1, shutdown) == 64);

// MetisHostApiV1: 2 + 4 pointer-sized slots.
const _: () = assert!(size_of::<MetisHostApiV1>() == 48);
const _: () = assert!(align_of::<MetisHostApiV1>() == 8);
const _: () = assert!(offset_of!(MetisHostApiV1, size) == 0);
const _: () = assert!(offset_of!(MetisHostApiV1, user_data) == 8);
const _: () = assert!(offset_of!(MetisHostApiV1, complete) == 16);
const _: () = assert!(offset_of!(MetisHostApiV1, progress) == 24);
const _: () = assert!(offset_of!(MetisHostApiV1, emit_event) == 32);
const _: () = assert!(offset_of!(MetisHostApiV1, log) == 40);

// An appended field must land exactly after the current tail, otherwise
// "append-only" is not a layout guarantee.
const _: () = assert!(offset_of!(MetisHostApiV1, log) + size_of::<usize>() == size_of::<MetisHostApiV1>());
const _: () = assert!(offset_of!(MetisPluginTableV1, shutdown) + size_of::<usize>() == size_of::<MetisPluginTableV1>());
