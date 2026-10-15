//! The ABI harness: it loads the fixture cdylib and drives the revised v1 table
//! through `plugin-contract`.
//!
//! Run it with:
//!
//! ```sh
//! devenv shell -- cargo run -p plugin-fixture --bin abi-harness
//! ```
//!
//! The fixture library is rebuilt before it is loaded. `cargo run -p` does not
//! rebuild a sibling cdylib target, so a harness that skipped that step would
//! load a stale `.so` and report results belonging to an older source tree;
//! this is the spike's stale-library lesson encoded rather than documented.
//!
//! Output is one `[ok]` or `[fail]` line per check, then a `[result] PASS` or
//! `[result] FAIL` line, and the process exits non-zero when anything failed.
//! Every check prints as it runs, so a hang names the step that did not finish.
//! The teardown ordering prints as it happens, because the ordering is itself a
//! check.
//!
//! The harness is a real host: it loads the real library, allocates its own
//! request buffers, releases every plugin buffer it receives through the
//! plugin's own `release_buffer`, polls the plugin's futures on threads it
//! owns, and compares its counters with the fixture's ledger. Nothing here
//! reimplements the ABI.
//!
//! It also carries the version tolerance checks, which are about compiled
//! versions rather than about one table: a table with an entry appended after
//! this build's last one (the reader stops at its own layout), the retired
//! pre-release minor refused before any entry is read, a library built for
//! another ABI major refused at symbol resolution with a typed error, one
//! capability refused by descriptor version while the plugin keeps serving, and
//! a request body carrying a field this build does not know. On Linux the
//! reader's restraint is asserted by memory that cannot be read: the guarded
//! table probes below.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::slice;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use libloading::Library;
use plugin_contract::abi::*;
use plugin_contract::payload::{metis_encode_payload, metis_payload_body};
use plugin_fixture::payload::{
    CapabilityDescriptor, CapabilityKind, CapabilityList, EchoRequest, EchoResponse,
};

/// How long the harness waits for a call that should finish promptly.
const POLL_DEADLINE: Duration = Duration::from_secs(2);

/// How long the watchdog waits before it calls a poll abandoned. The fixture's
/// blocking capability blocks for longer than this on purpose.
const WATCHDOG: Duration = Duration::from_millis(80);

/// How long the harness waits for a cancel to be honoured before it records a
/// missed deadline. The stubborn capability never honours one.
const CANCEL_WINDOW: Duration = Duration::from_millis(150);

/// The capabilities the fixture advertises, in declaration order.
const EXPECTED_CAPABILITIES: [&str; 10] = [
    "tool.echo",
    "tool.pending",
    "tool.cancellable",
    "tool.stubborn",
    "tool.panic",
    "tool.panic_drop",
    "tool.blocking",
    "tool.deferred",
    "tool.wake_deferred",
    REFUSED_CAPABILITY,
];

/// The capability the fixture advertises with a descriptor version this host
/// does not support. It is one capability, so refusing it must leave the rest
/// of the plugin listed and serving.
const REFUSED_CAPABILITY: &str = "tool.newer";

/// The descriptor version [`REFUSED_CAPABILITY`] declares.
const REFUSED_CAPABILITY_VERSION: u32 = 2;

/// The highest capability descriptor version this host serves.
const HOST_CAPABILITY_VERSION: u32 = 1;

/// The capabilities the routing check drives to completion by name alone.
/// `tool.cancellable` needs a cancel, and the hazard capabilities need their own
/// handling, so each of them is driven by its own check below.
const ROUTED_CAPABILITIES: [&str; 3] = ["tool.echo", "tool.pending", "tool.stubborn"];

fn main() -> ExitCode {
    install_panic_hook();

    let mut checks = Checks::new();

    if !rebuild_fixture(&mut checks) {
        return checks.report();
    }

    let path = fixture_library_path();
    if !checks.check(path.exists(), "fixture.library_present", path.display()) {
        return checks.report();
    }

    // SAFETY: the harness controls the library it just built. It stays loaded
    // until after every future and waker clone has been released, so no plugin
    // code can run inside an unloaded library.
    let library = match unsafe { Library::new(&path) } {
        Ok(library) => library,
        Err(err) => {
            checks.check(
                false,
                "fixture.library_loaded",
                format!("dlopen failed: {err}"),
            );
            return checks.report();
        }
    };
    checks.check(true, "fixture.library_loaded", path.display());

    // --- the entry symbol --------------------------------------------------

    // The library's exported `metis_*` symbols are read before the entry symbol
    // is resolved, because the resolution error names the major a library
    // exports instead of this build's symbol.
    let symbols = metis_dynamic_symbols(&path);
    let exported = symbols.clone().unwrap_or_default();

    let table_ptr = match resolve_entry(&library, &exported) {
        Ok(entry) => unsafe { entry() },
        Err(error) => {
            checks.check(false, "symbol.entry_v1_resolved", error.to_string());
            return checks.report();
        }
    };
    if !checks.check(
        !table_ptr.is_null(),
        "symbol.entry_v1_resolved",
        "metis_plugin_entry_v1 returned a table",
    ) {
        return checks.report();
    }

    match symbols {
        Ok(symbols) => {
            checks.check(
                symbols == vec!["metis_plugin_entry_v1".to_string()],
                "symbol.exactly_one_metis_symbol",
                format!("nm -D --defined-only reports {symbols:?}"),
            );
        }
        Err(err) => {
            checks.check(false, "symbol.exactly_one_metis_symbol", err);
        }
    }

    // --- the version gate and the presence rule ----------------------------

    match unsafe { metis_read_table(table_ptr) } {
        Ok(read) => {
            checks.check(
                read.size == mem::size_of::<MetisPluginTableV1>(),
                "table.size_matches_layout",
                format!(
                    "plugin reports size={} this build's layout is {}",
                    read.size,
                    mem::size_of::<MetisPluginTableV1>()
                ),
            );
            checks.check(
                read.abi_major == METIS_ABI_MAJOR && read.abi_minor == METIS_ABI_MINOR,
                "table.version_is_1_1",
                format!(
                    "abi {}.{} (expected {}.{})",
                    read.abi_major, read.abi_minor, METIS_ABI_MAJOR, METIS_ABI_MINOR
                ),
            );
        }
        Err(err) => {
            checks.check(false, "table.size_matches_layout", err);
            checks.check(false, "table.version_is_1_1", err);
        }
    }

    // The stale pre-release plugin: the same symbol and major, the superseded
    // entry order, and a minor this build does not know.
    let mut probe = unsafe { std::ptr::read(table_ptr) };
    probe.abi_minor = 0;
    checks.check(
        matches!(
            unsafe { metis_read_table(&probe) },
            Err(MetisAbiError::MinorMismatch { known: 1, found: 0 })
        ),
        "table.stale_minor_refused",
        "a plugin reporting minor 0 is refused before its entries are read",
    );
    probe.abi_minor = METIS_ABI_MINOR;
    probe.abi_major = 2;
    checks.check(
        matches!(
            unsafe { metis_read_table(&probe) },
            Err(MetisAbiError::MajorMismatch {
                expected: 1,
                found: 2
            })
        ),
        "table.other_major_refused",
        "a plugin reporting major 2 is refused with both versions named",
    );
    let short_size = mem::size_of::<MetisTablePrefix>() + 7 * mem::size_of::<usize>();
    probe.abi_major = METIS_ABI_MAJOR;
    probe.size = short_size;
    checks.check(
        matches!(
            unsafe { metis_read_table(&probe) },
            Err(MetisAbiError::EntryMissing {
                entry: "service_complete"
            })
        ),
        "table.short_table_entry_named",
        format!("a table of {short_size} bytes does not cover service_complete"),
    );
    checks.check(
        !metis_plugin_table_has_service_complete(short_size)
            && metis_plugin_table_has_service_complete(mem::size_of::<MetisPluginTableV1>()),
        "table.presence_is_size",
        "the presence predicate follows the reported size",
    );
    check_guarded_tables(&mut checks, table_ptr);

    // --- the instance, the declaration, and the futures path ---------------

    let state = HostState::new(table_ptr);
    state.order("library loaded");

    let mut instance: *mut c_void = std::ptr::null_mut();
    let create_status = unsafe { ((*table_ptr).create_instance)(state.api(), &mut instance) };
    state.instance.store(instance, SeqCst);
    checks.check(
        create_status == MetisStatus::Ok && !instance.is_null(),
        "instance.create_ok",
        format!("status={} instance={instance:p}", create_status.as_str()),
    );
    if create_status != MetisStatus::Ok || instance.is_null() {
        return checks.report();
    }
    checks.check(
        state.saw_log("instance created"),
        "instance.started_ran",
        "the lifecycle hook ran and the adapter logged it",
    );
    checks.check(
        state.saw_event("fixture.ready"),
        "instance.start_event",
        "the lifecycle hook reached the host event route",
    );

    let list = list_capabilities(&mut checks, &state, instance);
    drive_capability_gate(&mut checks, &state, instance, list.as_ref());
    drive_capabilities(&mut checks, &state, instance, list.as_ref());
    drive_unknown_field(&mut checks, &state, instance);
    run_hazard_checks(&mut checks, &state, instance);

    drive_c_plugin(&mut checks);
    drive_other_major_plugin(&mut checks);

    // --- teardown: quiesce, drop, then unload ------------------------------

    checks.check(
        state.in_flight_is_empty(),
        "teardown.no_future_in_flight",
        format!("{} futures still held by the host", state.in_flight().len()),
    );
    let shutdown = unsafe { ((*table_ptr).shutdown)(instance) };
    state.order("shutdown after every future was dropped");
    checks.check(
        shutdown == MetisStatus::Ok,
        "teardown.shutdown_ok",
        format!("status={}", shutdown.as_str()),
    );
    match state.fixture_ledger() {
        Some(ledger) => {
            let balanced =
                ledger.get("allocs") == ledger.get("frees") && ledger.get("live") == Some(&0);
            checks.check(
                balanced,
                "ownership.fixture_ledger_balanced",
                format!("{ledger:?}"),
            );
            // Exactly one violation is expected, and it is the deliberate
            // second drop of one future: the ledger has to record it rather
            // than become wrong.
            checks.check(
                ledger.get("violations") == Some(&1),
                "ownership.only_the_deliberate_violation",
                format!("{ledger:?}"),
            );
            checks.check(
                ledger.get("frees").copied() == Some(state.released()),
                "ownership.host_released_every_buffer",
                format!(
                    "the host released {} buffers and the fixture freed {}",
                    state.released(),
                    ledger.get("frees").copied().unwrap_or_default()
                ),
            );
        }
        None => {
            checks.check(
                false,
                "ownership.fixture_ledger_balanced",
                "the fixture reported no ledger",
            );
        }
    }
    checks.check(
        state.waker_clones() == state.waker_drops(),
        "waker.every_clone_released",
        format!(
            "{} clones, {} releases",
            state.waker_clones(),
            state.waker_drops()
        ),
    );

    let destroy = unsafe { ((*table_ptr).destroy_instance)(instance) };
    checks.check(
        destroy == MetisStatus::Ok,
        "teardown.destroy_ok",
        format!("status={}", destroy.as_str()),
    );
    // The library stays resident until here: nothing may be inside plugin code
    // when it is unloaded, and the ordering ledger above records that.
    drop(library);
    state.order("library unloaded after destroy");
    let order = state.ordering();
    let first_unload = order
        .iter()
        .position(|event| event.starts_with("library unloaded"));
    let shutdown_at = order
        .iter()
        .position(|event| event.starts_with("shutdown after"));
    checks.check(
        matches!((shutdown_at, first_unload), (Some(s), Some(u)) if s < u),
        "ordering.quiesce_drop_unload",
        "quiescence is recorded before the unload",
    );

    checks.report()
}

