//! The ABI harness: it loads the fixture cdylib and drives every primitive of
//! the version 1 function table through `plugin-contract`.
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
//!
//! The harness is a real host: it loads the real library, allocates its own
//! request buffers, releases every plugin buffer it receives through the
//! plugin's own `release_buffer`, and checks its accounting against the
//! fixture's ledger. Nothing here reimplements the ABI.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::slice;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering::SeqCst};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use libloading::Library;
use plugin_contract::abi::*;
use plugin_contract::payload::{
    metis_encode_payload, metis_encode_payload_with, metis_payload_body,
};
use plugin_fixture::payload::{CapabilityKind, CapabilityList, EchoRequest, EchoResponse};

/// How long a pending request may take before the harness calls it missing. An
/// unbounded wait on a completion that never arrives hangs forever, which is
/// exactly what a missing callback looks like from the outside (D11).
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(5);

/// The request whose completion the harness panics inside, to prove that a
/// panic in a host callback is caught at the boundary instead of unwinding
/// through plugin frames (D7).
const HOST_PANIC_REQUEST_ID: u64 = 4;

/// The capabilities this host expects to discover.
const EXPECTED_CAPABILITIES: [(&str, CapabilityKind, u32); 4] = [
    ("tool.echo", CapabilityKind::Tool, 1),
    ("tool.pending", CapabilityKind::Tool, 1),
    ("tool.cancellable", CapabilityKind::Tool, 1),
    ("tool.panic", CapabilityKind::Tool, 1),
];

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

    // SAFETY: the harness controls the library it just built, and it keeps the
    // library loaded until after `shutdown` has joined every plugin worker, so
    // no plugin thread can call back into unloaded code.
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

    // --- the entry symbol (D8) --------------------------------------------

    let entry = unsafe { library.get::<MetisPluginEntryV1Fn>(METIS_PLUGIN_ENTRY_V1) };
    let table_ptr = match &entry {
        Ok(entry) => unsafe { entry() },
        Err(err) => {
            checks.check(
                false,
                "symbol.entry_v1_resolved",
                format!("dlsym failed: {err}"),
            );
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

    // A differing major fails at resolution, before any struct is interpreted.
    let wrong_major = unsafe { library.get::<MetisPluginEntryV1Fn>(b"metis_plugin_entry_v2\0") };
    checks.check(
        wrong_major.is_err(),
        "symbol.wrong_major_absent",
        match &wrong_major {
            Ok(_) => "metis_plugin_entry_v2 unexpectedly resolved".to_string(),
            Err(err) => format!("metis_plugin_entry_v2 lookup failed: {err}"),
        },
    );

    // Exactly one versioned entry symbol: this is the seam the loader depends
    // on, so the check reads the dynamic symbol table rather than trusting the
    // names the harness happens to try.
    match metis_dynamic_symbols(&path) {
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

    // --- the table prefix (D8) --------------------------------------------

    match unsafe { metis_read_prefix(table_ptr) } {
        Some(prefix) => {
            checks.check(
                prefix.size == mem::size_of::<MetisPluginTableV1>(),
                "table.size_matches_layout",
                format!(
                    "plugin reports size={} this build's layout is {}",
                    prefix.size,
                    mem::size_of::<MetisPluginTableV1>()
                ),
            );
            checks.check(
                prefix.abi_major == METIS_ABI_MAJOR && prefix.abi_minor == METIS_ABI_MINOR,
                "table.version_is_1_0",
                format!(
                    "abi {}.{} (expected {}.{})",
                    prefix.abi_major, prefix.abi_minor, METIS_ABI_MAJOR, METIS_ABI_MINOR
                ),
            );
        }
        None => {
            checks.check(
                false,
                "table.size_matches_layout",
                "the table prefix is unreadable",
            );
            checks.check(
                false,
                "table.version_is_1_0",
                "the table prefix is unreadable",
            );
        }
    }

    // A table that claims to be shorter than the prefix must be refused, not
    // read past its end.
    #[repr(C)]
    struct ShortTable {
        size: usize,
        abi_major: u32,
        abi_minor: u32,
    }
    let short = ShortTable {
        size: mem::size_of::<usize>(),
        abi_major: METIS_ABI_MAJOR,
        abi_minor: METIS_ABI_MINOR,
    };
    checks.check(
        unsafe { metis_read_prefix(&short as *const ShortTable as *const MetisPluginTableV1) }
            .is_none(),
        "table.short_table_rejected",
        "a table reporting size=8 is refused instead of read",
    );

    // An older host knows only the prefix and the first entries. It must read
    // exactly those from a table that has grown since (D8).
    #[repr(C)]
    struct OlderHostView {
        size: usize,
        abi_major: u32,
        abi_minor: u32,
        create_instance: MetisCreateInstanceFn,
        destroy_instance: MetisDestroyInstanceFn,
    }
    let older_view = unsafe { &*(table_ptr as *const OlderHostView) };
    let prefix_agrees = older_view.size >= mem::size_of::<OlderHostView>()
        && matches!(
            (older_view.create_instance, unsafe { (*table_ptr).create_instance }),
            (a, b) if std::ptr::fn_addr_eq(a, b)
        )
        && matches!(
            (older_view.destroy_instance, unsafe { (*table_ptr).destroy_instance }),
            (a, b) if std::ptr::fn_addr_eq(a, b)
        );
    checks.check(
        prefix_agrees,
        "table.older_host_reads_its_prefix",
        "the prefix an older host knows agrees with the full table",
    );

    // --- host table, instance lifecycle, callbacks (D1, D11) ---------------

    let state = Box::new(HostState::new(table_ptr));
    let host_api = Box::new(MetisHostApiV1 {
        size: mem::size_of::<MetisHostApiV1>(),
        user_data: &*state as *const HostState as *mut c_void,
        complete: host_complete,
        progress: host_progress,
        emit_event: host_emit_event,
        log: host_log,
    });
    state.host_api.store(
        &*host_api as *const MetisHostApiV1 as *mut MetisHostApiV1,
        SeqCst,
    );

    let mut instance: *mut c_void = std::ptr::null_mut();
    let create = unsafe { (*table_ptr).create_instance };
    let create_status = unsafe { create(state.host_api(), &mut instance) };
    state.instance.store(instance, SeqCst);
    checks.check(
        create_status == MetisStatus::Ok && !instance.is_null(),
        "instance.create_ok",
        format!("status={} instance={:p}", create_status.as_str(), instance),
    );
    if create_status != MetisStatus::Ok || instance.is_null() {
        return checks.report();
    }
    checks.check(
        state.saw_log("fixture: instance created"),
        "callback.log_observed",
        format!(
            "{} log lines through the host table",
            state.log_line_count()
        ),
    );
    checks.check(
        state.event_callbacks.load(SeqCst) == 1,
        "callback.emit_event_from_create",
        format!("event callbacks={}", state.event_callbacks.load(SeqCst)),
    );

    // --- capability discovery (D9) ----------------------------------------

    let list_capabilities = unsafe { (*table_ptr).list_capabilities };
    let mut caps_buffer = MetisBuffer::empty();
    let caps_status = unsafe { list_capabilities(instance, &mut caps_buffer) };
    let descriptors = match state.read_and_release(caps_buffer) {
        Ok(body) => match serde_json::from_slice::<CapabilityList>(&body) {
            Ok(list) => list.capabilities,
            Err(err) => {
                checks.check(
                    false,
                    "capabilities.listed",
                    format!("capability payload did not parse: {err}"),
                );
                Vec::new()
            }
        },
        Err(err) => {
            checks.check(false, "capabilities.listed", err);
            Vec::new()
        }
    };
    let listed: Vec<(&str, CapabilityKind, u32)> = descriptors
        .iter()
        .map(|descriptor| {
            (
                descriptor.name.as_str(),
                descriptor.kind,
                descriptor.version,
            )
        })
        .collect();
    let all_expected = listed.len() == EXPECTED_CAPABILITIES.len()
        && EXPECTED_CAPABILITIES
            .iter()
            .all(|expected| listed.contains(expected));
    checks.check(
        caps_status == MetisStatus::Ok && all_expected,
        "capabilities.listed",
        format!("status={} descriptors={listed:?}", caps_status.as_str()),
    );
    let schemas_named = descriptors
        .iter()
        .all(|descriptor| !descriptor.schema.is_empty());
    checks.check(
        schemas_named,
        "capabilities.descriptors_carry_a_schema",
        format!(
            "schemas={:?}",
            descriptors
                .iter()
                .map(|descriptor| descriptor.schema.as_str())
                .collect::<Vec<_>>()
        ),
    );
    // Per-capability version gating is deliberately not exercised here: it is
    // `plugin-compatibility`'s subject, and this fixture advertises no version a
    // host would have to refuse.

    // --- the envelope across the boundary (D4) ----------------------------

    let request_body = serde_json::to_vec(&EchoRequest {
        echo: "hi".to_string(),
        repeat: 2,
    })
    .expect("the request serialises");
    for (label, header, expected) in [
        (
            "envelope.reserved_bits_invalid_argument",
            0x81_u8,
            MetisStatus::InvalidArgument,
        ),
        (
            "envelope.bad_format_version_invalid_argument",
            0x02,
            MetisStatus::InvalidArgument,
        ),
        (
            "envelope.unknown_codec_unsupported",
            0x11,
            MetisStatus::Unsupported,
        ),
    ] {
        let request = HostRequest::with_header(&state, header, &request_body);
        let mut out = MetisBuffer::empty();
        let status = unsafe { call_invoke(&state, 900, "tool.echo", request.buffer(), &mut out) };
        drop(request);
        checks.check(
            status == expected,
            label,
            format!(
                "header {header:#04x} returned {} (expected {})",
                status.as_str(),
                expected.as_str()
            ),
        );
    }

    // --- inline invoke (D11) ----------------------------------------------

    let request = HostRequest::new(&state, &request_body);
    let mut out = MetisBuffer::empty();
    let status = unsafe { call_invoke(&state, 1, "tool.echo", request.buffer(), &mut out) };
    let inline_body = state.read_and_release(out);
    drop(request);
    let echo: Option<EchoResponse> = inline_body
        .as_ref()
        .ok()
        .and_then(|body| serde_json::from_slice(body).ok());
    checks.check(
        status == MetisStatus::Ok
            && matches!(&echo, Some(response) if response.echo == "hihi" && response.bytes == 4),
        "invoke.inline_result",
        format!(
            "status={} body={}",
            status.as_str(),
            inline_body
                .map(|body| String::from_utf8_lossy(&body).into_owned())
                .unwrap_or_else(|err| err)
        ),
    );

    let request = HostRequest::new(&state, &request_body);
    let mut out = MetisBuffer::empty();
    let status = unsafe { call_invoke(&state, 2, "tool.nope", request.buffer(), &mut out) };
    drop(request);
    checks.check(
        status == MetisStatus::NotFound,
        "invoke.unknown_capability_not_found",
        format!("status={}", status.as_str()),
    );

    // --- pending invoke, progress, completion (D11) -----------------------

    let request = HostRequest::new(&state, &request_body);
    let mut out = MetisBuffer::empty();
    state.register_pending(3);
    let status = unsafe { call_invoke(&state, 3, "tool.pending", request.buffer(), &mut out) };
    drop(request);
    checks.check(
        status == MetisStatus::Pending && out.ptr.is_null(),
        "invoke.pending_returned",
        format!("status={} out buffer is null", status.as_str()),
    );
    let completed = state.wait_for_completion(3, COMPLETION_TIMEOUT);
    let pending_body: Option<EchoResponse> = completed
        .as_ref()
        .and_then(|completed| serde_json::from_slice(&completed.body).ok());
    checks.check(
        matches!(&completed, Some(completed) if completed.status == MetisStatus::Ok)
            && matches!(&pending_body, Some(response) if response.echo == "async" && response.bytes == 5),
        "invoke.pending_completion_via_callback",
        format!(
            "outcome={:?} body={:?}",
            completed.as_ref().map(|completed| completed.status.as_str()),
            pending_body
        ),
    );
    checks.check(
        state.progress_callbacks.load(SeqCst) == 1,
        "invoke.pending_progress_callback",
        format!(
            "progress callbacks={}",
            state.progress_callbacks.load(SeqCst)
        ),
    );

    // --- a panic inside a host callback (D7) ------------------------------

    let request = HostRequest::new(&state, &request_body);
    let mut out = MetisBuffer::empty();
    state.register_pending(HOST_PANIC_REQUEST_ID);
    let status = unsafe {
        call_invoke(
            &state,
            HOST_PANIC_REQUEST_ID,
            "tool.pending",
            request.buffer(),
            &mut out,
        )
    };
    drop(request);
    let completed = state.wait_for_completion(HOST_PANIC_REQUEST_ID, COMPLETION_TIMEOUT);
    checks.check(
        status == MetisStatus::Pending
            && matches!(&completed, Some(completed) if completed.status == MetisStatus::Internal),
        "callback.host_callback_panic_is_internal",
        format!(
            "invoke={} outcome={:?} (the harness panicked inside its complete callback)",
            status.as_str(),
            completed
                .as_ref()
                .map(|completed| completed.status.as_str())
        ),
    );
    checks.check(
        state.host_callback_panics.load(SeqCst) == 1,
        "callback.host_callback_panic_caught_once",
        format!(
            "host callback panics caught={}",
            state.host_callback_panics.load(SeqCst)
        ),
    );

    // --- cancel (D11, D12) ------------------------------------------------

    let request = HostRequest::new(&state, &request_body);
    let mut out = MetisBuffer::empty();
    state.register_pending(5);
    let status = unsafe { call_invoke(&state, 5, "tool.cancellable", request.buffer(), &mut out) };
    drop(request);
    let cancel_status = unsafe { call_cancel(&state, 5) };
    state.cancelled.lock().unwrap().insert(5);
    // No completion is guaranteed after a successful cancel, so the host
    // retires its own pending entry (D12): ignoring late callbacks alone would
    // leak one record per cancelled request.
    state.pending.lock().unwrap().remove(&5);
    checks.check(
        status == MetisStatus::Pending && cancel_status == MetisStatus::Ok,
        "cancel.accepted",
        format!(
            "invoke={} cancel={}",
            status.as_str(),
            cancel_status.as_str()
        ),
    );
    let suppressed = state.wait_until(COMPLETION_TIMEOUT, |state| {
        state.saw_log("suppressed completion for cancelled request 5")
    });
    checks.check(
        suppressed,
        "cancel.plugin_suppressed_completion",
        "the plugin logged the suppression instead of completing",
    );
    let no_completion = !state.completed.lock().unwrap().contains_key(&5);
    checks.check(
        no_completion,
        "cancel.no_completion_after_cancel",
        format!("completed entries for request 5 present={}", !no_completion),
    );

    let unknown_cancel = unsafe { call_cancel(&state, 4242) };
    checks.check(
        unknown_cancel == MetisStatus::NotFound,
        "cancel.unknown_request_not_found",
        format!("status={}", unknown_cancel.as_str()),
    );

    // `tool.pending` never honours a cancel, so its completion arrives anyway.
    // The host drops that completion and still releases the buffer it carried,
    // so a cancel cannot leak plugin memory (D6, D12).
    let request = HostRequest::new(&state, &request_body);
    let mut out = MetisBuffer::empty();
    state.register_pending(6);
    let status = unsafe { call_invoke(&state, 6, "tool.pending", request.buffer(), &mut out) };
    drop(request);
    let cancel_status = unsafe { call_cancel(&state, 6) };
    state.cancelled.lock().unwrap().insert(6);
    state.pending.lock().unwrap().remove(&6);
    let dropped = state.wait_until(COMPLETION_TIMEOUT, |state| {
        state.dropped_completions.load(SeqCst) == 1
    });
    checks.check(
        status == MetisStatus::Pending && cancel_status == MetisStatus::Ok && dropped,
        "cancel.host_dropped_late_completion",
        format!(
            "invoke={} cancel={} dropped={}",
            status.as_str(),
            cancel_status.as_str(),
            dropped
        ),
    );
    let released = state.dropped_buffers_released.load(SeqCst);
    let late_recorded = state.completed.lock().unwrap().contains_key(&6);
    checks.check(
        released == 1 && !late_recorded,
        "cancel.dropped_payload_still_released",
        format!("dropped payloads released={released} a completion was recorded={late_recorded}"),
    );

    // --- a panic inside an exported function (D7) -------------------------

    let request = HostRequest::new(&state, &request_body);
    let mut out = MetisBuffer::empty();
    let status = unsafe { call_invoke(&state, 7, "tool.panic", request.buffer(), &mut out) };
    drop(request);
    checks.check(
        status == MetisStatus::Internal && state.saw_log("panic caught at the boundary"),
        "panic.exported_function_is_internal",
        format!("status={} (the plugin logged the catch)", status.as_str()),
    );

    let request = HostRequest::new(
        &state,
        &serde_json::to_vec(&EchoRequest {
            echo: "z".to_string(),
            repeat: 3,
        })
        .expect("the request serialises"),
    );
    let mut out = MetisBuffer::empty();
    let status = unsafe { call_invoke(&state, 8, "tool.echo", request.buffer(), &mut out) };
    let after_panic = state
        .read_and_release(out)
        .ok()
        .and_then(|body| serde_json::from_slice::<EchoResponse>(&body).ok());
    drop(request);
    checks.check(
        status == MetisStatus::Ok
            && matches!(&after_panic, Some(response) if response.echo == "zzz"),
        "panic.instance_still_serves_the_next_call",
        format!("status={} body={after_panic:?}", status.as_str()),
    );

    // --- shutdown and the two ledgers (D6) --------------------------------

    let shutdown_status = unsafe { call_shutdown(&state) };
    checks.check(
        shutdown_status == MetisStatus::Ok,
        "instance.shutdown_ok",
        format!("status={}", shutdown_status.as_str()),
    );
    let destroy_after_shutdown = unsafe { call_destroy(&state) };
    checks.check(
        destroy_after_shutdown == MetisStatus::NotFound,
        "instance.destroy_after_shutdown_not_found",
        format!(
            "releasing the handle twice returns {}",
            destroy_after_shutdown.as_str()
        ),
    );

    let ledger = state.fixture_ledger();
    match &ledger {
        Some(ledger) => {
            let balanced =
                ledger.get("allocs") == ledger.get("frees") && ledger.get("live") == Some(&0);
            checks.check(
                balanced,
                "ownership.plugin_ledger_balanced",
                format!(
                    "allocs={:?} frees={:?} live={:?}",
                    ledger.get("allocs"),
                    ledger.get("frees"),
                    ledger.get("live")
                ),
            );
            checks.check(
                ledger.get("violations") == Some(&0) && ledger.get("suppressed") == Some(&1),
                "ownership.plugin_ledger_clean",
                format!(
                    "violations={:?} suppressed={:?}",
                    ledger.get("violations"),
                    ledger.get("suppressed")
                ),
            );
            let received = state.plugin_buffers_released.load(SeqCst);
            checks.check(
                ledger.get("allocs") == Some(&received),
                "ownership.ledger_agrees_with_harness",
                format!(
                    "plugin allocs={:?} harness releases={received}",
                    ledger.get("allocs")
                ),
            );
        }
        None => {
            checks.check(
                false,
                "ownership.plugin_ledger_balanced",
                "the fixture logged no ledger line",
            );
            checks.check(
                false,
                "ownership.plugin_ledger_clean",
                "the fixture logged no ledger line",
            );
            checks.check(
                false,
                "ownership.ledger_agrees_with_harness",
                "the fixture logged no ledger line",
            );
        }
    }

    let host_allocs = state.host_allocs.load(SeqCst);
    let host_frees = state.host_frees.load(SeqCst);
    checks.check(
        host_allocs == host_frees && host_allocs > 0,
        "ownership.host_allocations_freed_by_host",
        format!("host allocs={host_allocs} frees={host_frees}"),
    );

    let foreign = state.foreign_free_attempts.load(SeqCst);
    let rejections = state.rejections.lock().unwrap().clone();
    checks.check(
        foreign == 0 && rejections.is_empty(),
        "ownership.no_foreign_or_rejected_releases",
        format!("a buffer the host did not allocate was never offered: attempts={foreign} rejections={rejections:?}"),
    );

    let pending_left = state.pending.lock().unwrap().len();
    checks.check(
        pending_left == 0,
        "ownership.pending_registry_empty",
        format!("pending entries left={pending_left}"),
    );

    // The library may only be unloaded once no plugin thread can call back,
    // which `shutdown` guaranteed by joining them (D12).
    drop(host_api);
    drop(state);
    drop(library);

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

// --- rebuilding and locating the fixture -----------------------------------

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

// --- host-allocated request payloads (D6) ---------------------------------

/// A request payload the host allocated, so the host frees it.
///
/// Its buffer is tagged `METIS_BUFFER_OWNER_HOST` and is valid only for the call
/// that receives it; the pointer is never handed to the plugin's
/// `release_buffer`.
struct HostRequest<'a> {
    state: &'a HostState,
    bytes: Vec<u8>,
}

impl<'a> HostRequest<'a> {
    fn new(state: &'a HostState, body: &[u8]) -> Self {
        state.host_allocs.fetch_add(1, SeqCst);
        HostRequest {
            state,
            bytes: metis_encode_payload(body),
        }
    }

    /// A request with an arbitrary header byte, for the envelope checks.
    fn with_header(state: &'a HostState, header: u8, body: &[u8]) -> Self {
        state.host_allocs.fetch_add(1, SeqCst);
        HostRequest {
            state,
            bytes: metis_encode_payload_with(header, body),
        }
    }

    fn buffer(&self) -> MetisBuffer {
        MetisBuffer {
            ptr: self.bytes.as_ptr() as *mut u8,
            len: self.bytes.len(),
            cap: self.bytes.capacity(),
            owner: METIS_BUFFER_OWNER_HOST,
            reserved: 0,
        }
    }
}

impl Drop for HostRequest<'_> {
    fn drop(&mut self) {
        self.state.host_frees.fetch_add(1, SeqCst);
    }
}

// --- host state ------------------------------------------------------------

struct Completed {
    status: MetisStatus,
    body: Vec<u8>,
}

struct HostState {
    /// Borrowed from the plugin and valid while the library stays loaded.
    plugin: *const MetisPluginTableV1,
    host_api: AtomicPtr<MetisHostApiV1>,
    instance: AtomicPtr<c_void>,

    pending: Mutex<HashSet<u64>>,
    completed: Mutex<HashMap<u64, Completed>>,
    cancelled: Mutex<HashSet<u64>>,
    completion_cv: Condvar,

    host_allocs: AtomicUsize,
    host_frees: AtomicUsize,
    plugin_buffers_released: AtomicUsize,
    foreign_free_attempts: AtomicUsize,
    completion_callbacks: AtomicUsize,
    progress_callbacks: AtomicUsize,
    event_callbacks: AtomicUsize,
    dropped_completions: AtomicUsize,
    dropped_buffers_released: AtomicUsize,
    host_callback_panics: AtomicUsize,

    logs: Mutex<Vec<(i32, String)>>,
    rejections: Mutex<Vec<String>>,
}

impl HostState {
    fn new(plugin: *const MetisPluginTableV1) -> Self {
        HostState {
            plugin,
            host_api: AtomicPtr::new(std::ptr::null_mut()),
            instance: AtomicPtr::new(std::ptr::null_mut()),
            pending: Mutex::new(HashSet::new()),
            completed: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(HashSet::new()),
            completion_cv: Condvar::new(),
            host_allocs: AtomicUsize::new(0),
            host_frees: AtomicUsize::new(0),
            plugin_buffers_released: AtomicUsize::new(0),
            foreign_free_attempts: AtomicUsize::new(0),
            completion_callbacks: AtomicUsize::new(0),
            progress_callbacks: AtomicUsize::new(0),
            event_callbacks: AtomicUsize::new(0),
            dropped_completions: AtomicUsize::new(0),
            dropped_buffers_released: AtomicUsize::new(0),
            host_callback_panics: AtomicUsize::new(0),
            logs: Mutex::new(Vec::new()),
            rejections: Mutex::new(Vec::new()),
        }
    }

    fn host_api(&self) -> *const MetisHostApiV1 {
        self.host_api.load(SeqCst)
    }

    fn register_pending(&self, request_id: u64) {
        // D11: pending state is registered before the plugin is called, so a
        // completion that overtakes the return still finds its request.
        self.pending.lock().unwrap().insert(request_id);
    }

    fn record_completion(&self, request_id: u64, status: MetisStatus, body: Vec<u8>) {
        self.completed
            .lock()
            .unwrap()
            .insert(request_id, Completed { status, body });
        self.completion_cv.notify_all();
    }

    /// Releases a plugin-allocated buffer through the plugin's own
    /// `release_buffer`, which is the only code allowed to free plugin memory.
    fn release_plugin_buffer(&self, buffer: MetisBuffer) -> MetisStatus {
        if buffer.owner != METIS_BUFFER_OWNER_PLUGIN {
            self.foreign_free_attempts.fetch_add(1, SeqCst);
            self.rejections.lock().unwrap().push(format!(
                "a buffer with owner={} was offered to release_buffer",
                buffer.owner
            ));
            return MetisStatus::InvalidArgument;
        }
        if buffer.ptr.is_null() {
            return MetisStatus::Ok;
        }
        // SAFETY: `plugin` outlives the state, and the table is a plugin static.
        let table = unsafe { &*self.plugin };
        let release = table.release_buffer;
        let status = unsafe { release(buffer) };
        if status == MetisStatus::Ok {
            self.plugin_buffers_released.fetch_add(1, SeqCst);
        } else {
            self.rejections
                .lock()
                .unwrap()
                .push(format!("release_buffer returned {}", status.as_str()));
        }
        status
    }

    /// Copies a plugin payload out, validating the envelope, then releases it.
    fn read_and_release(&self, buffer: MetisBuffer) -> Result<Vec<u8>, String> {
        if buffer.ptr.is_null() {
            return Ok(Vec::new());
        }
        // SAFETY: the plugin allocated `len` bytes at `ptr`, and it stays valid
        // until `release_buffer` is called below.
        let bytes = unsafe { slice::from_raw_parts(buffer.ptr, buffer.len) };
        let body = metis_payload_body(bytes).map(<[u8]>::to_vec);
        self.release_plugin_buffer(buffer);
        body.map_err(|status| format!("the payload header was rejected with {}", status.as_str()))
    }

    fn wait_for_completion(&self, request_id: u64, timeout: Duration) -> Option<Completed> {
        let deadline = Instant::now() + timeout;
        let mut completed = self.completed.lock().unwrap();
        loop {
            if let Some(completed) = completed.remove(&request_id) {
                return Some(completed);
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            completed = self
                .completion_cv
                .wait_timeout(completed, deadline - now)
                .unwrap()
                .0;
        }
    }

    fn wait_until(&self, timeout: Duration, mut predicate: impl FnMut(&HostState) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if predicate(self) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn log_lines(&self) -> Vec<String> {
        self.logs
            .lock()
            .unwrap()
            .iter()
            .map(|(_, line)| line.clone())
            .collect()
    }

    fn log_line_count(&self) -> usize {
        self.logs.lock().unwrap().len()
    }

    fn saw_log(&self, needle: &str) -> bool {
        self.log_lines().iter().any(|line| line.contains(needle))
    }

    /// Parses the fixture's accounting line, which is logged through the host's
    /// own `log` callback at shutdown.
    fn fixture_ledger(&self) -> Option<HashMap<String, usize>> {
        let line = self
            .log_lines()
            .into_iter()
            .find(|line| line.contains("FIXTURE_ACCOUNT"))?;
        let mut fields = HashMap::new();
        for token in line.split_whitespace() {
            if let Some((key, value)) = token.split_once('=') {
                fields.insert(key.to_string(), value.parse().unwrap_or(usize::MAX));
            }
        }
        Some(fields)
    }
}

// --- host callbacks (D7, D12) ---------------------------------------------

/// Every callback body is wrapped in `catch_unwind`: a callback is itself an
/// `extern "C"` frame reached from plugin code, so a panic that escaped it
/// would unwind into plugin frames. The catch is observable, not silent: a
/// completion whose callback panicked is recorded as an internal error.
unsafe extern "C" fn host_complete(
    user_data: *mut c_void,
    request_id: u64,
    status: MetisStatus,
    payload: MetisBuffer,
) {
    let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user_data` is the boxed HostState, alive for the whole run.
        let state = unsafe { &*(user_data as *const HostState) };
        state.completion_callbacks.fetch_add(1, SeqCst);
        if request_id == HOST_PANIC_REQUEST_ID {
            panic!("harness: deliberate panic inside a host callback");
        }
        state.pending.lock().unwrap().remove(&request_id);

        // A completion that arrives after a successful cancel is dropped, but
        // the payload it carries is plugin memory and only the plugin may free
        // it (D6, D12).
        if state.cancelled.lock().unwrap().contains(&request_id) {
            state.dropped_completions.fetch_add(1, SeqCst);
            if !payload.ptr.is_null() && state.release_plugin_buffer(payload) == MetisStatus::Ok {
                state.dropped_buffers_released.fetch_add(1, SeqCst);
            }
            return;
        }

        let body = match state.read_and_release(payload) {
            Ok(body) => body,
            Err(err) => {
                state.rejections.lock().unwrap().push(err);
                Vec::new()
            }
        };
        state.record_completion(request_id, status, body);
    }));

    if caught.is_err() {
        // SAFETY: as above.
        let state = unsafe { &*(user_data as *const HostState) };
        state.host_callback_panics.fetch_add(1, SeqCst);
        state.pending.lock().unwrap().remove(&request_id);
        if !payload.ptr.is_null() {
            state.release_plugin_buffer(payload);
        }
        state.record_completion(request_id, MetisStatus::Internal, Vec::new());
    }
}

unsafe extern "C" fn host_progress(user_data: *mut c_void, request_id: u64, payload: MetisBuffer) {
    let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { &*(user_data as *const HostState) };
        state.progress_callbacks.fetch_add(1, SeqCst);
        let body = state.read_and_release(payload).unwrap_or_default();
        state.logs.lock().unwrap().push((
            METIS_LOG_INFO,
            format!(
                "host: progress for request {request_id}: {}",
                String::from_utf8_lossy(&body)
            ),
        ));
    }));
    if caught.is_err() {
        let state = unsafe { &*(user_data as *const HostState) };
        state.host_callback_panics.fetch_add(1, SeqCst);
    }
}

unsafe extern "C" fn host_emit_event(
    user_data: *mut c_void,
    topic: *const u8,
    topic_len: usize,
    payload: MetisBuffer,
) {
    let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { &*(user_data as *const HostState) };
        state.event_callbacks.fetch_add(1, SeqCst);
        // SAFETY: the topic is borrowed from the plugin for the duration of the
        // call (D6).
        let topic = if topic.is_null() {
            String::new()
        } else {
            String::from_utf8_lossy(unsafe { slice::from_raw_parts(topic, topic_len) }).into_owned()
        };
        let body = state.read_and_release(payload).unwrap_or_default();
        state.logs.lock().unwrap().push((
            METIS_LOG_INFO,
            format!("host: event {topic}: {}", String::from_utf8_lossy(&body)),
        ));
    }));
    if caught.is_err() {
        let state = unsafe { &*(user_data as *const HostState) };
        state.host_callback_panics.fetch_add(1, SeqCst);
    }
}

unsafe extern "C" fn host_log(
    user_data: *mut c_void,
    level: i32,
    message: *const u8,
    message_len: usize,
) {
    let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { &*(user_data as *const HostState) };
        // A log message is borrowed, not allocated, so there is nothing to
        // release (D6).
        let message = if message.is_null() {
            String::new()
        } else {
            String::from_utf8_lossy(unsafe { slice::from_raw_parts(message, message_len) })
                .into_owned()
        };
        state.logs.lock().unwrap().push((level, message));
    }));
    if caught.is_err() {
        let state = unsafe { &*(user_data as *const HostState) };
        state.host_callback_panics.fetch_add(1, SeqCst);
    }
}

// --- calls through the table ----------------------------------------------

unsafe fn call_invoke(
    state: &HostState,
    request_id: u64,
    capability: &str,
    payload: MetisBuffer,
    out_payload: *mut MetisBuffer,
) -> MetisStatus {
    // SAFETY: the table comes from the loaded plugin and the instance was
    // created by it.
    let table = unsafe { &*state.plugin };
    let invoke = table.invoke;
    unsafe {
        invoke(
            state.instance.load(SeqCst),
            capability.as_ptr(),
            capability.len(),
            request_id,
            payload,
            state.host_api(),
            out_payload,
        )
    }
}

unsafe fn call_cancel(state: &HostState, request_id: u64) -> MetisStatus {
    let table = unsafe { &*state.plugin };
    let cancel = table.cancel;
    unsafe { cancel(state.instance.load(SeqCst), request_id) }
}

unsafe fn call_shutdown(state: &HostState) -> MetisStatus {
    let table = unsafe { &*state.plugin };
    let shutdown = table.shutdown;
    unsafe { shutdown(state.instance.load(SeqCst)) }
}

unsafe fn call_destroy(state: &HostState) -> MetisStatus {
    let table = unsafe { &*state.plugin };
    let destroy = table.destroy_instance;
    unsafe { destroy(state.instance.load(SeqCst)) }
}
