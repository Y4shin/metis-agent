//! Spike-only host.
//!
//! Loads the cdylib plugin with `libloading`, exercises every primitive in the
//! `plugin-abi-layout` decisions, prints a PASS/FAIL table and exits non-zero
//! when anything failed.

mod appended;
mod checks;

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::process::ExitCode;
use std::slice;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering::SeqCst};
use std::sync::{Condvar, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use appended::{MetisPluginTableNext, MetisTablePrefixOldHost, read_appended};
use checks::Checks;
use libloading::Library;
use spike_abi::*;
use spike_payload::{CapabilityDescriptor, CapabilityKind, CapabilityList, EchoRequest, EchoResponse};

/// Highest per-capability version this host understands (D5, D10).
const HOST_CAPABILITY_VERSION_MAX: u32 = 1;

static HOST_CALLBACK_PANICS: AtomicUsize = AtomicUsize::new(0);

// --- host-side request buffers (D6: host allocates, host frees) ------------

struct HostRequest<'a> {
    state: &'a HostState,
    bytes: Vec<u8>,
}

impl<'a> HostRequest<'a> {
    fn new(state: &'a HostState, body: &[u8]) -> Self {
        state.host_allocs.fetch_add(1, SeqCst);
        HostRequest { state, bytes: metis_encode_payload(body) }
    }

    fn raw(state: &'a HostState, header: u8, body: &[u8]) -> Self {
        state.host_allocs.fetch_add(1, SeqCst);
        HostRequest { state, bytes: metis_encode_payload_with(header, body) }
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
        // The host allocated it, so the host frees it, after the call returned
        // (D6). Nothing here ever hands this pointer to `release_buffer`.
        self.state.host_frees.fetch_add(1, SeqCst);
    }
}

// --- host state ------------------------------------------------------------

struct HostState {
    /// Borrowed from the plugin; valid while the library stays loaded.
    plugin: *const MetisPluginTableV1,
    host_api: AtomicPtr<MetisHostApiV1>,
    instance: AtomicPtr<c_void>,

    pending: Mutex<HashSet<u64>>,
    completed: Mutex<HashMap<u64, Completed>>,
    cancelled: Mutex<HashSet<u64>>,
    queue_cv: Condvar,

    host_allocs: AtomicUsize,
    host_frees: AtomicUsize,
    plugin_buffers_released: AtomicUsize,
    foreign_free_attempts: AtomicUsize,
    completion_callbacks: AtomicUsize,
    progress_callbacks: AtomicUsize,
    event_callbacks: AtomicUsize,
    late_completions_dropped: AtomicUsize,
    late_dropped_buffers_released: AtomicUsize,

    logs: Mutex<Vec<(i32, String)>>,
    rejections: Mutex<Vec<String>>,
}

