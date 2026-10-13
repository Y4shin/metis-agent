//! Layout seam: the C translation unit and this crate agree on every number
//! (D13).
//!
//! `c/layout_static_asserts.c` is compiled by `build.rs` into this test binary.
//! Its `_Static_assert`s already refuse to compile when the generated header
//! stops agreeing with the pointer-size formulas in `src/abi/layout.rs`. This
//! test adds the other direction: it compares the values the C compiler
//! actually measured with Rust's `size_of`, `align_of` and `offset_of`, so a
//! stale literal on either side is caught as well as a layout change.
//!
//! The C side is also where the *generated header* is proven usable: it is the
//! artifact a non-Rust implementer compiles against, and it is the only thing
//! the C file includes.

#![allow(non_upper_case_globals)]

use core::mem::{align_of, offset_of, size_of};

use plugin_contract::abi::*;

unsafe extern "C" {
    static metis_c_size_MetisStatus: usize;
    static metis_c_align_MetisStatus: usize;

    static metis_c_size_MetisBuffer: usize;
    static metis_c_align_MetisBuffer: usize;
    static metis_c_offset_MetisBuffer_ptr: usize;
    static metis_c_offset_MetisBuffer_len: usize;
    static metis_c_offset_MetisBuffer_cap: usize;
    static metis_c_offset_MetisBuffer_owner: usize;
    static metis_c_offset_MetisBuffer_reserved: usize;

    static metis_c_size_MetisTablePrefix: usize;
    static metis_c_align_MetisTablePrefix: usize;
    static metis_c_offset_MetisTablePrefix_size: usize;
    static metis_c_offset_MetisTablePrefix_abi_major: usize;
    static metis_c_offset_MetisTablePrefix_abi_minor: usize;

    static metis_c_size_MetisPluginTableV1: usize;
    static metis_c_align_MetisPluginTableV1: usize;
    static metis_c_offset_MetisPluginTableV1_size: usize;
    static metis_c_offset_MetisPluginTableV1_abi_major: usize;
    static metis_c_offset_MetisPluginTableV1_abi_minor: usize;
    static metis_c_offset_MetisPluginTableV1_create_instance: usize;
    static metis_c_offset_MetisPluginTableV1_destroy_instance: usize;
    static metis_c_offset_MetisPluginTableV1_list_capabilities: usize;
    static metis_c_offset_MetisPluginTableV1_invoke: usize;
    static metis_c_offset_MetisPluginTableV1_cancel: usize;
    static metis_c_offset_MetisPluginTableV1_release_buffer: usize;
    static metis_c_offset_MetisPluginTableV1_shutdown: usize;

    static metis_c_size_MetisHostApiV1: usize;
    static metis_c_align_MetisHostApiV1: usize;
    static metis_c_offset_MetisHostApiV1_size: usize;
    static metis_c_offset_MetisHostApiV1_user_data: usize;
    static metis_c_offset_MetisHostApiV1_complete: usize;
    static metis_c_offset_MetisHostApiV1_progress: usize;
    static metis_c_offset_MetisHostApiV1_emit_event: usize;
    static metis_c_offset_MetisHostApiV1_log: usize;
}

/// Asserts that the C compiler measured `$c_symbol` as `$rust_expr`.
macro_rules! c_agrees_with_rust {
    ($c_symbol:ident, $rust_expr:expr) => {
        assert_eq!(
            unsafe { $c_symbol },
            $rust_expr,
            "the C compiler's value for {} disagrees with Rust's",
            stringify!($c_symbol)
        );
    };
}

#[test]
fn c_and_rust_agree_on_sizes_and_alignments() {
    c_agrees_with_rust!(metis_c_size_MetisStatus, size_of::<MetisStatus>());
    c_agrees_with_rust!(metis_c_align_MetisStatus, align_of::<MetisStatus>());

    c_agrees_with_rust!(metis_c_size_MetisBuffer, size_of::<MetisBuffer>());
    c_agrees_with_rust!(metis_c_align_MetisBuffer, align_of::<MetisBuffer>());

    c_agrees_with_rust!(metis_c_size_MetisTablePrefix, size_of::<MetisTablePrefix>());
    c_agrees_with_rust!(
        metis_c_align_MetisTablePrefix,
        align_of::<MetisTablePrefix>()
    );

    c_agrees_with_rust!(
        metis_c_size_MetisPluginTableV1,
        size_of::<MetisPluginTableV1>()
    );
    c_agrees_with_rust!(
        metis_c_align_MetisPluginTableV1,
        align_of::<MetisPluginTableV1>()
    );

    c_agrees_with_rust!(metis_c_size_MetisHostApiV1, size_of::<MetisHostApiV1>());
    c_agrees_with_rust!(metis_c_align_MetisHostApiV1, align_of::<MetisHostApiV1>());
}

#[test]
fn c_and_rust_agree_on_every_field_offset() {
    c_agrees_with_rust!(metis_c_offset_MetisBuffer_ptr, offset_of!(MetisBuffer, ptr));
    c_agrees_with_rust!(metis_c_offset_MetisBuffer_len, offset_of!(MetisBuffer, len));
    c_agrees_with_rust!(metis_c_offset_MetisBuffer_cap, offset_of!(MetisBuffer, cap));
    c_agrees_with_rust!(
        metis_c_offset_MetisBuffer_owner,
        offset_of!(MetisBuffer, owner)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisBuffer_reserved,
        offset_of!(MetisBuffer, reserved)
    );

    c_agrees_with_rust!(
        metis_c_offset_MetisTablePrefix_size,
        offset_of!(MetisTablePrefix, size)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisTablePrefix_abi_major,
        offset_of!(MetisTablePrefix, abi_major)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisTablePrefix_abi_minor,
        offset_of!(MetisTablePrefix, abi_minor)
    );

    c_agrees_with_rust!(
        metis_c_offset_MetisPluginTableV1_size,
        offset_of!(MetisPluginTableV1, size)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisPluginTableV1_abi_major,
        offset_of!(MetisPluginTableV1, abi_major)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisPluginTableV1_abi_minor,
        offset_of!(MetisPluginTableV1, abi_minor)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisPluginTableV1_create_instance,
        offset_of!(MetisPluginTableV1, create_instance)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisPluginTableV1_destroy_instance,
        offset_of!(MetisPluginTableV1, destroy_instance)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisPluginTableV1_list_capabilities,
        offset_of!(MetisPluginTableV1, list_capabilities)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisPluginTableV1_invoke,
        offset_of!(MetisPluginTableV1, invoke)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisPluginTableV1_cancel,
        offset_of!(MetisPluginTableV1, cancel)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisPluginTableV1_release_buffer,
        offset_of!(MetisPluginTableV1, release_buffer)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisPluginTableV1_shutdown,
        offset_of!(MetisPluginTableV1, shutdown)
    );

    c_agrees_with_rust!(
        metis_c_offset_MetisHostApiV1_size,
        offset_of!(MetisHostApiV1, size)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisHostApiV1_user_data,
        offset_of!(MetisHostApiV1, user_data)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisHostApiV1_complete,
        offset_of!(MetisHostApiV1, complete)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisHostApiV1_progress,
        offset_of!(MetisHostApiV1, progress)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisHostApiV1_emit_event,
        offset_of!(MetisHostApiV1, emit_event)
    );
    c_agrees_with_rust!(
        metis_c_offset_MetisHostApiV1_log,
        offset_of!(MetisHostApiV1, log)
    );
}