// --- checks ----------------------------------------------------------------

/// Records checks and prints one line each as they happen.
struct Checks {
    passed: usize,
    failed: usize,
}

impl Checks {
    fn new() -> Self {
        Checks {
            passed: 0,
            failed: 0,
        }
    }

    fn check(&mut self, ok: bool, name: &str, note: impl std::fmt::Display) -> bool {
        if ok {
            self.passed += 1;
            println!("[ok] {name}: {note}");
        } else {
            self.failed += 1;
            println!("[fail] {name}: {note}");
        }
        ok
    }

    fn report(&self) -> ExitCode {
        let total = self.passed + self.failed;
        if self.failed == 0 {
            println!("[result] PASS ({total} checks)");
            ExitCode::SUCCESS
        } else {
            println!("[result] FAIL ({} of {total} checks failed)", self.failed);
            ExitCode::FAILURE
        }
    }
}

/// Replaces the default panic hook with a one-line note.
///
/// The harness panics on purpose inside one host callback, and the fixture
/// panics on purpose inside one exported adapter. Both are the designed
/// behaviour, so the default hook's backtrace notice is noise; the point of the
/// line is that a panic reached the boundary and was caught there.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let message = match info.payload().downcast_ref::<&str>() {
            Some(message) => (*message).to_string(),
            None => match info.payload().downcast_ref::<String>() {
                Some(message) => message.clone(),
                None => "<non-string panic payload>".to_string(),
            },
        };
        println!("[info] a deliberate panic was caught at the boundary: {message}");
    }));
}

/// Rebuilds the fixture cdylib and reports whether it succeeded.
///
/// Nothing is loaded when this fails: the harness must never fall back to a
/// library an earlier run left behind.
fn rebuild_fixture(checks: &mut Checks) -> bool {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let output = Command::new(env!("CARGO"))
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--lib")
        .output();
    match output {
        Ok(output) if output.status.success() => checks.check(
            true,
            "fixture.rebuilt_before_load",
            "cargo build --lib succeeded before the library was loaded",
        ),
        Ok(output) => {
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            checks.check(
                false,
                "fixture.rebuilt_before_load",
                format!(
                    "cargo build --lib exited {} (its stderr is above)",
                    output.status
                ),
            )
        }
        Err(err) => checks.check(
            false,
            "fixture.rebuilt_before_load",
            format!("cannot run cargo: {err}"),
        ),
    }
}

/// The library the harness loads: the fixture built beside the harness binary.
fn fixture_library_path() -> PathBuf {
    let exe = std::env::current_exe().expect("the harness knows where it is");
    let directory = exe
        .parent()
        .expect("the harness binary has a parent directory");
    let name = if cfg!(target_os = "macos") {
        "libplugin_fixture.dylib"
    } else if cfg!(target_os = "windows") {
        "plugin_fixture.dll"
    } else {
        "libplugin_fixture.so"
    };
    directory.join(name)
}

/// Why this build's entry symbol did not resolve from a library.
///
/// The loader's own error names the byte string it searched for, which is a fact
/// about the loader and not about the plugin. A host reports what it learned
/// about the plugin instead: the symbol name carries the ABI major, so a library
/// that exports only another major's entry symbol is a plugin built for that
/// major. `plugin-runtime` inherits this shape when it owns library loading;
/// until then the probe lives here.
#[derive(Debug)]
enum EntryError {
    /// The library exports no entry symbol for this build's major, and it does
    /// export one for another major.
    MajorMismatch {
        /// The major this build speaks.
        expected: u32,
        /// The major the library exports an entry symbol for.
        found: u32,
    },
    /// The library exports no `metis_plugin_entry_v*` symbol at all, so it is
    /// not a plugin this host can load. The loader's own message is the detail.
    NoEntrySymbol {
        /// The major this build speaks.
        expected: u32,
        /// What the loader said, kept as detail rather than as the error.
        detail: String,
    },
}

impl std::fmt::Display for EntryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntryError::MajorMismatch { expected, found } => write!(
                f,
                "the plugin exports the entry symbol for ABI major {found}, and this build speaks major {expected}"
            ),
            EntryError::NoEntrySymbol { expected, detail } => write!(
                f,
                "the plugin exports no entry symbol for ABI major {expected}: {detail}"
            ),
        }
    }
}

/// The ABI majors a library exports an entry symbol for.
fn exported_entry_majors(exported: &[String]) -> Vec<u32> {
    exported
        .iter()
        .filter_map(|name| name.strip_prefix("metis_plugin_entry_v"))
        .filter_map(|major| major.parse().ok())
        .collect()
}

/// Resolves this build's entry symbol, reporting a typed refusal.
///
/// This is the resolution half of the major check, and it runs before any table
/// exists: a plugin built for another major exports `metis_plugin_entry_v<its
/// major>` and not this build's symbol, so no table pointer is ever obtained and
/// nothing in a table is read.
fn resolve_entry(
    library: &Library,
    exported: &[String],
) -> Result<MetisPluginEntryV1Fn, EntryError> {
    // SAFETY: the caller loaded this library and keeps it alive for the call.
    match unsafe { library.get::<MetisPluginEntryV1Fn>(METIS_PLUGIN_ENTRY_V1) } {
        Ok(entry) => Ok(*entry),
        Err(err) => {
            let other = exported_entry_majors(exported)
                .into_iter()
                .find(|major| *major != METIS_ABI_MAJOR);
            Err(match other {
                Some(found) => EntryError::MajorMismatch {
                    expected: METIS_ABI_MAJOR,
                    found,
                },
                None => EntryError::NoEntrySymbol {
                    expected: METIS_ABI_MAJOR,
                    detail: err.to_string(),
                },
            })
        }
    }
}

/// A table whose bytes past the part this build may read sit in a page with no
/// access at all.
///
/// This is the canary the prefix seam asks for. A reader that touches an entry
/// past the size it was allowed does not get a wrong value out of a copy: it
/// faults, and the harness stops with the fault, so "the reader touches only the
/// entries its own size covers" is asserted by the memory rather than by a
/// comment. Linux only, because the guard is `mmap` and a page: a platform
/// without it prints a note and skips these three checks.
#[cfg(target_os = "linux")]
struct GuardedTable {
    base: *mut u8,
    table: *mut u8,
}

