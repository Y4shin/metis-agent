//! The version gate and the presence rule, at the seam a loader reaches first.
//!
//! These tests build tables by hand rather than loading a library: the point is
//! what a reader does with a table that is the wrong version or too short, and
//! that is a property of the bytes in front of it. The fixture harness and the C
//! implementer cover the same gate through a real `cdylib`.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use plugin_contract::abi::*;

/// A table whose entries are all one harmless function, so a refusal can be
/// attributed to the gate rather than to a null entry.
unsafe extern "C" fn noop_init(
    _host: *const MetisHostApiV1,
    _payload: MetisBuffer,
    _out_instance: *mut *mut c_void,
    _out_future: *mut MetisFutureHandle,
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
        init_plugin: noop_init,
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
fn the_revision_reports_minor_two_and_keeps_the_major_and_symbol() {
    assert_eq!(METIS_ABI_MAJOR, 1);
    assert_eq!(METIS_ABI_MINOR, 2);
    assert_eq!(METIS_PLUGIN_ENTRY_V1, b"metis_plugin_entry_v1\0");
}

#[test]
fn a_current_table_is_accepted() {
    let table = full_table(size_of::<MetisPluginTableV1>(), 1, METIS_ABI_MINOR);
    // SAFETY: the table is a live local for the duration of the call.
    let read = unsafe { metis_read_table(&table) }.expect("a current table is accepted");
    assert_eq!(read.abi_minor, METIS_ABI_MINOR);
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
fn a_newer_minor_inside_the_major_is_tolerated() {
    // Additive tolerance: a plugin built later reports a higher minor, and its
    // extra entries are past the last entry this build reads.
    let newer = full_table(size_of::<MetisPluginTableV1>(), 1, METIS_ABI_MINOR + 1);
    // SAFETY: the table is a live local.
    let read = unsafe { metis_read_table(&newer) }.expect("a newer minor is tolerated");
    assert_eq!(read.abi_minor, METIS_ABI_MINOR + 1);
    assert_eq!(read.abi_major, METIS_ABI_MAJOR);
}

#[test]
fn a_retired_minor_is_refused_before_any_table_entry_is_read() {
    // The retired pre-release plugin: same symbol and major, entries in the
    // superseded order, so the minor is what refuses it. The table is also too
    // short to cover `service_complete`, which is what makes the error prove
    // the order of the two checks: the version is refused before the size is
    // consulted, so nothing in the table is read.
    let short = size_of::<MetisTablePrefix>() + 7 * size_of::<usize>();
    let stale = full_table(short, 1, 0);
    // SAFETY: the table is a live local.
    let error = refused(unsafe { metis_read_table(&stale) }); // minor 0 is the old entry order
    assert_eq!(
        error,
        MetisAbiError::MinorMismatch {
            known: METIS_ABI_MINOR,
            found: 0
        }
    );

    // The same short table at this build's own minor is the size error instead,
    // which is the other half of the same ordering.
    let short_at_this_minor = full_table(short, 1, METIS_ABI_MINOR);
    // SAFETY: the table is a live local.
    let error = refused(unsafe { metis_read_table(&short_at_this_minor) });
    assert_eq!(
        error,
        MetisAbiError::EntryMissing {
            entry: "service_complete"
        }
    );
}

/// Counts calls into the appended entry's canary, which must stay at zero.
static CANARY_CALLS: AtomicUsize = AtomicUsize::new(0);

/// The stand-in for the entry a later ticket appends after this build's last
/// one. Nothing in this build may call it.
unsafe extern "C" fn canary_configure(_payload: MetisBuffer) -> MetisStatus {
    CANARY_CALLS.fetch_add(1, Ordering::SeqCst);
    MetisStatus::Internal
}

/// The table a later ticket would publish: this build's eight entries plus the
/// entry appended after them.
///
/// It is `repr(C)` with the same prefix and the same first eight entries as
/// [`MetisPluginTableV1`], so a reader that receives a pointer to it reads
/// exactly what it would read from a real plugin, and `size` reports the longer
/// table the newer plugin provides.
#[repr(C)]
struct ExtendedTableV1 {
    size: usize,
    abi_major: u32,
    abi_minor: u32,
    init_plugin: MetisInitPluginFn,
    destroy_instance: MetisDestroyInstanceFn,
    list_capabilities: MetisListCapabilitiesFn,
    invoke_future: MetisInvokeFutureFn,
    cancel: MetisCancelFn,
    release_buffer: MetisReleaseBufferFn,
    shutdown: MetisShutdownFn,
    service_complete: MetisServiceCompleteFn,
    /// The appended entry, the entry this build knows nothing about.
    configure: unsafe extern "C" fn(payload: MetisBuffer) -> MetisStatus,
}

#[test]
fn an_appended_entry_and_a_newer_minor_still_interoperate() {
    // The host and the plugin agree on the major and differ in a table entry
    // added after this ticket lands: the plugin's table carries one more entry
    // and reports a size that covers it.
    let known = full_table(
        size_of::<MetisPluginTableV1>(),
        METIS_ABI_MAJOR,
        METIS_ABI_MINOR,
    );
    let extended = ExtendedTableV1 {
        size: size_of::<ExtendedTableV1>(),
        abi_major: METIS_ABI_MAJOR,
        abi_minor: METIS_ABI_MINOR + 1,
        init_plugin: known.init_plugin,
        destroy_instance: known.destroy_instance,
        list_capabilities: known.list_capabilities,
        invoke_future: known.invoke_future,
        cancel: known.cancel,
        release_buffer: known.release_buffer,
        shutdown: known.shutdown,
        service_complete: known.service_complete,
        configure: canary_configure,
    };

    // The canary is armed: calling it is what records a touch, so a reader that
    // reached the appended entry would move the counter.
    // SAFETY: the canary is an ordinary function.
    let _ = unsafe { canary_configure(MetisBuffer::empty()) };
    assert_eq!(
        CANARY_CALLS.swap(0, Ordering::SeqCst),
        1,
        "the canary records a call"
    );

    // SAFETY: the prefix and the first eight entries of `ExtendedTableV1` have
    // the layout of `MetisPluginTableV1`, and the appended entry is never read.
    let read = unsafe {
        metis_read_table(&extended as *const ExtendedTableV1 as *const MetisPluginTableV1)
    }
    .expect("a newer minor inside the major, and an appended entry, are tolerated");

    assert_eq!(read.abi_major, METIS_ABI_MAJOR);
    assert_eq!(
        read.abi_minor,
        METIS_ABI_MINOR + 1,
        "additive tolerance accepts the newer minor"
    );
    assert_eq!(
        read.size,
        size_of::<ExtendedTableV1>(),
        "the size a reader checks is the size the plugin reported"
    );
    assert!(
        read.size > size_of::<MetisPluginTableV1>(),
        "the newer plugin's table is longer than this build's layout"
    );

    // Every entry this build knows is the plugin's own value, read at its own
    // offset: the reader touches the entries its own layout covers and no
    // further.
    assert!(ptr::fn_addr_eq(read.init_plugin, known.init_plugin));
    assert!(ptr::fn_addr_eq(
        read.destroy_instance,
        known.destroy_instance
    ));
    assert!(ptr::fn_addr_eq(
        read.list_capabilities,
        known.list_capabilities
    ));
    assert!(ptr::fn_addr_eq(read.invoke_future, known.invoke_future));
    assert!(ptr::fn_addr_eq(read.cancel, known.cancel));
    assert!(ptr::fn_addr_eq(read.release_buffer, known.release_buffer));
    assert!(ptr::fn_addr_eq(read.shutdown, known.shutdown));
    assert!(ptr::fn_addr_eq(
        read.service_complete,
        known.service_complete
    ));

    assert_eq!(
        CANARY_CALLS.load(Ordering::SeqCst),
        0,
        "the entry appended after this build's layout was never touched"
    );
}

#[test]
fn a_table_too_short_for_an_entry_names_the_entry() {
    // A table that stops just after `shutdown`, which is the shape an older
    // revision would have had.
    let short = size_of::<MetisTablePrefix>() + 7 * size_of::<usize>();
    let table = full_table(short, 1, METIS_ABI_MINOR);
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
    let table = full_table(size_of::<MetisTablePrefix>(), 1, METIS_ABI_MINOR);
    // SAFETY: as above.
    let error = refused(unsafe { metis_read_table(&table) }); // no entry is covered
    assert_eq!(
        error,
        MetisAbiError::EntryMissing {
            entry: "init_plugin"
        }
    );
}

#[test]
fn presence_is_decided_by_size() {
    let full_plugin = size_of::<MetisPluginTableV1>();
    let full_host = size_of::<MetisHostApiV1>();
    assert!(metis_plugin_table_has_init_plugin(full_plugin));
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
    let table = full_table(size_of::<MetisPluginTableV1>(), 1, METIS_ABI_MINOR);
    // SAFETY: the table is a live local.
    let prefix = unsafe { metis_read_prefix(&table) }.expect("the prefix is covered");
    assert_eq!(prefix.abi_major, 1);
    assert_eq!(prefix.abi_minor, METIS_ABI_MINOR);

    let trimmed = size_of::<MetisTablePrefix>() - 1;
    let table = full_table(trimmed, 1, METIS_ABI_MINOR);
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

// --- the table's description, checked against the table -------------------

/// The repository root, which is where the contract text and the generated
/// header live.
fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("the repository root")
        .to_path_buf()
}

/// The eight entries of the plugin table, in slot order.
const TABLE_ENTRIES: [&str; 8] = [
    "init_plugin",
    "destroy_instance",
    "list_capabilities",
    "invoke_future",
    "cancel",
    "release_buffer",
    "shutdown",
    "service_complete",
];

/// The generated header, which is the artifact a non-Rust implementer uses.
fn generated_header() -> String {
    std::fs::read_to_string(
        repo_root().join("crates/plugin-contract/include/metis_plugin_abi.h"),
    )
    .expect("the generated header is committed")
}

/// The contract text's plugin-table section, which is the table's description.
fn contract_table_section() -> String {
    let contract = std::fs::read_to_string(repo_root().join("docs/plugin-contract.md"))
        .expect("the contract text is committed");
    contract
        .split("## 5. The plugin table")
        .nth(1)
        .and_then(|rest| rest.split("## 6.").next())
        .expect("the contract has a plugin table section")
        .to_string()
}

#[test]
fn the_contract_text_and_the_generated_header_agree_with_the_table() {
    let header = generated_header();
    let section = contract_table_section();

    // The header carries one slot constant per entry, at the index the layout
    // asserts, and the table is exactly those slots.
    assert_eq!(METIS_PLUGIN_TABLE_V1_SLOTS, TABLE_ENTRIES.len());
    assert_eq!(METIS_PLUGIN_TABLE_V1_SLOTS, 8);
    for (slot, entry) in TABLE_ENTRIES.iter().enumerate() {
        let define = format!(
            "#define METIS_PLUGIN_TABLE_V1_{}_SLOT {slot}",
            entry.to_uppercase()
        );
        assert!(
            header.contains(&define),
            "the generated header is missing `{define}`"
        );
        assert!(
            section.contains(&format!("`{entry}`")),
            "the contract's plugin table has no row for `{entry}`"
        );
    }
    assert!(
        !section.contains("create_instance"),
        "the contract still describes the instance-creation entry this revision replaced"
    );
    assert_eq!(
        size_of::<MetisPluginTableV1>(),
        size_of::<MetisTablePrefix>() + METIS_PLUGIN_TABLE_V1_SLOTS * size_of::<*const c_void>(),
        "the table is the prefix plus its pointer slots"
    );
}

#[test]
fn no_counter_entry_is_reachable_through_the_table() {
    let header = generated_header();
    let section = contract_table_section();

    // Eight entries and no ninth. The table's own struct body and the
    // description's rows are what the check reads: a slot constant or a member
    // named after a counter would be a slot the table does not have.
    assert_eq!(METIS_PLUGIN_TABLE_V1_SLOTS, 8);
    let body = header
        .split("typedef struct MetisPluginTableV1 {")
        .nth(1)
        .and_then(|rest| rest.split("} MetisPluginTableV1;").next())
        .expect("the header declares MetisPluginTableV1");
    assert!(
        !body.to_lowercase().contains("counter"),
        "the plugin table carries a counter member"
    );
    let slot_defines: String = header
        .lines()
        .filter(|line| line.starts_with("#define METIS_PLUGIN_TABLE_V1_"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !slot_defines.to_lowercase().contains("counter"),
        "the generated header names a counter slot"
    );
    let rows: String = section
        .lines()
        .filter(|line| line.trim_start().starts_with('|'))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !rows.to_lowercase().contains("counter"),
        "the contract's plugin table has a counter row"
    );
    // The first slot is the initialisation entry, and the replaced
    // instance-creation entry is gone from the header entirely.
    assert_eq!(METIS_PLUGIN_TABLE_V1_INIT_PLUGIN_SLOT, 0);
    assert!(!header.contains("create_instance"));
    assert!(header.contains("init_plugin"));
}
