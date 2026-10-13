//! The version gate and the presence rule, at the seam a loader reaches first.
//!
//! These tests build tables by hand rather than loading a library: the point is
//! what a reader does with a table that is the wrong version or too short, and
//! that is a property of the bytes in front of it. The fixture harness and the C
//! implementer cover the same gate through a real `cdylib`.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

use plugin_contract::abi::*;

/// A table whose entries are all one harmless function, so a refusal can be
/// attributed to the gate rather than to a null entry.
unsafe extern "C" fn noop_create(
    _host: *const MetisHostApiV1,
    _out: *mut *mut c_void,
) -> MetisStatus {
    MetisStatus::Internal
}

unsafe extern "C" fn noop_destroy(_instance: *mut c_void) -> MetisStatus {
    MetisStatus::Internal
}

unsafe extern "C" fn noop_list(_instance: *mut c_void, _out: *mut MetisBuffer) -> MetisStatus {
    MetisStatus::Internal
}

unsafe extern "C" fn noop_invoke(
    _instance: *mut c_void,
    _capability: *const u8,
    _len: usize,
    _request_id: u64,
    _payload: MetisBuffer,
    _host: *const MetisHostApiV1,
    _out: *mut MetisFutureHandle,
) -> MetisStatus {
    MetisStatus::Internal
}

unsafe extern "C" fn noop_cancel(_instance: *mut c_void, _request_id: u64) -> MetisStatus {
    MetisStatus::Internal
}

unsafe extern "C" fn noop_release(_buffer: MetisBuffer) -> MetisStatus {
    MetisStatus::Internal
}

unsafe extern "C" fn noop_shutdown(_instance: *mut c_void) -> MetisStatus {
    MetisStatus::Internal
}

unsafe extern "C" fn noop_service_complete(
    _instance: *mut c_void,
    _request_id: u64,
    _status: MetisStatus,
    _payload: MetisBuffer,
) -> MetisStatus {
    MetisStatus::Internal
}

/// Expects a refusal, without needing the accepted reference to be printable.
fn refused(result: Result<&MetisPluginTableV1, MetisAbiError>) -> MetisAbiError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("the table should have been refused"),
    }
}

/// The full table, which every mismatch test starts from.
fn full_table(size: usize, abi_major: u32, abi_minor: u32) -> MetisPluginTableV1 {
    MetisPluginTableV1 {
        size,
        abi_major,
        abi_minor,
        create_instance: noop_create,
        destroy_instance: noop_destroy,
        list_capabilities: noop_list,
        invoke_future: noop_invoke,
        cancel: noop_cancel,
        release_buffer: noop_release,
        shutdown: noop_shutdown,
        service_complete: noop_service_complete,
    }
}

#[test]
fn the_revision_reports_minor_one_and_keeps_the_major_and_symbol() {
    assert_eq!(METIS_ABI_MAJOR, 1);
    assert_eq!(METIS_ABI_MINOR, 1);
    assert_eq!(METIS_PLUGIN_ENTRY_V1, b"metis_plugin_entry_v1\0");
}

#[test]
fn a_current_table_is_accepted() {
    let table = full_table(size_of::<MetisPluginTableV1>(), 1, 1);
    // SAFETY: the table is a live local for the duration of the call.
    let read = unsafe { metis_read_table(&table) }.expect("a current table is accepted");
    assert_eq!(read.abi_minor, 1);
    assert_eq!(
        read.size,
        size_of::<MetisPluginTableV1>(),
        "the size a host checks is the size the plugin reported"
    );
}

#[test]
fn a_null_table_is_unreadable() {
    // SAFETY: a null pointer is exactly what this checks for.
    let error = refused(unsafe { metis_read_table(ptr::null()) });
    assert_eq!(error, MetisAbiError::Unreadable);
}

#[test]
fn another_major_is_refused_with_both_versions_named() {
    let table = full_table(size_of::<MetisPluginTableV1>(), 2, 0);
    // SAFETY: the table is a live local.
    let error = refused(unsafe { metis_read_table(&table) }); // major 2 is not this build
    assert_eq!(
        error,
        MetisAbiError::MajorMismatch {
            expected: 1,
            found: 2
        }
    );
}