#[cfg(target_os = "linux")]
impl GuardedTable {
    /// Places `readable` so that its last byte is the last byte of a readable
    /// page. Every byte after it is in a page that cannot be read.
    fn new(readable: &[u8]) -> GuardedTable {
        // SAFETY: the two anonymous pages belong to this guard and are released
        // on drop, and the copy covers only bytes inside the first page.
        unsafe {
            let base = mmap(
                std::ptr::null_mut(),
                2 * PAGE,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            );
            assert!(base != MAP_FAILED, "mmap failed for the guarded table");
            assert_eq!(
                mprotect(base.cast::<u8>().add(PAGE).cast(), PAGE, PROT_NONE),
                0,
                "mprotect failed for the guarded table"
            );
            let table = base.cast::<u8>().add(PAGE - readable.len());
            assert_eq!(
                table as usize % mem::align_of::<usize>(),
                0,
                "the guarded table must be aligned like the real one"
            );
            std::ptr::copy_nonoverlapping(readable.as_ptr(), table, readable.len());
            GuardedTable {
                base: base.cast::<u8>(),
                table,
            }
        }
    }

    fn ptr(&self) -> *const MetisPluginTableV1 {
        self.table.cast::<MetisPluginTableV1>()
    }
}

#[cfg(target_os = "linux")]
impl Drop for GuardedTable {
    fn drop(&mut self) {
        // SAFETY: the region is the one this guard mapped.
        unsafe {
            munmap(self.base.cast(), 2 * PAGE);
        }
    }
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: i64,
    ) -> *mut c_void;
    fn mprotect(addr: *mut c_void, len: usize, prot: i32) -> i32;
    fn munmap(addr: *mut c_void, len: usize) -> i32;
}

/// One readable page and one page with no access. The kernel's page size on the
/// targets this probe runs on.
#[cfg(target_os = "linux")]
const PAGE: usize = 4096;
#[cfg(target_os = "linux")]
const PROT_NONE: i32 = 0;
#[cfg(target_os = "linux")]
const PROT_READ: i32 = 1;
#[cfg(target_os = "linux")]
const PROT_WRITE: i32 = 2;
#[cfg(target_os = "linux")]
const MAP_PRIVATE: i32 = 2;
#[cfg(target_os = "linux")]
const MAP_ANONYMOUS: i32 = 0x20;
/// `MAP_FAILED` is `(void *)-1`.
#[cfg(target_os = "linux")]
const MAP_FAILED: *mut c_void = std::ptr::without_provenance_mut(usize::MAX);

/// The bytes of this build's table, copied from the library's real one, with the
/// prefix patched to what another plugin would report and cut to `readable`
/// bytes.
///
/// A newer plugin reports a longer table and a newer minor; the retired
/// pre-release plugin reported the seven-entry size and minor 0.
#[cfg(target_os = "linux")]
fn patched_table_bytes(
    table_ptr: *const MetisPluginTableV1,
    size: usize,
    minor: u32,
    readable: usize,
) -> Vec<u8> {
    // SAFETY: the harness has already read this table, and only bytes inside the
    // table's own layout are copied.
    let mut bytes = unsafe {
        slice::from_raw_parts(table_ptr.cast::<u8>(), mem::size_of::<MetisPluginTableV1>())
    }
    .to_vec();
    bytes[..mem::size_of::<usize>()].copy_from_slice(&size.to_ne_bytes());
    let minor_at = mem::size_of::<usize>() + mem::size_of::<u32>();
    bytes[minor_at..minor_at + mem::size_of::<u32>()].copy_from_slice(&minor.to_ne_bytes());
    bytes.truncate(readable);
    bytes
}

/// The prefix seam, with the memory as the canary.
///
/// Three tables are placed so that the first byte the reader is not allowed to
/// touch is in a page that cannot be read:
///
/// - a newer plugin's table, whose reported size covers an appended entry. The
///   reader must answer with this build's entries and stop; reaching the
///   appended entry faults.
/// - a table whose reported size does not cover `service_complete`, which is the
///   entry the reader must refuse by name without reading it.
/// - the retired pre-release table, whose every entry is unreachable: the
///   version gate must refuse it before anything in it is read.
#[cfg(target_os = "linux")]
fn check_guarded_tables(checks: &mut Checks, table_ptr: *const MetisPluginTableV1) {
    let layout = mem::size_of::<MetisPluginTableV1>();
    let slot = mem::size_of::<*const c_void>();
    let prefix = mem::size_of::<MetisTablePrefix>();
    // SAFETY: the harness read this table already; it is the library's live one.
    let real = unsafe { &*table_ptr };

    // A newer plugin: its table is this build's layout plus the entry appended
    // after it, and the appended entry is in the page with no access.
    let longer = layout + slot;
    let guarded = GuardedTable::new(&patched_table_bytes(
        table_ptr,
        longer,
        METIS_ABI_MINOR + 1,
        layout,
    ));
    match unsafe { metis_read_table(guarded.ptr()) } {
        Ok(read) => {
            checks.check(
                read.size == longer
                    && read.abi_minor == METIS_ABI_MINOR + 1
                    && std::ptr::fn_addr_eq(read.service_complete, real.service_complete),
                "table.appended_entry_not_read",
                format!(
                    "the reader took this build's {layout} bytes of the {longer}-byte table it was shown and read no further"
                ),
            );
        }
        Err(err) => {
            checks.check(false, "table.appended_entry_not_read", err.to_string());
        }
    }

    // A table whose reported size stops before `service_complete`: the reader
    // has to name the entry it cannot reach, and it cannot read it either.
    let short = prefix + 7 * slot;
    let guarded = GuardedTable::new(&patched_table_bytes(
        table_ptr,
        short,
        METIS_ABI_MINOR,
        short,
    ));
    match unsafe { metis_read_table(guarded.ptr()) } {
        Err(MetisAbiError::EntryMissing { entry }) => {
            checks.check(
                entry == "service_complete",
                "table.uncovered_entry_not_read",
                format!("a {short}-byte table does not cover {entry}, and {entry} was never read"),
            );
        }
        Ok(_) => {
            checks.check(
                false,
                "table.uncovered_entry_not_read",
                "a table that stops before service_complete was accepted",
            );
        }
        Err(err) => {
            checks.check(false, "table.uncovered_entry_not_read", err.to_string());
        }
    }

    // The retired pre-release table: minor 0 with the superseded seven-entry
    // size, and every entry in the page with no access. Only the version may
    // refuse it, and the refusal has to come before any entry is read.
    let stale = prefix + 7 * slot;
    let guarded = GuardedTable::new(&patched_table_bytes(table_ptr, stale, 0, prefix));
    match unsafe { metis_read_table(guarded.ptr()) } {
        Err(MetisAbiError::MinorMismatch { known, found }) => {
            checks.check(
                known == METIS_ABI_MINOR && found == 0,
                "table.stale_minor_entries_not_read",
                "a table reporting minor 0 and the superseded size is refused on the version alone",
            );
        }
        Ok(_) => {
            checks.check(
                false,
                "table.stale_minor_entries_not_read",
                "a table reporting minor 0 was accepted",
            );
        }
        Err(err) => {
            checks.check(false, "table.stale_minor_entries_not_read", err.to_string());
        }
    }
}

/// The platforms without a page mapping run no guarded probe.
#[cfg(not(target_os = "linux"))]
fn check_guarded_tables(_checks: &mut Checks, _table_ptr: *const MetisPluginTableV1) {
    println!(
        "[order] the guarded-table probes are skipped: this platform has no page mapping here"
    );
}

