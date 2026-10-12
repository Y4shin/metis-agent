//! Cross-checks the C translation unit against the Rust layout (D13).
//!
//! `c/layout_static_asserts.c` is compiled by `build.rs` and contains literal
//! `_Static_assert`s. This test takes the values that C measured and compares
//! them with Rust's `size_of`, `align_of` and `offset_of`, so both sides of the
//! agreement are checked: the C literals, and the numbers the two compilers
//! actually produced. It then resolves a version 1 function table written in C
//! and drives it from Rust.

#![allow(non_upper_case_globals)]

use core::ffi::c_void;
use core::mem::{align_of, offset_of, size_of};

use spike_abi::*;

unsafe extern "C" {
    static SPIKE_size_MetisStatus: usize;
    static SPIKE_align_MetisStatus: usize;
    static SPIKE_size_MetisBuffer: usize;
    static SPIKE_align_MetisBuffer: usize;
    static SPIKE_offset_MetisBuffer_len: usize;
    static SPIKE_offset_MetisBuffer_owner: usize;
    static SPIKE_size_MetisTablePrefix: usize;
    static SPIKE_size_MetisPluginTableV1: usize;
    static SPIKE_align_MetisPluginTableV1: usize;
    static SPIKE_offset_MetisPluginTableV1_create_instance: usize;
    static SPIKE_offset_MetisPluginTableV1_invoke: usize;
    static SPIKE_offset_MetisPluginTableV1_shutdown: usize;
    static SPIKE_size_MetisHostApiV1: usize;
    static SPIKE_align_MetisHostApiV1: usize;
    static SPIKE_offset_MetisHostApiV1_complete: usize;
    static SPIKE_offset_MetisHostApiV1_log: usize;

    fn spike_c_entry_v1() -> *const MetisPluginTableV1;
}

#[test]
fn c_and_rust_agree_on_sizes_and_offsets() {
    // SAFETY: reading immutable globals defined by the C translation unit that
    // build.rs compiled from c/layout_static_asserts.c.
    unsafe {
        assert_eq!(SPIKE_size_MetisStatus, size_of::<MetisStatus>(), "MetisStatus size");
        assert_eq!(SPIKE_align_MetisStatus, align_of::<MetisStatus>(), "MetisStatus alignment");

        assert_eq!(SPIKE_size_MetisBuffer, size_of::<MetisBuffer>(), "MetisBuffer size");
        assert_eq!(SPIKE_align_MetisBuffer, align_of::<MetisBuffer>(), "MetisBuffer alignment");
        assert_eq!(SPIKE_offset_MetisBuffer_len, offset_of!(MetisBuffer, len), "MetisBuffer.len");
        assert_eq!(SPIKE_offset_MetisBuffer_owner, offset_of!(MetisBuffer, owner), "MetisBuffer.owner");

        assert_eq!(SPIKE_size_MetisTablePrefix, size_of::<MetisTablePrefix>(), "MetisTablePrefix size");

        assert_eq!(
            SPIKE_size_MetisPluginTableV1,
            size_of::<MetisPluginTableV1>(),
            "MetisPluginTableV1 size"
        );
        assert_eq!(
            SPIKE_align_MetisPluginTableV1,
            align_of::<MetisPluginTableV1>(),
            "MetisPluginTableV1 alignment"
        );
        assert_eq!(
            SPIKE_offset_MetisPluginTableV1_create_instance,
            offset_of!(MetisPluginTableV1, create_instance),
            "MetisPluginTableV1.create_instance"
        );
        assert_eq!(
            SPIKE_offset_MetisPluginTableV1_invoke,
            offset_of!(MetisPluginTableV1, invoke),
            "MetisPluginTableV1.invoke"
        );
        assert_eq!(
            SPIKE_offset_MetisPluginTableV1_shutdown,
            offset_of!(MetisPluginTableV1, shutdown),
            "MetisPluginTableV1.shutdown"
        );

        assert_eq!(SPIKE_size_MetisHostApiV1, size_of::<MetisHostApiV1>(), "MetisHostApiV1 size");
        assert_eq!(SPIKE_align_MetisHostApiV1, align_of::<MetisHostApiV1>(), "MetisHostApiV1 alignment");
        assert_eq!(
            SPIKE_offset_MetisHostApiV1_complete,
            offset_of!(MetisHostApiV1, complete),
            "MetisHostApiV1.complete"
        );
        assert_eq!(SPIKE_offset_MetisHostApiV1_log, offset_of!(MetisHostApiV1, log), "MetisHostApiV1.log");
    }
}

#[test]
fn rust_reads_a_c_authored_function_table() {
    // SAFETY: spike_c_entry_v1 returns a pointer to a static table inside the C
    // translation unit; it never becomes dangling.
    unsafe {
        let table = spike_c_entry_v1();
        assert!(!table.is_null(), "C entry point returned null");

        let prefix = metis_read_prefix(table).expect("C table prefix must be readable");
        assert_eq!(prefix.size, size_of::<MetisPluginTableV1>(), "C table size field");
        assert_eq!(prefix.abi_major, METIS_ABI_MAJOR, "C table major version");

        let mut instance: *mut c_void = std::ptr::null_mut();
        let status = ((*table).create_instance)(std::ptr::null(), &mut instance);
        assert_eq!(status, MetisStatus::Ok, "C create_instance status");
        assert_eq!(instance as usize, 1, "C create_instance wrote through the out parameter");
        assert_eq!(((*table).destroy_instance)(instance), MetisStatus::Ok, "C destroy_instance status");
    }
}