struct Completed {
    status: MetisStatus,
    body: Vec<u8>,
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
            queue_cv: Condvar::new(),
            host_allocs: AtomicUsize::new(0),
            host_frees: AtomicUsize::new(0),
            plugin_buffers_released: AtomicUsize::new(0),
            foreign_free_attempts: AtomicUsize::new(0),
            completion_callbacks: AtomicUsize::new(0),
            progress_callbacks: AtomicUsize::new(0),
            event_callbacks: AtomicUsize::new(0),
            late_completions_dropped: AtomicUsize::new(0),
            late_dropped_buffers_released: AtomicUsize::new(0),
            logs: Mutex::new(Vec::new()),
            rejections: Mutex::new(Vec::new()),
        }
    }

    fn host_api(&self) -> *const MetisHostApiV1 {
        self.host_api.load(SeqCst)
    }

    fn register_pending(&self, request_id: u64) {
        // D11: pending state is registered before the plugin is called.
        self.pending.lock().unwrap().insert(request_id);
    }

    fn release_plugin_buffer(&self, buffer: MetisBuffer) -> MetisStatus {
        if buffer.owner != METIS_BUFFER_OWNER_PLUGIN {
            self.foreign_free_attempts.fetch_add(1, SeqCst);
            self.rejections
                .lock()
                .unwrap()
                .push(format!("buffer with owner={} offered to release_buffer", buffer.owner));
            return MetisStatus::InvalidArgument;
        }
        if buffer.ptr.is_null() {
            return MetisStatus::Ok;
        }
        // SAFETY: `plugin` outlives the state; the table is a plugin static.
        let table = unsafe { &*self.plugin };
        let release = table.release_buffer;
        let status = unsafe { release(buffer) };
        if status == MetisStatus::Ok {
            self.plugin_buffers_released.fetch_add(1, SeqCst);
        } else {
            self.rejections.lock().unwrap().push(format!("release_buffer returned {}", status.as_str()));
        }
        status
    }

    /// Copies a plugin payload out, validating the header, then releases the
    /// buffer. Used for inline results and event payloads.
    fn read_and_release(&self, buffer: MetisBuffer) -> Result<Vec<u8>, String> {
        if buffer.ptr.is_null() {
            return Err("null buffer from the plugin".to_string());
        }
        // SAFETY: the plugin allocated this buffer with length `len`.
        let bytes = unsafe { slice::from_raw_parts(buffer.ptr, buffer.len) };
        let body = match metis_payload_body(bytes) {
            Ok(body) => body.to_vec(),
            Err(status) => {
                self.release_plugin_buffer(buffer);
                return Err(format!("payload header rejected with {}", status.as_str()));
            }
        };
        self.release_plugin_buffer(buffer);
        Ok(body)
    }

    fn wait_for_completion(&self, request_id: u64, timeout: Duration) -> Option<Completed> {
        let deadline = Instant::now() + timeout;
        let mut guard = self.completed.lock().unwrap();
        loop {
            if let Some(completed) = guard.remove(&request_id) {
                return Some(completed);
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            guard = self.queue_cv.wait_timeout(guard, deadline - now).unwrap().0;
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
            sleep(Duration::from_millis(10));
        }
    }

    fn log_lines(&self) -> Vec<String> {
        self.logs.lock().unwrap().iter().map(|(_, line)| line.clone()).collect()
    }

    fn saw_log(&self, needle: &str) -> bool {
        self.log_lines().iter().any(|line| line.contains(needle))
    }

    fn rejections(&self) -> Vec<String> {
        self.rejections.lock().unwrap().clone()
    }

    fn account(&self) -> Option<HashMap<String, usize>> {
        let line = self.log_lines().into_iter().find(|line| line.contains("SPIKE_ACCOUNT"))?;
        let mut fields = HashMap::new();
        for token in line.split_whitespace() {
            if let Some((key, value)) = token.split_once('=') {
                fields.insert(key.to_string(), value.parse().unwrap_or(usize::MAX));
            }
        }
        Some(fields)
    }
}

// --- host callbacks (D12: any plugin thread; also a boundary, so catch) ----