/// The `metis_*` symbols the library defines dynamically (seam: entry symbol).
fn metis_dynamic_symbols(path: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("nm")
        .args(["-D", "--defined-only"])
        .arg(path)
        .output()
        .map_err(|err| format!("nm could not be run ({err}); the harness needs binutils on PATH, which the devenv shell provides"))?;
    if !output.status.success() {
        return Err(format!("nm -D --defined-only exited {}", output.status));
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    Ok(listing
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .filter(|name| name.starts_with("metis"))
        .map(str::to_string)
        .collect())
}

/// Lists the capabilities the fixture declares and checks the declaration.
fn list_capabilities(
    checks: &mut Checks,
    state: &HostState,
    instance: *mut c_void,
) -> Option<CapabilityList> {
    let mut payload = MetisBuffer::empty();
    // SAFETY: the instance is live and the out parameter is a live local.
    let status = unsafe { ((*state.table).list_capabilities)(instance, &mut payload) };
    if status != MetisStatus::Ok {
        checks.check(
            false,
            "capabilities.listed",
            format!("list_capabilities returned {}", status.as_str()),
        );
        return None;
    }
    let bytes = state.take_plugin_buffer(payload);
    let list: CapabilityList = match metis_payload_body(&bytes)
        .ok()
        .and_then(|body| serde_json::from_slice(body).ok())
    {
        Some(list) => list,
        None => {
            checks.check(false, "capabilities.listed", "the list is not a payload");
            return None;
        }
    };
    let names: Vec<&str> = list
        .capabilities
        .iter()
        .map(|descriptor| descriptor.name.as_str())
        .collect();
    checks.check(
        names == EXPECTED_CAPABILITIES.to_vec(),
        "capabilities.listed",
        format!("{names:?}"),
    );
    checks.check(
        list.capabilities.len() > 4,
        "capabilities.more_than_four",
        format!(
            "{} capabilities, which the four-arm vocabulary could not name",
            names.len()
        ),
    );
    let inline = list.capabilities.iter().all(|descriptor| {
        descriptor.kind == CapabilityKind::Tool.as_str()
            && descriptor.request_schema.as_object().is_some()
            && descriptor.response_schema.as_object().is_some()
    });
    checks.check(
        inline,
        "capabilities.inline_schemas",
        "every descriptor carries both schemas inline",
    );
    Some(list)
}

/// The per-capability version gate, with the plugin kept whole.
///
/// A capability whose descriptor version this host does not support is refused
/// with `Unsupported` for that name alone: the plugin is loaded, its other
/// capabilities stay listed, and they still serve. Nothing here fails the
/// plugin, and the check that the refused capability itself serves when it is
/// called directly is what proves the refusal is the host's gate rather than a
/// broken plugin.
fn drive_capability_gate(
    checks: &mut Checks,
    state: &HostState,
    instance: *mut c_void,
    list: Option<&CapabilityList>,
) {
    let Some(list) = list else {
        checks.check(
            false,
            "gate.version_refused",
            "the declaration was not listed, so no capability could be gated",
        );
        return;
    };
    let Some(descriptor) = list
        .capabilities
        .iter()
        .find(|descriptor| descriptor.name == REFUSED_CAPABILITY)
    else {
        checks.check(
            false,
            "gate.version_refused",
            format!("no capability named {REFUSED_CAPABILITY} was declared"),
        );
        return;
    };

    // The declared versions: one capability above the version this host serves,
    // every other one at or below it.
    let versions: Vec<(&str, u32)> = list
        .capabilities
        .iter()
        .map(|descriptor| (descriptor.name.as_str(), descriptor.version))
        .collect();
    checks.check(
        descriptor.version == REFUSED_CAPABILITY_VERSION
            && list.capabilities.iter().all(|descriptor| {
                descriptor.name == REFUSED_CAPABILITY
                    || descriptor.version <= HOST_CAPABILITY_VERSION
            }),
        "gate.declared_versions",
        format!("host serves up to {HOST_CAPABILITY_VERSION}: {versions:?}"),
    );

    // The gate, applied to every capability by name.
    let verdicts: Vec<(&str, Result<(), MetisStatus>)> = list
        .capabilities
        .iter()
        .map(capability_gate)
        .zip(list.capabilities.iter())
        .map(|(verdict, descriptor)| (descriptor.name.as_str(), verdict))
        .collect();
    let refused: Vec<&str> = verdicts
        .iter()
        .filter(|(_, verdict)| verdict.is_err())
        .map(|(name, _)| *name)
        .collect();
    checks.check(
        refused == vec![REFUSED_CAPABILITY]
            && verdicts
                .iter()
                .any(|(_, verdict)| *verdict == Err(MetisStatus::Unsupported)),
        "gate.version_refused_for_one_capability",
        format!("{verdicts:?}"),
    );

    // The refused capability itself is not broken: the plugin serves it when a
    // caller skips the gate, so the `Unsupported` above is the host's policy and
    // not the plugin's status.
    let ungated = state.drive(instance, REFUSED_CAPABILITY, "gate", 140);
    checks.check(
        ungated.status == MetisStatus::Ok,
        "gate.refusal_is_the_hosts",
        format!(
            "the plugin itself serves {REFUSED_CAPABILITY} with status={} when the gate is not applied",
            ungated.status.as_str()
        ),
    );

    // And the rest of the plugin is untouched by the refusal.
    let after = state.drive(instance, "tool.echo", "after the refusal", 141);
    checks.check(
        after.status == MetisStatus::Ok && after.body == "after the refusal",
        "gate.rest_of_plugin_still_serves",
        format!("status={} body={:?}", after.status.as_str(), after.body),
    );
}

/// This host's per-capability version policy.
///
/// A capability whose descriptor version is above the one this host serves is
/// refused for that name alone with `Unsupported`; everything else passes.
/// `plugin-runtime`'s registry carries the same rule for the loader, which is
/// what this demonstration is measured against.
fn capability_gate(descriptor: &CapabilityDescriptor) -> Result<(), MetisStatus> {
    if descriptor.version > HOST_CAPABILITY_VERSION {
        return Err(MetisStatus::Unsupported);
    }
    Ok(())
}

/// Drives every routed capability by name.
fn drive_capabilities(
    checks: &mut Checks,
    state: &HostState,
    instance: *mut c_void,
    list: Option<&CapabilityList>,
) {
    let declared = match list {
        Some(list) => list
            .capabilities
            .iter()
            .map(|descriptor| descriptor.name.clone())
            .collect::<Vec<_>>(),
        None => EXPECTED_CAPABILITIES
            .iter()
            .map(|name| name.to_string())
            .collect(),
    };
    let mut dispatched = Vec::new();
    for (index, capability) in ROUTED_CAPABILITIES.iter().enumerate() {
        let request_id = 100 + index as u64;
        let outcome = state.drive(instance, capability, "route", request_id);
        dispatched.push((capability.to_string(), outcome.status));
    }
    checks.check(
        dispatched
            .iter()
            .all(|(_, status)| *status == MetisStatus::Ok),
        "capabilities.every_capability_dispatched",
        format!("{dispatched:?}"),
    );
    checks.check(
        declared.len() == EXPECTED_CAPABILITIES.len(),
        "capabilities.declared_count",
        format!("{} declared", declared.len()),
    );

    let absent = state.drive(instance, "tool.absent", "route", 199);
    checks.check(
        absent.status == MetisStatus::NotFound,
        "capabilities.unknown_name_not_found",
        format!("status={}", absent.status.as_str()),
    );

    // Ready on the first poll, and the capability's event reached the host.
    let echo = state.drive(instance, "tool.echo", "hello", 200);
    checks.check(
        echo.status == MetisStatus::Ok && echo.polls == 1,
        "futures.ready_on_first_poll",
        format!("status={} polls={}", echo.status.as_str(), echo.polls),
    );
    checks.check(
        !echo.body.is_empty() && echo.events,
        "futures.capability_emitted_event",
        "the capability answered and its event reached the host",
    );

    // The host-answered timer: the first poll parks, the host answers
    // `service_complete`, the woken future answers.
    let pending = state.drive(instance, "tool.pending", "wait", 201);
    checks.check(
        pending.status == MetisStatus::Ok && pending.polls >= 2,
        "futures.host_answered_timer",
        format!("status={} polls={}", pending.status.as_str(), pending.polls),
    );
    checks.check(
        pending.saw_timer_request,
        "futures.service_request_payload",
        "the plugin asked for a timer with a plugin-owned payload",
    );
    checks.check(
        pending.body.contains("timer"),
        "futures.service_answer_reached_capability",
        format!("the capability decoded the host's answer: {}", pending.body),
    );

    // A cancel that resolves as Cancelled.
    let cancel = state.cancel_call(instance, "tool.cancellable", 202);
    checks.check(
        cancel.status == MetisStatus::Cancelled,
        "cancel.resolves_as_cancelled",
        format!("status={}", cancel.status.as_str()),
    );

    // A cancel the plugin never honours.
    let missed = state.cancel_call(instance, "tool.stubborn", 203);
    checks.check(
        missed.missed_deadline && missed.status == MetisStatus::Ok,
        "cancel.missed_deadline_recorded",
        format!("{:?}", missed.status),
    );
}

/// A request body carrying a field this build does not know.
///
/// The payload types never deny an unknown field and a field added later is
/// optional with a default, so a plugin built later and a host built earlier
/// still interoperate. The body is written here byte for byte because that is
/// the only place such a field can come from: no type in this build has it.
fn drive_unknown_field(checks: &mut Checks, state: &HostState, instance: *mut c_void) {
    let body = br#"{"echo":"from a newer build","repeat":2,"added_after_this_build":true}"#;
    let outcome = state.drive_body(instance, "tool.echo", body, 210);
    checks.check(
        outcome.status == MetisStatus::Ok && outcome.body == "from a newer build".repeat(2),
        "payload.unknown_field_tolerated",
        format!(
            "status={} body={:?} (the known fields kept their meaning)",
            outcome.status.as_str(),
            outcome.body
        ),
    );
}

/// The hazard checks: panics at both boundaries, a blocking poll, a double
/// drop, and the reclaim a late poll return performs.
fn run_hazard_checks(checks: &mut Checks, state: &HostState, instance: *mut c_void) {
    let panic = state.drive(instance, "tool.panic", "boom", 300);
    checks.check(
        panic.status == MetisStatus::Internal,
        "hazard.panic_in_poll",
        format!("status={}", panic.status.as_str()),
    );

    let drop_panic = state.panic_in_drop(instance, "tool.panic_drop", 301);
    checks.check(
        drop_panic,
        "hazard.panic_in_drop",
        "the destructor panicked, the drop was contained, and the process is still here",
    );

    let callback = state.callback_panic(instance, "tool.pending", 302);
    checks.check(
        callback,
        "hazard.panic_in_host_callback",
        "the callback's own frame caught the panic and the plugin saw a status",
    );

    let blocked = state.watchdog(instance, "tool.blocking", 303);
    checks.check(
        blocked.abandoned && blocked.quarantined,
        "hazard.blocking_poll_watchdog",
        format!("{blocked:?}"),
    );
    checks.check(
        blocked.reclaimed,
        "reclaim.late_poll_return",
        "the late poll return reclaimed the context box and the waker clone once",
    );
    checks.check(
        state.new_futures_refused(instance),
        "hazard.quarantine_refuses_new_futures",
        "a quarantined instance accepts no new future",
    );

    checks.check(
        state.double_drop(instance, "tool.echo", 304),
        "hazard.double_drop_refused",
        "the second drop was refused instead of freeing the context twice",
    );
}

// --- host state ------------------------------------------------------------

/// One waker the host handed to a plugin poll.
///
/// The slot is deliberately leaked: the contract says a wake may arrive after
/// the future was dropped, so what the wake dereferences has to outlive the
/// future. The counters it carries are what the harness reads.
struct WakerSlot {
    state: *const HostState,
    request_id: u64,
    live: AtomicBool,
}

impl Default for CallOutcome {
    fn default() -> Self {
        CallOutcome {
            status: MetisStatus::Internal,
            polls: 0,
            body: String::new(),
            events: false,
            saw_timer_request: false,
            missed_deadline: false,
        }
    }
}

impl Default for WatchdogOutcome {
    fn default() -> Self {
        WatchdogOutcome {
            abandoned: false,
            quarantined: false,
            reclaimed: false,
            status: MetisStatus::Internal,
        }
    }
}

/// The C implementer of the table, compiled by the script and driven here.
///
/// This is the half of the proof the Rust fixture cannot give: that the
/// committed header is enough for a non-Rust author. The library is compiled
/// with `-std=c11 -Wall -Wextra -Werror` under gcc and clang, loaded, and driven
/// through the same host driver as the fixture, and the C side's own ledger has
/// to agree with the host's counters at the end.
fn drive_c_plugin(checks: &mut Checks) {
    let root = repo_root();
    let script = root.join("scripts/build-c-plugin.sh");
    let compiled = Command::new("bash").arg(&script).output();
    match compiled {
        Ok(output) if output.status.success() => {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if !line.trim().is_empty() {
                    println!("[order] {line}");
                }
            }
            checks.check(
                true,
                "cplugin.compiled_by_gcc_and_clang",
                "both compilers accepted the committed header",
            );
        }
        Ok(output) => {
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            checks.check(
                false,
                "cplugin.compiled_by_gcc_and_clang",
                format!("the build script exited {}", output.status),
            );
            return;
        }
        Err(err) => {
            checks.check(false, "cplugin.compiled_by_gcc_and_clang", err);
            return;
        }
    }

    let path = root.join("target/c-plugin/libcplugin.so");
    if !checks.check(path.exists(), "cplugin.library_present", path.display()) {
        return;
    }
    // SAFETY: the harness built this library from the tree it is testing.
    let library = match unsafe { Library::new(&path) } {
        Ok(library) => library,
        Err(err) => {
            checks.check(
                false,
                "cplugin.library_loaded",
                format!("dlopen failed: {err}"),
            );
            return;
        }
    };
    checks.check(true, "cplugin.library_loaded", "the gcc build loads");

    let entry = unsafe { library.get::<MetisPluginEntryV1Fn>(METIS_PLUGIN_ENTRY_V1) };
    let table_ptr = match &entry {
        Ok(entry) => unsafe { entry() },
        Err(err) => {
            checks.check(false, "cplugin.entry_symbol", err);
            return;
        }
    };
    if !checks.check(
        !table_ptr.is_null(),
        "cplugin.entry_symbol",
        "metis_plugin_entry_v1 resolved",
    ) {
        return;
    }
    match metis_dynamic_symbols(&path) {
        Ok(symbols) => {
            checks.check(
                symbols == vec!["metis_plugin_entry_v1".to_string()],
                "cplugin.exactly_one_metis_symbol",
                format!("{symbols:?}"),
            );
        }
        Err(err) => {
            checks.check(false, "cplugin.exactly_one_metis_symbol", err);
        }
    }
    match unsafe { metis_read_table(table_ptr) } {
        Ok(read) => {
            checks.check(
                read.abi_major == METIS_ABI_MAJOR
                    && read.abi_minor == METIS_ABI_MINOR
                    && read.size == mem::size_of::<MetisPluginTableV1>(),
                "cplugin.table_is_the_revision",
                format!(
                    "the C table reports {}.{} and {} bytes",
                    read.abi_major, read.abi_minor, read.size
                ),
            );
        }
        Err(err) => {
            checks.check(false, "cplugin.table_is_the_revision", err);
        }
    }

    let state = HostState::new(table_ptr);
    state.order("the C library loaded");
    let mut instance: *mut c_void = std::ptr::null_mut();
    // SAFETY: the host table lives in the box and stays valid for the
    // instance's lifetime.
    let created = unsafe { ((*table_ptr).create_instance)(state.api(), &mut instance) };
    if !checks.check(
        created == MetisStatus::Ok && !instance.is_null(),
        "cplugin.instance_created",
        format!("status={}", created.as_str()),
    ) {
        return;
    }

    let mut listed = MetisBuffer::empty();
    let status = unsafe { ((*table_ptr).list_capabilities)(instance, &mut listed) };
    let names = if status == MetisStatus::Ok {
        let bytes = state.take_plugin_buffer(listed);
        metis_payload_body(&bytes)
            .ok()
            .and_then(|body| serde_json::from_slice::<CapabilityList>(body).ok())
            .map(|list| {
                list.capabilities
                    .iter()
                    .map(|descriptor| descriptor.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    checks.check(
        names
            == vec![
                "c.ready".to_string(),
                "c.timer".to_string(),
                "c.cancel".to_string(),
                "c.drop_in_flight".to_string(),
            ],
        "cplugin.capabilities_listed",
        format!("{names:?}"),
    );

    let ready = state.drive(instance, "c.ready", "ready", 400);
    checks.check(
        ready.status == MetisStatus::Ok && ready.polls == 1,
        "cplugin.ready_path",
        format!("status={} polls={}", ready.status.as_str(), ready.polls),
    );

    let timer = state.drive(instance, "c.timer", "timer", 401);
    checks.check(
        timer.status == MetisStatus::Ok && timer.polls >= 2,
        "cplugin.timer_round_trip",
        format!("status={} polls={}", timer.status.as_str(), timer.polls),
    );

    let cancel = state.cancel_call(instance, "c.cancel", 402);
    checks.check(
        cancel.status == MetisStatus::Cancelled,
        "cplugin.cancel_resolves_cancelled",
        format!("status={}", cancel.status.as_str()),
    );

    checks.check(
        state.drop_with_reply_in_flight(instance, "c.drop_in_flight", 403),
        "cplugin.drop_with_reply_in_flight",
        "the reply arrived after the drop and was reported, not dereferenced",
    );

    let shutdown = unsafe { ((*table_ptr).shutdown)(instance) };
    state.order("the C instance quiesced");
    checks.check(
        shutdown == MetisStatus::Ok,
        "cplugin.shutdown_ok",
        format!("status={}", shutdown.as_str()),
    );
    match state.c_ledger() {
        Some(ledger) => {
            checks.check(
                ledger.get("allocs") == ledger.get("frees") && ledger.get("live") == Some(&0),
                "cplugin.ledger_zero_live",
                format!("{ledger:?}"),
            );
            checks.check(
                ledger.get("double_frees") == Some(&0) && ledger.get("bad_frees") == Some(&0),
                "cplugin.no_double_free",
                format!("{ledger:?}"),
            );
            checks.check(
                ledger.get("waker_clones") == ledger.get("waker_releases"),
                "cplugin.c_waker_count_balanced",
                format!("{ledger:?}"),
            );
            checks.check(
                ledger
                    .get("replies_after_drop")
                    .copied()
                    .unwrap_or_default()
                    >= 1,
                "cplugin.reply_after_drop_recorded",
                format!("{ledger:?}"),
            );
        }
        None => {
            checks.check(
                false,
                "cplugin.ledger_zero_live",
                "the C plugin reported no ledger",
            );
        }
    }
    checks.check(
        state.waker_clones() == state.waker_drops(),
        "cplugin.host_waker_count_balanced",
        format!(
            "{} clones, {} releases",
            state.waker_clones(),
            state.waker_drops()
        ),
    );

    let destroy = unsafe { ((*table_ptr).destroy_instance)(instance) };
    checks.check(
        destroy == MetisStatus::Ok,
        "cplugin.destroy_ok",
        format!("status={}", destroy.as_str()),
    );
    // Nothing is inside the C plugin code once every future is dropped, so the
    // library may go.
    drop(library);
    state.order("the C library unloaded after destroy");
}

/// A plugin built for another ABI major.
///
/// The symbol name carries the major, so this host cannot resolve a v2 plugin's
/// entry symbol at all: there is no table pointer to read, and the refusal names
/// the major the plugin exports instead of the loader's own byte string. The
/// library is the one `scripts/build-c-plugin.sh` compiles, which is the same
/// step the C section above runs, so this section follows it.
fn drive_other_major_plugin(checks: &mut Checks) {
    let path = repo_root().join("target/c-plugin/libother-major.so");
    if !checks.check(
        path.exists(),
        "symbol.other_major_library_present",
        path.display(),
    ) {
        return;
    }
    // SAFETY: the harness built this library from the tree it is testing.
    let library = match unsafe { Library::new(&path) } {
        Ok(library) => library,
        Err(err) => {
            checks.check(
                false,
                "symbol.other_major_library_loaded",
                format!("dlopen failed: {err}"),
            );
            return;
        }
    };
    checks.check(
        true,
        "symbol.other_major_library_loaded",
        "the library built for major 2 loads",
    );

    let symbols = match metis_dynamic_symbols(&path) {
        Ok(symbols) => symbols,
        Err(err) => {
            checks.check(false, "symbol.other_major_exactly_one_symbol", err);
            return;
        }
    };
    checks.check(
        symbols == vec!["metis_plugin_entry_v2".to_string()],
        "symbol.other_major_exactly_one_symbol",
        format!("nm -D --defined-only reports {symbols:?}"),
    );

    // The refusal: typed, naming both majors, and reached without a table.
    match resolve_entry(&library, &symbols) {
        Err(
            error @ EntryError::MajorMismatch {
                expected: 1,
                found: 2,
            },
        ) => {
            checks.check(true, "symbol.other_major_typed_error", format!("{error}"));
        }
        Ok(_) => {
            checks.check(
                false,
                "symbol.other_major_typed_error",
                "the v1 entry symbol resolved in a plugin built for another major",
            );
        }
        Err(other) => {
            checks.check(
                false,
                "symbol.other_major_typed_error",
                format!("the refusal is not the typed major mismatch: {other}"),
            );
        }
    }
    drop(library);
}

/// The repository root, which is where the build scripts and their outputs live.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("the repository root")
        .to_path_buf()
}

/// The outcome of one driven call.
#[derive(Debug, Clone)]
struct CallOutcome {
    status: MetisStatus,
    polls: usize,
    body: String,
    events: bool,
    saw_timer_request: bool,
    missed_deadline: bool,
}

/// What the watchdog saw.
#[derive(Debug, Clone)]
struct WatchdogOutcome {
    abandoned: bool,
    quarantined: bool,
    reclaimed: bool,
    status: MetisStatus,
}

/// The host side of the boundary: the callback table, the counters, and the
/// ordering ledger.
///
/// It is boxed and never moved, because the plugin keeps the host table pointer
/// for its instance's lifetime and the waker slots point back at it.
struct HostState {
    table: *const MetisPluginTableV1,
    instance: AtomicPtr<c_void>,
    api: MetisHostApiV1,
    logs: Mutex<Vec<String>>,
    events: Mutex<Vec<String>>,
    services: Mutex<Vec<(String, u64, String)>>,
    answers: Mutex<Vec<u64>>,
    released: AtomicUsize,
    waker_clones: AtomicUsize,
    waker_drops: AtomicUsize,
    wake_after_drop: AtomicUsize,
    in_flight: Mutex<HashSet<u64>>,
    wakers: Mutex<HashMap<u64, Box<WakerSlot>>>,
    quarantined: AtomicBool,
    order: Mutex<Vec<String>>,
    callback_panic_request: AtomicUsize,
}

impl HostState {
    fn new(table: *const MetisPluginTableV1) -> Box<HostState> {
        let mut host = Box::new(HostState {
            table,
            instance: AtomicPtr::new(std::ptr::null_mut()),
            api: MetisHostApiV1 {
                size: mem::size_of::<MetisHostApiV1>(),
                user_data: std::ptr::null_mut(),
                progress: host_progress,
                emit_event: host_emit_event,
                log: host_log,
                service_request: host_service_request,
            },
            logs: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
            services: Mutex::new(Vec::new()),
            answers: Mutex::new(Vec::new()),
            released: AtomicUsize::new(0),
            waker_clones: AtomicUsize::new(0),
            waker_drops: AtomicUsize::new(0),
            wake_after_drop: AtomicUsize::new(0),
            in_flight: Mutex::new(HashSet::new()),
            wakers: Mutex::new(HashMap::new()),
            quarantined: AtomicBool::new(false),
            order: Mutex::new(Vec::new()),
            callback_panic_request: AtomicUsize::new(usize::MAX),
        });
        // The box never moves while it is alive, so this pointer stays valid
        // for every callback and every waker the plugin holds.
        host.api.user_data = &*host as *const HostState as *mut c_void;
        host
    }

    fn api(&self) -> *const MetisHostApiV1 {
        &self.api
    }

    fn order(&self, event: impl Into<String>) {
        let event = event.into();
        println!("[order] {event}");
        self.order.lock().unwrap().push(event);
    }

    fn ordering(&self) -> Vec<String> {
        self.order.lock().unwrap().clone()
    }

    fn released(&self) -> usize {
        self.released.load(SeqCst)
    }

    fn waker_clones(&self) -> usize {
        self.waker_clones.load(SeqCst)
    }

    fn waker_drops(&self) -> usize {
        self.waker_drops.load(SeqCst)
    }

    fn in_flight(&self) -> HashSet<u64> {
        self.in_flight.lock().unwrap().clone()
    }

    fn in_flight_is_empty(&self) -> bool {
        self.in_flight.lock().unwrap().is_empty()
    }

    fn saw_log(&self, needle: &str) -> bool {
        self.logs
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains(needle))
    }

    fn saw_event(&self, topic: &str) -> bool {
        self.events.lock().unwrap().iter().any(|t| t == topic)
    }

    /// The fixture's one-line ledger, parsed.
    fn fixture_ledger(&self) -> Option<HashMap<String, usize>> {
        let logs = self.logs.lock().unwrap();
        let line = logs
            .iter()
            .rev()
            .find(|line| line.contains("FIXTURE_ACCOUNT"))?
            .clone();
        let payload = line.split("FIXTURE_ACCOUNT").nth(1)?.trim().to_string();
        let mut ledger = HashMap::new();
        for field in payload.split_whitespace() {
            let (key, value) = field.split_once('=')?;
            ledger.insert(key.to_string(), value.parse().ok()?);
        }
        Some(ledger)
    }

    /// Reads a plugin-owned buffer and releases it through the plugin's own
    /// `release_buffer`, which is the only way a host may free one.
    fn take_plugin_buffer(&self, buffer: MetisBuffer) -> Vec<u8> {
        if buffer.ptr.is_null() {
            return Vec::new();
        }
        // SAFETY: the buffer is a plugin allocation, valid until released.
        let bytes = unsafe { slice::from_raw_parts(buffer.ptr, buffer.len) }.to_vec();
        // SAFETY: the table is the live one this host loaded.
        let status = unsafe { ((*self.table).release_buffer)(buffer) };
        assert_eq!(
            status,
            MetisStatus::Ok,
            "the plugin releases its own buffer"
        );
        self.released.fetch_add(1, SeqCst);
        bytes
    }

    /// Starts one call. A quarantined instance refuses a new future, which is
    /// the host's own policy once a poll has been abandoned.
    fn invoke(
        &self,
        instance: *mut c_void,
        capability: &str,
        body: &str,
        request_id: u64,
    ) -> Option<MetisFutureHandle> {
        let request = serde_json::to_vec(&EchoRequest {
            echo: body.to_string(),
            repeat: 1,
        })
        .unwrap_or_default();
        self.invoke_body(instance, capability, &request, request_id)
    }

    /// Starts one call whose request body the caller wrote itself.
    ///
    /// A body carrying a field this build does not know cannot come from a type
    /// this build has, so the bytes are the only place it can come from.
    fn invoke_body(
        &self,
        instance: *mut c_void,
        capability: &str,
        body: &[u8],
        request_id: u64,
    ) -> Option<MetisFutureHandle> {
        if self.quarantined.load(SeqCst) {
            println!("[order] a new future was refused: the instance is quarantined");
            return None;
        }
        let request = metis_encode_payload(body);
        let mut handle = MetisFutureHandle::null();
        // SAFETY: the instance is live, the request buffer is host-owned and
        // valid for this call, and the out parameter is a live local.
        let status = unsafe {
            ((*self.table).invoke_future)(
                instance,
                capability.as_ptr(),
                capability.len(),
                request_id,
                MetisBuffer {
                    ptr: request.as_ptr() as *mut u8,
                    len: request.len(),
                    cap: request.len(),
                    owner: METIS_BUFFER_OWNER_HOST,
                    reserved: 0,
                },
                self.api(),
                &mut handle,
            )
        };
        if status == MetisStatus::Ok {
            self.in_flight.lock().unwrap().insert(request_id);
            Some(handle)
        } else {
            println!(
                "[order] invoke_future for {capability} returned {}",
                status.as_str()
            );
            None
        }
    }

    /// One poll, with the host's waker for this request.
    fn poll(&self, handle: &MetisFutureHandle, request_id: u64) -> (MetisStatus, String) {
        let waker = self.waker_for(request_id);
        let mut out = MetisBuffer::empty();
        // SAFETY: the handle came from this table's `invoke_future`, and the
        // waker is this host's.
        let status = unsafe {
            let vtable = &*handle.vtable;
            (vtable.poll)(handle.ctx, waker, &mut out)
        };
        let body = if status == MetisStatus::Ok {
            let bytes = self.take_plugin_buffer(out);
            metis_payload_body(&bytes)
                .ok()
                .and_then(|body| serde_json::from_slice::<EchoResponse>(body).ok())
                .map(|response| response.echo)
                .unwrap_or_default()
        } else {
            String::new()
        };
        (status, body)
    }

    /// Drops one future and releases the slot that tracked it.
    fn drop_future(&self, handle: &MetisFutureHandle, request_id: u64) -> MetisStatus {
        // SAFETY: the handle came from this table and this is its one drop.
        let status = unsafe { ((*handle.vtable).drop)(handle.ctx) };
        if let Some(slot) = self.wakers.lock().unwrap().get(&request_id) {
            slot.live.store(false, SeqCst);
        }
        self.in_flight.lock().unwrap().remove(&request_id);
        status
    }

    /// Drives one call to its terminal status, answering the services it asks
    /// for.
    fn drive(
        &self,
        instance: *mut c_void,
        capability: &str,
        body: &str,
        request_id: u64,
    ) -> CallOutcome {
        let request = serde_json::to_vec(&EchoRequest {
            echo: body.to_string(),
            repeat: 1,
        })
        .unwrap_or_default();
        self.drive_body(instance, capability, &request, request_id)
    }

    /// Drives one call whose request body the caller wrote itself.
    fn drive_body(
        &self,
        instance: *mut c_void,
        capability: &str,
        body: &[u8],
        request_id: u64,
    ) -> CallOutcome {
        let mut outcome = CallOutcome::default();
        let Some(handle) = self.invoke_body(instance, capability, body, request_id) else {
            outcome.status = MetisStatus::Internal;
            return outcome;
        };
        let start = Instant::now();
        loop {
            outcome.polls += 1;
            let (status, response) = self.poll(&handle, request_id);
            match status {
                MetisStatus::Pending => {
                    if self.wanted_this(capability, request_id) {
                        outcome.saw_timer_request = true;
                    }
                    if self.take_answer(request_id) {
                        self.answer(instance, request_id, "{\"delay_ms\":20}");
                    }
                    if start.elapsed() > POLL_DEADLINE {
                        outcome.status = MetisStatus::Internal;
                        break;
                    }
                    // A future that never wakes must not be polled in a tight
                    // loop: a host yields, and this is that yield.
                    std::thread::sleep(Duration::from_millis(1));
                }
                terminal => {
                    outcome.body = response;
                    outcome.status = terminal;
                    break;
                }
            }
        }
        outcome.events = self.saw_event("fixture.answered");
        self.drop_future(&handle, request_id);
        outcome
    }

    fn wanted_this(&self, capability: &str, request_id: u64) -> bool {
        self.services
            .lock()
            .unwrap()
            .iter()
            .any(|(service, id, _)| service == "timer" && *id == request_id)
            || capability == "tool.pending"
    }

    /// Cancels a call and, if the plugin honours it, reports the terminal
    /// status; if the plugin misses the deadline, records the miss and still
    /// finishes the call rather than dropping it mid-flight.
    fn cancel_call(&self, instance: *mut c_void, capability: &str, request_id: u64) -> CallOutcome {
        let mut outcome = CallOutcome::default();
        let Some(handle) = self.invoke(instance, capability, "cancel", request_id) else {
            outcome.status = MetisStatus::Internal;
            return outcome;
        };
        let (first, _) = self.poll(&handle, request_id);
        outcome.polls = 1;
        if first != MetisStatus::Pending {
            outcome.status = first;
            self.drop_future(&handle, request_id);
            return outcome;
        }
        // SAFETY: the instance is live and the request is outstanding.
        let cancel = unsafe { ((*self.table).cancel)(instance, request_id) };
        if cancel != MetisStatus::Ok {
            outcome.status = cancel;
            self.drop_future(&handle, request_id);
            return outcome;
        }
        let deadline = Instant::now() + CANCEL_WINDOW;
        loop {
            outcome.polls += 1;
            let (status, body) = self.poll(&handle, request_id);
            match status {
                MetisStatus::Pending => {
                    if Instant::now() > deadline {
                        outcome.missed_deadline = true;
                        println!(
                            "[order] cancel deadline missed for request {request_id}; \
                             the host records it and finishes the call"
                        );
                        if self.take_answer(request_id) {
                            self.answer(instance, request_id, "{\"delay_ms\":20}");
                        }
                    }
                    if Instant::now() > deadline + POLL_DEADLINE {
                        outcome.status = MetisStatus::Internal;
                        break;
                    }
                }
                terminal => {
                    outcome.body = body;
                    outcome.status = terminal;
                    break;
                }
            }
        }
        self.drop_future(&handle, request_id);
        outcome
    }

    /// Drives a call whose future panics in its destructor.
    fn panic_in_drop(&self, instance: *mut c_void, capability: &str, request_id: u64) -> bool {
        let Some(handle) = self.invoke(instance, capability, "drop", request_id) else {
            return false;
        };
        let (polled, _) = self.poll(&handle, request_id);
        let dropped = self.drop_future(&handle, request_id);
        println!(
            "[order] drop of a panicking destructor returned {}",
            dropped.as_str()
        );
        polled == MetisStatus::Pending && dropped == MetisStatus::Internal
    }

    /// Drives a call whose `service_request` callback panics inside the host.
    fn callback_panic(&self, instance: *mut c_void, capability: &str, request_id: u64) -> bool {
        self.callback_panic_request
            .store(request_id as usize, SeqCst);
        let outcome = self.drive(instance, capability, "callback", request_id);
        self.callback_panic_request.store(usize::MAX, SeqCst);
        println!(
            "[order] a panicking host callback surfaced as {}",
            outcome.status.as_str()
        );
        outcome.status == MetisStatus::Internal
    }

    /// Polls a blocking capability under a watchdog, quarantines the instance
    /// when the poll is abandoned, and reclaims when it finally returns.
    fn watchdog(
        &self,
        instance: *mut c_void,
        capability: &str,
        request_id: u64,
    ) -> WatchdogOutcome {
        let mut outcome = WatchdogOutcome::default();
        let Some(handle) = self.invoke(instance, capability, "block", request_id) else {
            return outcome;
        };

        // The poll runs on a thread of the host's own, so a poll that blocks
        // does not block the watchdog that has to notice.
        let waker = self.waker_for(request_id);
        let ctx = handle.ctx as usize;
        let vtable = handle.vtable as usize;
        let waker_data = waker.data as usize;
        let waker_vtable = waker.vtable as usize;
        let (tx, rx) = mpsc::channel();
        let poll = std::thread::spawn(move || {
            // The handle and the waker travel as plain integers so the polling
            // thread owns copies of two-pointer values, which is exactly what
            // they are on the wire.
            // SAFETY: the context came from this table, the waker came from
            // this host, and the vtable pointer is a static.
            let waker = MetisWaker {
                data: waker_data as *mut c_void,
                vtable: waker_vtable as *const MetisWakerVtable,
            };
            let mut out = MetisBuffer::empty();
            let status = unsafe {
                let vtable = &*(vtable as *const MetisFutureVtable);
                (vtable.poll)(ctx as *mut c_void, waker, &mut out)
            };
            // The buffer travels as plain integers: a `MetisBuffer` holds a raw
            // pointer, so it is not `Send`, while the three numbers that
            // describe it are exactly what the other side needs.
            let _ = tx.send((status, out.ptr as usize, out.len, out.cap));
        });
        match rx.recv_timeout(WATCHDOG) {
            Ok((status, ptr, len, cap)) => {
                if status == MetisStatus::Ok {
                    let _ = self.take_plugin_buffer(MetisBuffer {
                        ptr: ptr as *mut u8,
                        len,
                        cap,
                        owner: METIS_BUFFER_OWNER_PLUGIN,
                        reserved: 0,
                    });
                }
                outcome.status = status;
            }
            Err(_) => {
                outcome.abandoned = true;
                self.order(format!(
                    "watchdog abandoned the poll for request {request_id} after {:?}",
                    WATCHDOG
                ));
                self.quarantined.store(true, SeqCst);
                outcome.quarantined = true;
                self.order("instance quarantined, its library stays resident");
                // Wait for the abandoned poll to return: reclaim is the
                // caller's, and the caller is this thread.
                let (status, ptr, len, cap) = rx.recv().expect("the abandoned poll returns");
                if status == MetisStatus::Ok {
                    let _ = self.take_plugin_buffer(MetisBuffer {
                        ptr: ptr as *mut u8,
                        len,
                        cap,
                        owner: METIS_BUFFER_OWNER_PLUGIN,
                        reserved: 0,
                    });
                }
                poll.join().expect("the poll thread returns");
                self.order(format!("the late poll returned {}", status.as_str()));
                outcome.reclaimed = true;
                outcome.status = status;
            }
        }
        let _ = instance;
        self.drop_future(&handle, request_id);
        outcome
    }

    /// Whether a quarantined instance refuses a new future.
    fn new_futures_refused(&self, instance: *mut c_void) -> bool {
        let refused = self
            .invoke(instance, "tool.echo", "after quarantine", 305)
            .is_none();
        self.quarantined.store(false, SeqCst);
        refused
    }

    /// The C plugin's one-line ledger, parsed.
    fn c_ledger(&self) -> Option<HashMap<String, usize>> {
        let logs = self.logs.lock().unwrap();
        let line = logs
            .iter()
            .rev()
            .find(|line| line.contains("CPLUGIN_ACCOUNT"))?
            .clone();
        let payload = line.split("CPLUGIN_ACCOUNT").nth(1)?.trim().to_string();
        let mut ledger = HashMap::new();
        for field in payload.split_whitespace() {
            let (key, value) = field.split_once('=')?;
            ledger.insert(key.to_string(), value.parse().ok()?);
        }
        Some(ledger)
    }

    /// Answers an outstanding service and reports the status the plugin gave,
    /// which is how the drop-with-a-reply-in-flight case is observed.
    fn answer_status(&self, instance: *mut c_void, request_id: u64, body: &str) -> MetisStatus {
        let encoded = metis_encode_payload(body.as_bytes());
        // SAFETY: the instance is live and the answer buffer is host-owned and
        // borrowed for this call.
        unsafe {
            ((*self.table).service_complete)(
                instance,
                request_id,
                MetisStatus::Ok,
                MetisBuffer {
                    ptr: encoded.as_ptr() as *mut u8,
                    len: encoded.len(),
                    cap: encoded.len(),
                    owner: METIS_BUFFER_OWNER_HOST,
                    reserved: 0,
                },
            )
        }
    }

    /// Starts a call that asks for a service, drops its future before the
    /// answer arrives, and then answers anyway.
    fn drop_with_reply_in_flight(
        &self,
        instance: *mut c_void,
        capability: &str,
        request_id: u64,
    ) -> bool {
        let Some(handle) = self.invoke(instance, capability, "inflight", request_id) else {
            return false;
        };
        let (parked, _) = self.poll(&handle, request_id);
        let dropped = self.drop_future(&handle, request_id);
        let answered = self.answer_status(instance, request_id, "{\"delay_ms\":20}");
        println!(
            "[order] the reply in flight for request {request_id} returned {}",
            answered.as_str()
        );
        parked == MetisStatus::Pending
            && dropped == MetisStatus::Ok
            && answered == MetisStatus::NotFound
    }

    /// Drops one future twice and reports whether the second drop was refused.
    fn double_drop(&self, instance: *mut c_void, capability: &str, request_id: u64) -> bool {
        let Some(handle) = self.invoke(instance, capability, "double", request_id) else {
            return false;
        };
        let _ = self.poll(&handle, request_id);
        // SAFETY: the handle came from this table; the second drop is the
        // hazard under test.
        let first = unsafe { ((*handle.vtable).drop)(handle.ctx) };
        let second = unsafe { ((*handle.vtable).drop)(handle.ctx) };
        println!(
            "[order] first drop {}, second drop {}",
            first.as_str(),
            second.as_str()
        );
        self.in_flight.lock().unwrap().remove(&request_id);
        first == MetisStatus::Ok && second != MetisStatus::Ok
    }

    fn waker_for(&self, request_id: u64) -> MetisWaker {
        let mut slots = self.wakers.lock().unwrap();
        let slot = slots.entry(request_id).or_insert_with(|| {
            Box::new(WakerSlot {
                state: self as *const HostState,
                request_id,
                live: AtomicBool::new(true),
            })
        });
        MetisWaker {
            data: &mut **slot as *mut WakerSlot as *mut c_void,
            vtable: &WAKER_VTABLE,
        }
    }

    fn take_answer(&self, request_id: u64) -> bool {
        let mut answers = self.answers.lock().unwrap();
        match answers.iter().position(|id| *id == request_id) {
            Some(index) => {
                answers.remove(index);
                true
            }
            None => false,
        }
    }

    /// Answers an outstanding service request with a host-owned payload.
    fn answer(&self, instance: *mut c_void, request_id: u64, body: &str) {
        let encoded = metis_encode_payload(body.as_bytes());
        // SAFETY: the instance is live and the answer buffer is host-owned and
        // borrowed for this call.
        let status = unsafe {
            ((*self.table).service_complete)(
                instance,
                request_id,
                MetisStatus::Ok,
                MetisBuffer {
                    ptr: encoded.as_ptr() as *mut u8,
                    len: encoded.len(),
                    cap: encoded.len(),
                    owner: METIS_BUFFER_OWNER_HOST,
                    reserved: 0,
                },
            )
        };
        assert_eq!(status, MetisStatus::Ok, "the plugin accepted the answer");
    }
}

// --- host callbacks --------------------------------------------------------
//
// Every one of them is an `extern "C"` frame the plugin calls into, so every
// one of them contains its own panic: only the callback's own frame can.

unsafe extern "C" fn host_progress(user_data: *mut c_void, request_id: u64, payload: MetisBuffer) {
    let state = unsafe { &*(user_data as *const HostState) };
    let body = state.take_plugin_buffer(payload);
    println!(
        "[order] progress for request {request_id}: {} bytes",
        body.len()
    );
}

unsafe extern "C" fn host_emit_event(
    user_data: *mut c_void,
    topic: *const u8,
    topic_len: usize,
    payload: MetisBuffer,
) {
    let state = unsafe { &*(user_data as *const HostState) };
    // SAFETY: the topic is borrowed for the call.
    let topic = unsafe { slice::from_raw_parts(topic, topic_len) };
    let topic = String::from_utf8_lossy(topic).to_string();
    let _ = state.take_plugin_buffer(payload);
    state.events.lock().unwrap().push(topic);
}

unsafe extern "C" fn host_log(
    user_data: *mut c_void,
    level: i32,
    message: *const u8,
    message_len: usize,
) {
    let state = unsafe { &*(user_data as *const HostState) };
    // SAFETY: the message is borrowed for the call.
    let message = unsafe { slice::from_raw_parts(message, message_len) };
    state
        .logs
        .lock()
        .unwrap()
        .push(format!("[{level}] {}", String::from_utf8_lossy(message)));
}

unsafe extern "C" fn host_service_request(
    user_data: *mut c_void,
    service: *const u8,
    service_len: usize,
    request_id: u64,
    payload: MetisBuffer,
) -> MetisStatus {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &*(user_data as *const HostState) };
        let panicking = state.callback_panic_request.load(SeqCst) == request_id as usize;
        // The service payload is plugin-owned and released here, from inside
        // the callback, which is the reentrancy the contract allows.
        let body = state.take_plugin_buffer(payload);
        // SAFETY: the service name is borrowed for the call.
        let service = unsafe { slice::from_raw_parts(service, service_len) };
        let service = String::from_utf8_lossy(service).to_string();
        println!("[order] service_request {service} for request {request_id}");
        state.services.lock().unwrap().push((
            service.clone(),
            request_id,
            String::from_utf8_lossy(&body).to_string(),
        ));
        if panicking {
            panic!("[harness] a deliberate panic inside a host callback");
        }
        if service != "timer" {
            return MetisStatus::NotFound;
        }
        state.answers.lock().unwrap().push(request_id);
        MetisStatus::Ok
    }));
    caught.unwrap_or(MetisStatus::Internal)
}

