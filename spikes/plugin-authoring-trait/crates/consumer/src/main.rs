//! Consumer two: loads the plugin cdylib and drives requests through the host
//! adapter.
//!
//! Every request goes through the pending path: `invoke` returns pending, the
//! plugin completes through the host callback, and the awaiting task here
//! resolves. The inline completion is deliberately not exercised.

mod checks;

use std::ffi::c_void;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use checks::Checks;
use libloading::Library;
use spike_abi::*;
use spike_authoring::executor::block_on;
use spike_authoring::host_adapter::HostAdapter;
use spike_authoring::payload::{
    CapabilityKind, CapabilityList, CompletionRequest, CompletionResponse, EchoRequest, EchoResponse,
};
use spike_authoring::plugin_adapter::{
    CAPABILITY_COMPLETE, CAPABILITY_ECHO, CAPABILITY_UNCANCELLABLE,
};

/// Bound on the whole run: an unbounded wait on a completion that never comes
/// would hang forever (mechanics spike finding).
const WATCHDOG: Duration = Duration::from_secs(30);
/// How long the checks that expect nothing to arrive are willing to wait.
const NO_COMPLETION: Duration = Duration::from_millis(600);

fn main() -> ExitCode {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        eprintln!("spike-trait: watchdog fired after {WATCHDOG:?}: a completion never arrived");
        std::process::exit(2);
    });

    let mut checks = Checks::new();
    let path = dylib_path();
    if !path.exists() {
        eprintln!(
            "plugin not found at {}\nbuild the spike first: devenv shell -- cargo build --manifest-path spikes/plugin-authoring-trait/Cargo.toml",
            path.display()
        );
        return ExitCode::from(2);
    }
    println!("SPIKE_TRAIT_PLUGIN {}", path.display());

    // SAFETY: this is the spike's own cdylib, whose entry point matches the
    // frozen signature (D8).
    let library = unsafe { Library::new(&path) }.expect("loading the plugin cdylib");
    let entry: libloading::Symbol<unsafe extern "C" fn() -> *const MetisPluginTableV1> =
        unsafe { library.get(METIS_PLUGIN_ENTRY_V1) }.expect("resolving metis_plugin_entry_v1");
    let table_ptr = unsafe { entry() };
    checks.check("entry point returns a table", !table_ptr.is_null());
    if table_ptr.is_null() {
        return checks.finish();
    }
    let prefix = unsafe { metis_read_prefix(table_ptr) }.expect("table prefix");
    checks.eq("table size matches the Rust layout", prefix.size, std::mem::size_of::<MetisPluginTableV1>());
    checks.eq("table carries the ABI major", prefix.abi_major, METIS_ABI_MAJOR);
    let table = unsafe { &*table_ptr };

    let adapter = HostAdapter::new();
    adapter.attach(table_ptr);

    // --- instance ----------------------------------------------------------

    let mut instance: *mut c_void = std::ptr::null_mut();
    let status = unsafe { (table.create_instance)(adapter.api(), &mut instance) };
    checks.check("create_instance returns ok", status == MetisStatus::Ok);
    checks.check("instance handle is opaque and non-null", !instance.is_null());

    // --- capability discovery (D9) ----------------------------------------

    let mut out = MetisBuffer::empty();
    let status = unsafe { (table.list_capabilities)(instance, &mut out) };
    checks.check("list_capabilities returns ok", status == MetisStatus::Ok);
    let list: CapabilityList =
        serde_json::from_slice(&adapter.read_and_release(out).unwrap_or_default())
            .expect("capability list payload");
    let mut names: Vec<&str> = list.capabilities.iter().map(|entry| entry.name.as_str()).collect();
    names.sort();
    checks.eq(
        "descriptors name the three trait methods",
        names.join(","),
        format!("{CAPABILITY_COMPLETE},{CAPABILITY_ECHO},{CAPABILITY_UNCANCELLABLE}"),
    );
    let provider = list.capabilities.iter().find(|entry| entry.name == CAPABILITY_COMPLETE);
    checks.check(
        "provider.complete is declared as a provider",
        provider.is_some_and(|entry| entry.kind == CapabilityKind::Provider && entry.version == 1),
    );

    // --- tool.echo: the barebones async path, end to end -------------------

    let echo_request =
        serde_json::to_vec(&EchoRequest { echo: "hi".to_string(), repeat: 3 }).expect("request");
    let completion = adapter.register(1);
    let status = unsafe { adapter.invoke(instance, CAPABILITY_ECHO, 1, &echo_request) };
    checks.check("invoke returns pending, never inline", status == MetisStatus::Pending);
    let done = block_on(completion);
    checks.check("the awaiting task resolved", done.status == MetisStatus::Ok);
    let echo: EchoResponse = serde_json::from_slice(&done.body).unwrap_or(EchoResponse { echo: String::new(), bytes: 0 });
    checks.eq("echo body came through the callback", echo.echo, "hihihi".to_string());
    checks.eq("echo length came through the callback", echo.bytes, 6usize);
    checks.eq("the completion found the pending record", adapter.late_completions(), 0usize);

    // --- provider.complete: progress, then completion (D11, D12) -----------

    let provider_request =
        serde_json::to_vec(&CompletionRequest { prompt: "hello".to_string(), max_tokens: 8 })
            .expect("request");
    let completion = adapter.register(2);
    let status = unsafe { adapter.invoke(instance, CAPABILITY_COMPLETE, 2, &provider_request) };
    checks.check("provider invoke returns pending", status == MetisStatus::Pending);
    let done = block_on(completion);
    checks.check("provider completion ok", done.status == MetisStatus::Ok);
    let answer: CompletionResponse =
        serde_json::from_slice(&done.body).unwrap_or(CompletionResponse { text: String::new() });
    checks.eq("provider body", answer.text, "hello (8)".to_string());
    checks.eq("progress callback fired once", adapter.progress_callbacks(), 1usize);

    // --- cancel: the plugin suppresses its completion (D12) ----------------

    let _pending = adapter.register(3);
    let status = unsafe { adapter.invoke(instance, CAPABILITY_COMPLETE, 3, &provider_request) };
    checks.check("cancellable request is pending", status == MetisStatus::Pending);
    checks.check("cancel accepted", unsafe { adapter.cancel(instance, 3) } == MetisStatus::Ok);
    checks.check(
        "plugin suppressed the completion for cancelled request 3",
        wait_until(NO_COMPLETION, || adapter.saw_log("completion suppressed for cancelled request 3")),
    );
    checks.eq("nothing reached the host for request 3", adapter.late_completions(), 0usize);

    // --- cancel: the plugin completes anyway, the host drops it (D12) ------

    let uncancellable_request =
        serde_json::to_vec(&EchoRequest { echo: "stubborn".to_string(), repeat: 1 }).expect("request");
    let _pending = adapter.register(4);
    let status = unsafe { adapter.invoke(instance, CAPABILITY_UNCANCELLABLE, 4, &uncancellable_request) };
    checks.check("uncancellable request is pending", status == MetisStatus::Pending);
    checks.check("cancel accepted for the uncancellable request", unsafe { adapter.cancel(instance, 4) } == MetisStatus::Ok);
    checks.check(
        "host dropped the late completion",
        wait_until(NO_COMPLETION, || adapter.late_completions() == 1),
    );
    checks.check(
        "the dropped completion still released its buffer",
        adapter.saw_log("dropped a late completion for 4 (released=true)"),
    );

    // --- inline rejections, which never register a pending record ---------

    let status = unsafe { adapter.invoke(instance, "tool.missing", 5, &echo_request) };
    checks.check("unknown capability returns not_found", status == MetisStatus::NotFound);
    checks.check("no pending record for the unknown capability", unsafe { adapter.cancel(instance, 5) } == MetisStatus::NotFound);
    let status = unsafe { adapter.invoke(instance, CAPABILITY_ECHO, 6, b"not json") };
    checks.check("malformed request returns invalid_argument", status == MetisStatus::InvalidArgument);
    checks.check("cancel of an unknown request returns not_found", unsafe { adapter.cancel(instance, 999) } == MetisStatus::NotFound);

    // --- the instance survives all of that --------------------------------

    let completion = adapter.register(7);
    let status = unsafe { adapter.invoke(instance, CAPABILITY_ECHO, 7, &echo_request) };
    checks.check("echo is still pending after the cancellations", status == MetisStatus::Pending);
    let done = block_on(completion);
    checks.check("instance is still healthy", done.status == MetisStatus::Ok);

    // --- accounting on both sides (D6) ------------------------------------

    checks.eq("invoke never completed inline", adapter.inline_returns(), 0usize);
    checks.eq("every accepted request returned pending", adapter.pending_returns(), 5usize);
    checks.eq("a pending return never carried a buffer", adapter.pending_with_buffer(), 0usize);
    checks.eq("no host callback panicked", adapter.callback_panics(), 0usize);
    checks.eq("no event was emitted (the trait has no lifecycle hook)", adapter.event_callbacks(), 0usize);

    checks.check("shutdown returns ok", unsafe { (table.shutdown)(instance) } == MetisStatus::Ok);
    let account = adapter.account();
    checks.check("the plugin reported its accounting through the host log", account.is_some());
    if let Some(account) = account {
        checks.eq("plugin allocations equal plugin frees", account.get("allocs"), account.get("frees"));
        checks.eq("no plugin buffer is still live", account.get("live"), Some(&0usize));
        checks.eq("no buffer ownership violation", account.get("violations"), Some(&0usize));
        checks.eq("exactly one completion was suppressed", account.get("suppressed"), Some(&1usize));
        checks.eq("the host released exactly what the plugin allocated", adapter.released(), *account.get("allocs").unwrap_or(&0));
    }
    checks.check("destroy_instance returns ok", unsafe { (table.destroy_instance)(instance) } == MetisStatus::Ok);

    drop(library);
    checks.finish()
}

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if predicate() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn dylib_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("SPIKE_TRAIT_PLUGIN_PATH") {
        return PathBuf::from(explicit);
    }
    let exe = std::env::current_exe().expect("current_exe");
    let name = if cfg!(target_os = "macos") { "libspike_trait_plugin.dylib" } else { "libspike_trait_plugin.so" };
    exe.parent().expect("target dir").join(name)
}