unsafe extern "C" fn host_complete(user_data: *mut c_void, request_id: u64, status: MetisStatus, payload: MetisBuffer) {
    let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user_data` is the boxed HostState for the whole run.
        let state = unsafe { &*(user_data as *const HostState) };
        state.completion_callbacks.fetch_add(1, SeqCst);
        state.pending.lock().unwrap().remove(&request_id);

        let body = if payload.ptr.is_null() {
            Vec::new()
        } else {
            // SAFETY: the plugin allocated this buffer; it stays valid for the
            // call and is released below.
            let bytes = unsafe { slice::from_raw_parts(payload.ptr, payload.len) };
            match metis_payload_body(bytes) {
                Ok(body) => body.to_vec(),
                Err(bad) => {
                    state
                        .rejections
                        .lock()
                        .unwrap()
                        .push(format!("completion payload header rejected with {}", bad.as_str()));
                    Vec::new()
                }
            }
        };

        // A cancel that already succeeded makes this completion a protocol
        // afterthought (D12), but the buffer is still plugin memory that only
        // the plugin may free (D6).
        let late = state.cancelled.lock().unwrap().contains(&request_id);
        if late {
            state.late_completions_dropped.fetch_add(1, SeqCst);
        }
        let released = state.release_plugin_buffer(payload);
        if late {
            if released == MetisStatus::Ok {
                state.late_dropped_buffers_released.fetch_add(1, SeqCst);
            }
            return;
        }
        state.completed.lock().unwrap().insert(request_id, Completed { status, body });
        state.queue_cv.notify_all();
    }));
    if caught.is_err() {
        HOST_CALLBACK_PANICS.fetch_add(1, SeqCst);
    }
}

unsafe extern "C" fn host_progress(user_data: *mut c_void, request_id: u64, payload: MetisBuffer) {
    let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { &*(user_data as *const HostState) };
        state.progress_callbacks.fetch_add(1, SeqCst);
        let body = if payload.ptr.is_null() {
            Vec::new()
        } else {
            let bytes = unsafe { slice::from_raw_parts(payload.ptr, payload.len) };
            metis_payload_body(bytes).map(|body| body.to_vec()).unwrap_or_default()
        };
        state
            .logs
            .lock()
            .unwrap()
            .push((METIS_LOG_INFO, format!("host: progress {request_id} {}", String::from_utf8_lossy(&body))));
        state.release_plugin_buffer(payload);
    }));
    if caught.is_err() {
        HOST_CALLBACK_PANICS.fetch_add(1, SeqCst);
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
        let topic = if topic.is_null() {
            String::new()
        } else {
            String::from_utf8_lossy(unsafe { slice::from_raw_parts(topic, topic_len) }).into_owned()
        };
        let body = state.read_and_release(payload).unwrap_or_default();
        state.logs.lock().unwrap().push((
            METIS_LOG_INFO,
            format!("host: event {topic} {}", String::from_utf8_lossy(&body)),
        ));
    }));
    if caught.is_err() {
        HOST_CALLBACK_PANICS.fetch_add(1, SeqCst);
    }
}

unsafe extern "C" fn host_log(user_data: *mut c_void, level: i32, message: *const u8, message_len: usize) {
    let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { &*(user_data as *const HostState) };
        let message = if message.is_null() {
            String::new()
        } else {
            String::from_utf8_lossy(unsafe { slice::from_raw_parts(message, message_len) }).into_owned()
        };
        state.logs.lock().unwrap().push((level, message));
    }));
    if caught.is_err() {
        HOST_CALLBACK_PANICS.fetch_add(1, SeqCst);
    }
}

// --- calls through the table ----------------------------------------------

unsafe fn call_invoke(
    state: &HostState,
    request_id: u64,
    capability: &str,
    payload: MetisBuffer,
    out: *mut MetisBuffer,
) -> MetisStatus {
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
            out,
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

fn dylib_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("SPIKE_PLUGIN_PATH") {
        return PathBuf::from(explicit);
    }
    let exe = std::env::current_exe().expect("current_exe");
    let name = if cfg!(target_os = "macos") { "libspike_plugin.dylib" } else { "libspike_plugin.so" };
    exe.parent().expect("target dir").join(name)
}

fn main() -> ExitCode {
    let mut checks = Checks::new();
    let path = dylib_path();
    if !path.exists() {
        eprintln!(
            "plugin not found at {}\nbuild the spike first: devenv shell -- cargo build --manifest-path spikes/plugin-abi-mechanics/Cargo.toml",
            path.display()
        );
        return ExitCode::FAILURE;
    }
    println!("host: loading {}", path.display());

    // SAFETY: the spike controls the plugin it builds. The library is kept
    // alive until after `shutdown` has joined every plugin worker thread.
    let library = match unsafe { Library::new(&path) } {
        Ok(library) => library,
        Err(err) => {
            eprintln!("host: dlopen failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    // 1. symbol resolution (D8).
    let entry = unsafe { library.get::<MetisPluginEntryV1Fn>(METIS_PLUGIN_ENTRY_V1) };
    let table_ptr = match &entry {
        Ok(entry) => unsafe { entry() },
        Err(err) => {
            checks.check(false, "symbol.entry_point_resolved", format!("{err}"));
            checks.report();
            return ExitCode::FAILURE;
        }
    };
    checks.check(!table_ptr.is_null(), "symbol.entry_point_resolved", "metis_plugin_entry_v1 returned a table");

    // 2. a differing major fails at resolution, before any struct is read.
    let wrong_major = unsafe { library.get::<MetisPluginEntryV1Fn>(b"metis_plugin_entry_v2\0") };
    checks.check(
        wrong_major.is_err(),
        "symbol.wrong_major_absent",
        format!("metis_plugin_entry_v2 lookup: {:?}", wrong_major.err().map(|err| err.to_string())),
    );

    // 3. the reported size and version, read through the shortest prefix.
    let prefix = unsafe { metis_read_prefix(table_ptr) };
    match prefix {
        Some(prefix) => {
            checks.check(
                prefix.size == mem::size_of::<MetisPluginTableV1>(),
                "table.size_matches_layout",
                format!("plugin reports size={} rustc size={}", prefix.size, mem::size_of::<MetisPluginTableV1>()),
            );
            checks.check(
                prefix.abi_major == METIS_ABI_MAJOR,
                "table.major_is_1",
                format!("abi_major={} abi_minor={}", prefix.abi_major, prefix.abi_minor),
            );
        }
        None => {
            checks.check(false, "table.size_matches_layout", "prefix unreadable");
            checks.check(false, "table.major_is_1", "prefix unreadable");
        }
    }

    // 4. a table that claims to be shorter than the prefix must be rejected,
    //    not read past its end.
    #[repr(C)]
    struct ShortTable {
        size: usize,
        abi_major: u32,
        abi_minor: u32,
    }
    let short = ShortTable { size: mem::size_of::<usize>(), abi_major: METIS_ABI_MAJOR, abi_minor: 0 };
    checks.check(
        unsafe { metis_read_prefix(&short as *const ShortTable as *const MetisPluginTableV1) }.is_none(),
        "table.short_table_rejected",
        "size=8 prefix read returns None",
    );

    // 5. an older host reads only the prefix it knows.
    let old_host_prefix = unsafe { &*(table_ptr as *const MetisTablePrefixOldHost) };
    let prefix_ok = old_host_prefix.size >= mem::size_of::<MetisTablePrefixOldHost>()
        && matches!(
            (old_host_prefix.create_instance, unsafe { (*table_ptr).create_instance }),
            (a, b) if std::ptr::fn_addr_eq(a, b)
        )
        && matches!(
            (old_host_prefix.destroy_instance, unsafe { (*table_ptr).destroy_instance }),
            (a, b) if std::ptr::fn_addr_eq(a, b)
        );
    checks.check(
        prefix_ok,
        "table.prefix_read_matches_full",
        "older host's 40-byte prefix agrees with the full v1 table",
    );

    // 6. a newer host asking for an appended entry gets nothing from an older
    //    plugin, and does not read past the reported size.
    let older_plugin_appended = unsafe { read_appended(table_ptr) };
    checks.check(
        older_plugin_appended.is_none(),
        "table.append_only.newer_host_older_plugin",
        format!(
            "plugin size={} appended size={}, extra entry not read",
            unsafe { (*table_ptr).size },
            mem::size_of::<MetisPluginTableNext>()
        ),
    );

    // 7. the same newer host against a plugin that does provide it.
    unsafe extern "C" fn stub_set_log_level(level: i32) -> MetisStatus {
        println!("host: appended entry called with level={level}");
        MetisStatus::Ok
    }
    let newer = MetisPluginTableNext {
        v1: MetisPluginTableV1 {
            size: mem::size_of::<MetisPluginTableNext>(),
            abi_major: METIS_ABI_MAJOR,
            abi_minor: METIS_ABI_MINOR + 1,
            create_instance: unsafe { (*table_ptr).create_instance },
            destroy_instance: unsafe { (*table_ptr).destroy_instance },
            list_capabilities: unsafe { (*table_ptr).list_capabilities },
            invoke: unsafe { (*table_ptr).invoke },
            cancel: unsafe { (*table_ptr).cancel },
            release_buffer: unsafe { (*table_ptr).release_buffer },
            shutdown: unsafe { (*table_ptr).shutdown },
        },
        set_log_level: stub_set_log_level,
    };
    let appended = unsafe { read_appended(&newer as *const MetisPluginTableNext as *const MetisPluginTableV1) };
    let appended_ok = match appended {
        Some(set_log_level) => unsafe { set_log_level(1) == MetisStatus::Ok },
        None => false,
    };
    checks.check(
        appended_ok,
        "table.append_only.newer_plugin_newer_host",
        "appended entry found and called when size covers it",
    );

    // 8. the payload header round trip (D4).
    let body = br#"{"echo":"hi"}"#;
    let encoded = metis_encode_payload(body);
    let round_trip_ok = encoded[0] == 0x01
        && metis_payload_format_version(encoded[0]) == METIS_PAYLOAD_FORMAT_VERSION
        && metis_payload_codec(encoded[0]) == METIS_CODEC_JSON
        && metis_payload_reserved(encoded[0]) == 0
        && metis_payload_body(&encoded).map(|body| body == br#"{"echo":"hi"}"#).unwrap_or(false);
    checks.check(
        round_trip_ok,
        "payload.roundtrip",
        format!("header byte {:#04x}, body preserved", encoded[0]),
    );
    checks.check(
        metis_payload_header(0x0f, METIS_CODEC_MESSAGEPACK) == 0x1f,
        "payload.header_bit_layout",
        "codec bit is bit 4, version nibble is low four bits",
    );

    // --- instance and host table -------------------------------------------

    let state = Box::new(HostState::new(table_ptr));
    let host_api = Box::new(MetisHostApiV1 {
        size: mem::size_of::<MetisHostApiV1>(),
        user_data: &*state as *const HostState as *mut c_void,
        complete: host_complete,
        progress: host_progress,
        emit_event: host_emit_event,
        log: host_log,
    });
    state.host_api.store(&*host_api as *const MetisHostApiV1 as *mut MetisHostApiV1, SeqCst);

    let mut instance: *mut c_void = std::ptr::null_mut();
    let create = unsafe { (*table_ptr).create_instance };
    let create_status = unsafe { create(state.host_api(), &mut instance) };
    state.instance.store(instance, SeqCst);
    checks.check(
        create_status == MetisStatus::Ok && !instance.is_null(),
        "instance.create_ok",
        format!("status={} instance={:p}", create_status.as_str(), instance),
    );
    checks.check(
        state.event_callbacks.load(SeqCst) == 1 && state.saw_log("spike.ready"),
        "callback.emit_event_from_create",
        format!("event callbacks={}", state.event_callbacks.load(SeqCst)),
    );

    // --- capability discovery (D9, D5, D10) --------------------------------

    let list_caps = unsafe { (*table_ptr).list_capabilities };
    let mut caps_buffer = MetisBuffer::empty();
    let caps_status = unsafe { list_caps(instance, &mut caps_buffer) };
    let descriptors: Vec<CapabilityDescriptor> = match state.read_and_release(caps_buffer) {
        Ok(body) => serde_json::from_slice::<CapabilityList>(&body).map(|list| list.capabilities).unwrap_or_default(),
        Err(err) => {
            checks.check(false, "capabilities.listed", err);
            Vec::new()
        }
    };
    checks.check(
        caps_status == MetisStatus::Ok && descriptors.len() == 6,
        "capabilities.listed",
        format!(
            "status={} count={} names={:?}",
            caps_status.as_str(),
            descriptors.len(),
            descriptors.iter().map(|d| d.name.as_str()).collect::<Vec<_>>()
        ),
    );
    let accepted = descriptors
        .iter()
        .filter(|descriptor| descriptor.kind == CapabilityKind::Tool && descriptor.version <= HOST_CAPABILITY_VERSION_MAX)
        .count();
    let refused: Vec<&str> = descriptors
        .iter()
        .filter(|descriptor| descriptor.version > HOST_CAPABILITY_VERSION_MAX)
        .map(|descriptor| descriptor.name.as_str())
        .collect();
    checks.check(
        accepted == 5 && refused == vec!["tool.future"],
        "capabilities.version_gate",
        format!("accepted={accepted} refused={refused:?} (plugin still usable)"),
    );

    // --- payload header rejections from the plugin side --------------------

    for (label, name, header, expected) in [
        ("payload.reserved_bits_rejected", "tool.echo", 0x81_u8 | 0x01, MetisStatus::InvalidArgument),
        ("payload.bad_format_version_rejected", "tool.echo", 0x02, MetisStatus::InvalidArgument),
        ("payload.messagepack_codec_refused", "tool.echo", 0x11, MetisStatus::Unsupported),
    ] {
        let request = HostRequest::raw(&state, header, body);
        let mut out = MetisBuffer::empty();
        let status = unsafe { call_invoke(&state, 900, name, request.buffer(), &mut out) };
        checks.check(
            status == expected,
            label,
            format!("header {header:#04x} -> {} (expected {})", status.as_str(), expected.as_str()),
        );
    }

    // --- inline invoke (D11) -----------------------------------------------

    let echo_request = serde_json::to_vec(&EchoRequest { echo: "hi".to_string(), repeat: 2 }).unwrap();
    let request = HostRequest::new(&state, &echo_request);
    let mut out = MetisBuffer::empty();
    let status = unsafe { call_invoke(&state, 1, "tool.echo", request.buffer(), &mut out) };
    let inline_body = state.read_and_release(out);
    let parsed: Option<EchoResponse> = inline_body.as_ref().ok().and_then(|body| serde_json::from_slice(body).ok());
    checks.check(
        status == MetisStatus::Ok
            && matches!(&parsed, Some(response) if response.echo == "hihi" && response.bytes == 4),
        "invoke.inline_result",
        format!("status={} body={:?}", status.as_str(), parsed),
    );
    drop(request);

    let request = HostRequest::new(&state, &serde_json::to_vec(&EchoRequest { echo: "x".into(), repeat: 1 }).unwrap());
    let mut out = MetisBuffer::empty();
    let status = unsafe { call_invoke(&state, 2, "tool.nope", request.buffer(), &mut out) };
    drop(request);
    checks.check(
        status == MetisStatus::NotFound,
        "invoke.unknown_capability_not_found",
        format!("status={}", status.as_str()),
    );

    // --- pending invoke completed through the host callback (D11, D12) -----

    let request = HostRequest::new(&state, &serde_json::to_vec(&EchoRequest { echo: "s".into(), repeat: 1 }).unwrap());
    let mut out = MetisBuffer::empty();
    state.register_pending(3);
    let status = unsafe { call_invoke(&state, 3, "tool.slow", request.buffer(), &mut out) };
    drop(request);
    checks.check(
        status == MetisStatus::Pending && out.ptr.is_null(),
        "invoke.pending_returned",
        format!("status={} out is null", status.as_str()),
    );
    let completed = state.wait_for_completion(3, Duration::from_secs(5));
    checks.check(
        matches!(&completed, Some(completed) if completed.status == MetisStatus::Ok && completed.body == br#"{"echo":"async","bytes":5}"#),
        "invoke.pending_completion_via_callback",
        format!(
            "callback arrived={} body={:?}",
            completed.is_some(),
            completed.as_ref().map(|c| String::from_utf8_lossy(&c.body).into_owned())
        ),
    );
    checks.check(
        state.progress_callbacks.load(SeqCst) == 1,
        "invoke.pending_progress_callback",
        format!("progress callbacks={}", state.progress_callbacks.load(SeqCst)),
    );

    // --- cancel (D11, D12) -------------------------------------------------

    let request = HostRequest::new(&state, &serde_json::to_vec(&EchoRequest { echo: "c".into(), repeat: 1 }).unwrap());
    let mut out = MetisBuffer::empty();
    state.register_pending(4);
    let status = unsafe { call_invoke(&state, 4, "tool.cancelme", request.buffer(), &mut out) };
    drop(request);
    checks.check(status == MetisStatus::Pending, "cancel.pending_before_cancel", format!("status={}", status.as_str()));
    sleep(Duration::from_millis(30));
    let cancel_status = unsafe { call_cancel(&state, 4) };
    state.cancelled.lock().unwrap().insert(4);
    // No completion is guaranteed after a successful cancel, so the host must
    // retire its own pending entry (D12); otherwise it leaks.
    state.pending.lock().unwrap().remove(&4);
    checks.check(
        cancel_status == MetisStatus::Ok,
        "cancel.accepted",
        format!("status={}", cancel_status.as_str()),
    );
    let suppressed = state.wait_until(Duration::from_secs(5), |state| {
        state.saw_log("dropped completion for cancelled request 4")
    });
    checks.check(
        suppressed && !state.completed.lock().unwrap().contains_key(&4),
        "cancel.plugin_suppressed_completion",
        format!("plugin logged the drop={suppressed}, no completion delivered"),
    );
    let unknown_cancel = unsafe { call_cancel(&state, 4242) };
    checks.check(
        unknown_cancel == MetisStatus::NotFound,
        "cancel.unknown_request_not_found",
        format!("status={}", unknown_cancel.as_str()),
    );

    // A plugin that completes anyway: the host drops it and still releases the
    // buffer, so cancellation cannot leak plugin memory.
    let request = HostRequest::new(&state, &serde_json::to_vec(&EchoRequest { echo: "r".into(), repeat: 1 }).unwrap());
    let mut out = MetisBuffer::empty();
    state.register_pending(5);
    let status = unsafe { call_invoke(&state, 5, "tool.cancelrace", request.buffer(), &mut out) };
    drop(request);
    sleep(Duration::from_millis(20));
    let cancel_status = unsafe { call_cancel(&state, 5) };
    state.cancelled.lock().unwrap().insert(5);
    state.pending.lock().unwrap().remove(&5);
    let dropped = state.wait_until(Duration::from_secs(5), |state| {
        state.late_completions_dropped.load(SeqCst) == 1
    });
    checks.check(
        status == MetisStatus::Pending && cancel_status == MetisStatus::Ok && dropped,
        "cancel.host_dropped_late_completion",
        format!(
            "invoke={} cancel={} late completions dropped={}",
            status.as_str(),
            cancel_status.as_str(),
            state.late_completions_dropped.load(SeqCst)
        ),
    );
    checks.check(
        state.late_dropped_buffers_released.load(SeqCst) == 1 && !state.completed.lock().unwrap().contains_key(&5),
        "cancel.late_completion_buffer_released",
        format!(
            "dropped completion buffers released={}",
            state.late_dropped_buffers_released.load(SeqCst)
        ),
    );

    // --- panic converted to internal error (D7) ---------------------------

    let request = HostRequest::new(&state, &serde_json::to_vec(&EchoRequest { echo: "p".into(), repeat: 1 }).unwrap());
    let mut out = MetisBuffer::empty();
    let status = unsafe { call_invoke(&state, 6, "tool.panic", request.buffer(), &mut out) };
    drop(request);
    checks.check(
        status == MetisStatus::Internal && state.saw_log("panic caught at the boundary"),
        "panic.converted_to_internal_error",
        format!("status={} plugin logged the catch", status.as_str()),
    );

    let request = HostRequest::new(&state, &serde_json::to_vec(&EchoRequest { echo: "z".into(), repeat: 3 }).unwrap());
    let mut out = MetisBuffer::empty();
    let status = unsafe { call_invoke(&state, 7, "tool.echo", request.buffer(), &mut out) };
    let after_panic = state.read_and_release(out).ok().and_then(|body| serde_json::from_slice::<EchoResponse>(&body).ok());
    drop(request);
    checks.check(
        status == MetisStatus::Ok && matches!(&after_panic, Some(response) if response.echo == "zzz"),
        "panic.plugin_still_usable",
        format!("status={} body={:?}", status.as_str(), after_panic),
    );

    // --- inline versus callback-only completion ---------------------------
    //
    // The same trivial call on both paths: the inline path pays one null check
    // on the out parameter, the pending path pays a registry insert, a condvar
    // wait on the caller's side, and a thread hop plus a completion callback on
    // the plugin's side. Measured here rather than argued about.

    const ECHO_RUNS: u32 = 200;
    const PENDING_RETURN_RUNS: u64 = 50;
    let body = serde_json::to_vec(&EchoRequest { echo: "t".to_string(), repeat: 1 }).unwrap();
    let start = Instant::now();
    let mut inline_ok = 0;
    for _ in 0..ECHO_RUNS {
        let request = HostRequest::new(&state, &body);
        let mut out = MetisBuffer::empty();
        let status = unsafe { call_invoke(&state, 20_000, "tool.echo", request.buffer(), &mut out) };
        if status == MetisStatus::Ok && state.read_and_release(out).is_ok() {
            inline_ok += 1;
        }
        drop(request);
    }
    let inline_elapsed = start.elapsed();

    let start = Instant::now();
    let mut pending_ok = 0;
    for index in 0..PENDING_RETURN_RUNS {
        let request = HostRequest::new(&state, &body);
        let mut out = MetisBuffer::empty();
        let status = unsafe { call_invoke(&state, 30_000 + index, "tool.slow", request.buffer(), &mut out) };
        if status == MetisStatus::Pending {
            pending_ok += 1;
        }
        drop(request);
    }
    let pending_return_elapsed = start.elapsed();
    checks.check(
        inline_ok == ECHO_RUNS && pending_ok == PENDING_RETURN_RUNS,
        "invoke.inline_vs_pending_cost",
        format!(
            "inline: {} runs in {:?} total ({:?}/call); pending return: {} runs in {:?} total ({:?}/call);              pending completions and their buffers arrive while shutdown joins",
            ECHO_RUNS,
            inline_elapsed,
            inline_elapsed / ECHO_RUNS,
            PENDING_RETURN_RUNS,
            pending_return_elapsed,
            pending_return_elapsed / PENDING_RETURN_RUNS as u32
        ),
    );

    // --- accounting (D6) ---------------------------------------------------

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
        format!("second free of the handle -> {}", destroy_after_shutdown.as_str()),
    );

    let account = state.account();
    match &account {
        Some(fields) => {
            checks.check(
                fields.get("live") == Some(&0) && fields.get("allocs") == fields.get("frees"),
                "buffer.plugin_allocs_all_released",
                format!(
                    "plugin allocs={:?} frees={:?} live={:?}",
                    fields.get("allocs"), fields.get("frees"), fields.get("live")
                ),
            );
            checks.check(
                fields.get("violations") == Some(&0),
                "buffer.no_plugin_side_violations",
                format!("plugin-side violations={:?}", fields.get("violations")),
            );
        }
        None => {
            checks.check(false, "buffer.plugin_allocs_all_released", "no SPIKE_ACCOUNT log line");
            checks.check(false, "buffer.no_plugin_side_violations", "no SPIKE_ACCOUNT log line");
        }
    }
    checks.check(
        state.host_allocs.load(SeqCst) == state.host_frees.load(SeqCst) && state.host_allocs.load(SeqCst) > 0,
        "buffer.host_allocs_freed_by_host",
        format!(
            "host allocs={} frees={}",
            state.host_allocs.load(SeqCst),
            state.host_frees.load(SeqCst)
        ),
    );
    checks.check(
        state.foreign_free_attempts.load(SeqCst) == 0 && state.rejections().is_empty(),
        "buffer.no_foreign_or_bad_releases",
        format!(
            "foreign free attempts={} rejections={:?}",
            state.foreign_free_attempts.load(SeqCst),
            state.rejections()
        ),
    );
    // Read the length first: a second `lock()` inside the check's own argument
    // list deadlocks, because std's Mutex is not reentrant.
    let pending_left = state.pending.lock().unwrap().len();
    eprintln!("host: tail: pending registry");
    checks.check(pending_left == 0, "pending.registry_empty", format!("pending entries left={pending_left}"));
    checks.check(
        HOST_CALLBACK_PANICS.load(SeqCst) == 0,
        "callback.host_callbacks_no_panic",
        format!("host callback panics={}", HOST_CALLBACK_PANICS.load(SeqCst)),
    );

    eprintln!("host: tail: log dump");
    println!("host: plugin log lines:");
    for (level, line) in state.logs.lock().unwrap().iter() {
        println!("  [log:{level}] {line}");
    }

    // The library may only be unloaded once no plugin thread can call back.
    // `shutdown` joined every worker, so this is now safe; before that it would
    // have been a use-after-unload.
    eprintln!("host: tail: dropping state");
    drop(state);
    eprintln!("host: tail: unloading the library");
    drop(library);
    eprintln!("host: tail: library unloaded");

    checks.report();
    if checks.all_ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