// --- the host waker -------------------------------------------------------

unsafe extern "C" fn waker_clone(waker: *const MetisWaker) -> MetisWaker {
    // SAFETY: the waker was handed out by this host and is live for the poll.
    let cloned = MetisWaker {
        data: unsafe { (*waker).data },
        vtable: unsafe { (*waker).vtable },
    };
    let slot = unsafe { &*(cloned.data as *const WakerSlot) };
    let state = unsafe { &*slot.state };
    state.waker_clones.fetch_add(1, SeqCst);
    cloned
}

unsafe extern "C" fn waker_wake(waker: MetisWaker) {
    // SAFETY: the data is a slot this host leaked on purpose.
    let slot = unsafe { &*(waker.data as *const WakerSlot) };
    let state = unsafe { &*slot.state };
    state.waker_drops.fetch_add(1, SeqCst);
    if !slot.live.load(SeqCst) {
        state.wake_after_drop.fetch_add(1, SeqCst);
        println!(
            "[order] a wake for request {} arrived after the future was dropped",
            slot.request_id
        );
    }
}

unsafe extern "C" fn waker_wake_by_ref(waker: *const MetisWaker) {
    // SAFETY: as in `waker_clone`.
    let data = unsafe { (*waker).data };
    let slot = unsafe { &*(data as *const WakerSlot) };
    let state = unsafe { &*slot.state };
    if !slot.live.load(SeqCst) {
        state.wake_after_drop.fetch_add(1, SeqCst);
    }
}

unsafe extern "C" fn waker_drop(waker: MetisWaker) {
    // SAFETY: as in `waker_clone`.
    let slot = unsafe { &*(waker.data as *const WakerSlot) };
    let state = unsafe { &*slot.state };
    state.waker_drops.fetch_add(1, SeqCst);
}

static WAKER_VTABLE: MetisWakerVtable = MetisWakerVtable {
    clone: waker_clone,
    wake: waker_wake,
    wake_by_ref: waker_wake_by_ref,
    drop: waker_drop,
};