#[test]
fn a_minor_this_build_does_not_know_is_refused() {
    // The stale pre-release plugin: same symbol and major, entries in the
    // superseded order, so the minor is what refuses it.
    let stale = full_table(size_of::<MetisPluginTableV1>(), 1, 0);
    // SAFETY: the table is a live local.
    let error = refused(unsafe { metis_read_table(&stale) }); // minor 0 is the old entry order
    assert_eq!(error, MetisAbiError::MinorMismatch { known: 1, found: 0 });

    let future = full_table(size_of::<MetisPluginTableV1>(), 1, 2);
    // SAFETY: the table is a live local.
    let error = refused(unsafe { metis_read_table(&future) }); // minor 2 is newer than this build
    assert_eq!(error, MetisAbiError::MinorMismatch { known: 1, found: 2 });
}

#[test]
fn a_table_too_short_for_an_entry_names_the_entry() {
    // A table that stops just after `shutdown`, which is the shape an older
    // revision would have had.
    let short = size_of::<MetisTablePrefix>() + 7 * size_of::<usize>();
    let table = full_table(short, 1, 1);
    // SAFETY: the checks read only the prefix, and the table is a live local
    // whose prefix is intact.
    let error = refused(unsafe { metis_read_table(&table) }); // service_complete is not covered
    assert_eq!(
        error,
        MetisAbiError::EntryMissing {
            entry: "service_complete"
        }
    );

    // A table that is the bare prefix still reads as a table, and names the
    // first entry it does not cover.
    let table = full_table(size_of::<MetisTablePrefix>(), 1, 1);
    // SAFETY: as above.
    let error = refused(unsafe { metis_read_table(&table) }); // no entry is covered
    assert_eq!(
        error,
        MetisAbiError::EntryMissing {
            entry: "create_instance"
        }
    );
}

#[test]
fn presence_is_decided_by_size() {
    let full_plugin = size_of::<MetisPluginTableV1>();
    let full_host = size_of::<MetisHostApiV1>();
    assert!(metis_plugin_table_has_invoke_future(full_plugin));
    assert!(metis_plugin_table_has_service_complete(full_plugin));
    assert!(metis_host_api_has_service_request(full_host));
    assert!(!metis_plugin_table_has_service_complete(
        METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_THRESHOLD - 1
    ));
    assert!(metis_plugin_table_has_service_complete(
        METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_THRESHOLD
    ));
    assert!(!metis_host_api_has_service_request(
        METIS_HOST_API_V1_SERVICE_REQUEST_THRESHOLD - 1
    ));
    assert!(metis_host_api_has_service_request(
        METIS_HOST_API_V1_SERVICE_REQUEST_THRESHOLD
    ));

    // The thresholds and the slots are the same numbers, which is what makes
    // the header's slot constants a usable C predicate.
    assert_eq!(
        METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_THRESHOLD,
        size_of::<MetisTablePrefix>()
            + (METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_SLOT + 1) * size_of::<usize>()
    );
    assert_eq!(METIS_PLUGIN_TABLE_V1_SLOTS, 8);
    assert_eq!(METIS_HOST_API_V1_SLOTS, 5);
}

#[test]
fn the_prefix_is_read_before_the_entries() {
    let table = full_table(size_of::<MetisPluginTableV1>(), 1, 1);
    // SAFETY: the table is a live local.
    let prefix = unsafe { metis_read_prefix(&table) }.expect("the prefix is covered");
    assert_eq!(prefix.abi_major, 1);
    assert_eq!(prefix.abi_minor, 1);

    let trimmed = size_of::<MetisTablePrefix>() - 1;
    let table = full_table(trimmed, 1, 1);
    // SAFETY: the size check is what this exercises.
    assert!(
        unsafe { metis_read_prefix(&table) }.is_none(),
        "a size that does not cover the prefix is refused before a field is read"
    );
}

#[test]
fn neither_vtable_carries_a_size_field() {
    // The revision deleted the two `size` fields rather than reading them. The
    // proof is the layout: each vtable is exactly its entries.
    assert_eq!(
        size_of::<MetisFutureVtable>(),
        2 * size_of::<*const c_void>()
    );
    assert_eq!(
        size_of::<MetisWakerVtable>(),
        4 * size_of::<*const c_void>()
    );
    assert!(MetisFutureHandle::null().is_null());
    assert!(MetisWaker::null().is_null());
}
