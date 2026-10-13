//! PROTOTYPE (throwaway). Not product code.
//!
//! The host binary. Prints one line per demonstrated behaviour, then a summary,
//! and exits non zero when any behaviour failed.
//!
//! Run it with the workspace built first:
//!
//! ```sh
//! devenv shell -- cargo build --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml
//! devenv shell -- cargo run   --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml -p proto-host
//! ```
//!
//! `cargo run -p proto-host` does not rebuild the cdylib member, so the build
//! has to come first or the host loads a stale library.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use plugin_contract::abi::MetisStatus;
use proto_host::abi_adapter::{AbiPlugin, OrderLedger};
use proto_host::native;
use proto_host::typed::{ProviderSet, Toolset};
use proto_host::types::{
    AddRequest, BadResponseRequest, CompleteRequest, EchoRequest, MismatchedEchoRequest,
    SummarizeRequest, UppercaseRequest, WordCountRequest,
};
use proto_registry::{
    Call, Payload, Plugin, PluginError, PluginLoader, PluginRegistry, RegistryError, RoutingMode,
    Status, VersionPolicy, block_on,
};
use serde_json::{Value, json};

struct Harness {
    checks: usize,
    failures: usize,
}

impl Harness {
    /// Flushes after every line, so a hang or a kill still leaves the marker
    /// for the group and check the run last reached on disk.
    fn flush() {
        let _ = std::io::stdout().flush();
    }

    fn check(&mut self, name: &str, ok: bool, detail: &str) {
        self.checks += 1;
        if ok {
            println!("PASS {name}");
        } else {
            self.failures += 1;
            println!("FAIL {name} :: {detail}");
        }
        Harness::flush();
    }

    fn note(&self, line: &str) {
        println!("NOTE {line}");
        Harness::flush();
    }

    fn section(&self, line: &str) {
        println!("\n== {line}");
        Harness::flush();
    }
}

impl proto_host::future_driver::CheckSink for Harness {
    fn check(&mut self, name: &str, ok: bool, detail: &str) {
        Harness::check(self, name, ok, detail);
    }

    fn note(&self, line: &str) {
        Harness::note(self, line);
    }

    fn section(&self, line: &str) {
        Harness::section(self, line);
    }

    fn bench(&self, line: &str) {
        println!("BENCH {line}");
        Harness::flush();
    }
}

fn main() -> ExitCode {
    let mut harness = Harness {
        checks: 0,
        failures: 0,
    };
    if let Err(setup) = drive(&mut harness) {
        harness.checks += 1;
        harness.failures += 1;
        println!("FAIL setup :: {setup}");
    }
    println!(
        "\n{} checks, {} failures",
        harness.checks, harness.failures
    );
    Harness::flush();
    if harness.failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

type Sink = Arc<Mutex<Vec<Arc<AbiPlugin>>>>;

fn make_loader(ledger: OrderLedger, sink: Sink, id: &'static str) -> PluginLoader {
    Arc::new(move |path: &Path| -> Result<Arc<dyn Plugin>, String> {
        let abi = AbiPlugin::load(path, id, Arc::clone(&ledger))?;
        let abi = Arc::new(abi);
        sink.lock().unwrap().push(Arc::clone(&abi));
        Ok(abi as Arc<dyn Plugin>)
    })
}

fn payload(value: Value) -> Payload {
    Payload::from_json(&value).expect("the literal is encodable")
}

/// One routed call through the registry, blocking on its completion.
fn invoke(registry: &PluginRegistry, key: &str, request: Value) -> Result<Payload, PluginError> {
    let call = Call::new();
    block_on(registry.invoke(key, payload(request), &call))
}

fn json_of(payload: &Payload) -> Value {
    payload.to_json().expect("the response is JSON")
}

/// Asks the plugin for its live buffer ledger.
fn accounting(abi: &AbiPlugin) -> Value {
    let body = abi
        .call_pending("metis.accounting", b"{}", Duration::from_secs(5))
        .expect("the accounting call answers");
    serde_json::from_slice(&body).expect("accounting is JSON")
}

/// Asks the plugin for its host-polled future counters, through the legacy
/// path. Used after the future path is done, for the timer round trip.
fn future_stats(abi: &AbiPlugin) -> Value {
    let body = abi
        .call_pending("metis.future_stats", b"{}", Duration::from_secs(5))
        .expect("the future stats call answers");
    serde_json::from_slice(&body).expect("future stats are JSON")
}

/// The deadline every host-polled seam check waits, short so a liveness failure
/// reports FAIL in seconds instead of blocking the harness.
const SEAM_DEADLINE: Duration = Duration::from_secs(2);

/// Reads the same counters through the host-polled seam, so the read itself
/// spawns no plugin thread and the counts stay comparable.
fn future_counters(future: &proto_host::future_driver::FutureHost) -> Value {
    let body = future
        .call(
            "metis.future_counters",
            br#"{"x":0}"#,
            Call::new().id(),
            SEAM_DEADLINE,
        )
        .expect("the counters future answers");
    serde_json::from_slice(&body).expect("counters are JSON")
}

/// Parses the `accounting allocs=... frees=... live=... violations=... suppressed=...`
/// line the plugin logs at shutdown.
fn parse_accounting_line(line: &str) -> Option<(u64, u64, u64, u64)> {
    let rest = line.split("accounting ").nth(1)?;
    let mut allocs = None;
    let mut frees = None;
    let mut live = None;
    let mut violations = None;
    for field in rest.split_whitespace() {
        let (key, value) = field.split_once('=')?;
        let value: u64 = value.parse().ok()?;
        match key {
            "allocs" => allocs = Some(value),
            "frees" => frees = Some(value),
            "live" => live = Some(value),
            "violations" => violations = Some(value),
            _ => {}
        }
    }
    Some((allocs?, frees?, live?, violations?))
}

fn drive(harness: &mut Harness) -> Result<(), String> {
    let library_path = library_path()?;
    harness.note(&format!("cdylib: {}", library_path.display()));

    let policy = VersionPolicy::new(2);
    let configuration = json!({"greeting": "hi", "timeout_ms": 500});

    // --- the registry proper ----------------------------------------------
    harness.section("plugin registry");
    let ledger: OrderLedger = Arc::new(Mutex::new(Vec::new()));
    let loaded: Sink = Arc::new(Mutex::new(Vec::new()));
    let mut registry = PluginRegistry::new(RoutingMode::GlobalUnique);
    registry.set_loader(make_loader(
        Arc::clone(&ledger),
        Arc::clone(&loaded),
        "proto-plugin-cdylib",
    ));

    let instance = registry
        .load(&library_path, configuration.clone(), &policy)
        .map_err(|err| format!("load: {err}"))?;
    let abi = loaded
        .lock()
        .unwrap()
        .first()
        .cloned()
        .ok_or("the loader did not keep a handle to the loaded plugin")?;

    let manifest = abi.manifest();
    harness.check(
        "a plugin loaded from a library path reports a validated manifest",
        manifest.id == "proto-plugin-cdylib"
            && manifest.abi_major == 1
            && manifest.version.starts_with("abi 1."),
        &format!("{manifest:?}"),
    );
    harness.check(
        "the registry returned an instance handle",
        instance == "proto-plugin-cdylib#0",
        &instance,
    );

    let advertised = abi.capabilities().map_err(|err| err.to_string())?;
    harness.check(
        "capability discovery read the plugin's published list",
        advertised.len() == 18,
        &format!("{} capabilities", advertised.len()),
    );
    harness.check(
        "version gating refused exactly one capability and left the rest routable",
        registry.refused().len() == 1
            && registry.refused()[0].capability == "tool.future"
            && registry.refused()[0].advertised_version == 99
            && registry.len() == 17,
        &format!("{:?}", registry.refused()),
    );
    harness.check(
        "per-plugin configuration was delivered before discovery",
        abi.saw_log("configured with keys"),
        "the plugin never logged a configure call",
    );
    harness.check(
        "the registry kept the configuration it delivered",
        registry.configuration("proto-plugin-cdylib") == Some(&configuration),
        "the stored configuration differs from what was passed",
    );
    harness.check(
        "schema discovery is one batched call, not one per capability",
        abi.schema_fetch_calls() == 1,
        &format!("{} calls", abi.schema_fetch_calls()),
    );
    harness.note(&format!(
        "routing keys: {}",
        registry.routing_keys().join(", ")
    ));
    harness.note(&format!(
        "descriptor listing {by_id} bytes by id ({caps} capabilities), schema fetch {fetch} bytes \
         in {calls} call, inline listing {inline} bytes",
        by_id = abi.capability_list_bytes(),
        caps = advertised.len(),
        fetch = abi.schema_fetch_bytes(),
        calls = abi.schema_fetch_calls(),
        inline = abi.measure_inline_descriptor_bytes().unwrap_or(0),
    ));

    // A native in-process plugin, same registry, same seam.
    registry
        .register(native::native_text_tools(), json!({}), &policy)
        .map_err(|err| format!("native register: {err}"))?;
    harness.check(
        "a native in-process plugin registered behind the same seam",
        registry.route("tool.word_count").is_some(),
        "tool.word_count is not routed",
    );
    harness.check(
        "routing spans every enabled plugin",
        registry.route("tool.echo").is_some() && registry.route("tool.word_count").is_some(),
        "an enabled plugin's capability is missing from the routing table",
    );

    // Duplicate name under global uniqueness.
    let before = registry.len();
    let duplicate = registry.register(native::duplicate_echo_tools(), json!({}), &policy);
    harness.check(
        "a duplicate capability name fails the load under global uniqueness",
        matches!(&duplicate, Err(RegistryError::DuplicateCapability { capability, .. }) if capability == "tool.echo"),
        &format!("{duplicate:?}"),
    );
    harness.check(
        "a failed load leaves the routing table unchanged",
        registry.len() == before,
        &format!("{} routes, was {before}", registry.len()),
    );

    // Fix 1: a capability listing that can fail.
    let broken = registry.register(native::broken_discovery(), json!({}), &policy);
    harness.check(
        "fix 1: a failed capability listing fails the load with a typed discovery error",
        matches!(&broken, Err(RegistryError::Discovery { plugin, .. }) if plugin == "broken-discovery"),
        &format!("{broken:?}"),
    );
    harness.check(
        "fix 1: nothing was registered for the plugin whose listing failed",
        !registry.list().iter().any(|info| info.id == "broken-discovery"),
        "a plugin with a failed listing was still registered",
    );

    // A manifest that targets another ABI major.
    let wrong = registry.register(native::wrong_abi(), json!({}), &policy);
    harness.check(
        "a manifest that targets another ABI major fails the load",
        matches!(&wrong, Err(RegistryError::ManifestInvalid { plugin, .. }) if plugin == "wrong-abi"),
        &format!("{wrong:?}"),
    );

    // Enable and disable state.
    registry
        .disable("native-text-tools")
        .map_err(|err| err.to_string())?;
    let disabled = invoke(&registry, "tool.word_count", json!({"text": "x"}));
    harness.check(
        "a disabled plugin stays loaded but none of its capabilities are routable",
        matches!(&disabled, Err(PluginError::Disabled { plugin, .. }) if plugin == "native-text-tools")
            && registry.list().iter().any(|info| info.id == "native-text-tools" && !info.enabled),
        &format!("{disabled:?}"),
    );
    registry
        .enable("native-text-tools")
        .map_err(|err| err.to_string())?;
    harness.check(
        "re-enabling makes the capability routable again",
        invoke(&registry, "tool.word_count", json!({"text": "one two"})).is_ok(),
        "the re-enabled capability did not route",
    );

    // Unregister.
    registry
        .register(native::throwaway(), json!({}), &policy)
        .map_err(|err| err.to_string())?;
    let doubled = invoke(&registry, "tool.throwaway", json!({"value": 21}));
    harness.check(
        "a freshly registered plugin routes by capability name",
        doubled.as_ref().map(|p| json_of(p)["doubled"] == 42).unwrap_or(false),
        &format!("{doubled:?}"),
    );
    registry
        .unregister("throwaway")
        .map_err(|err| err.to_string())?;
    harness.check(
        "unregistering shuts the plugin down and removes its routes",
        registry.route("tool.throwaway").is_none()
            && !registry.list().iter().any(|info| info.id == "throwaway"),
        "the plugin or its routes survived unregister",
    );
    harness.check(
        "an unregistered capability is no longer routable",
        matches!(
            invoke(&registry, "tool.throwaway", json!({"value": 1})),
            Err(PluginError::CapabilityNotFound { .. })
        ),
        "an unregistered capability still routed",
    );
    harness.note(&format!(
        "registered plugins: {}",
        registry
            .list()
            .iter()
            .map(|info| format!(
                "{} v{} abi{} instance={} caps={} enabled={}",
                info.id,
                info.version,
                info.abi_major,
                info.instance,
                info.capabilities.len(),
                info.enabled
            ))
            .collect::<Vec<_>>()
            .join("; ")
    ));

    let registry = Arc::new(registry);

    // --- fix 2: a machine-checkable error taxonomy -------------------------
    harness.section("fix 2: machine-checkable errors");
    let e_payload = invoke(&registry, "tool.add", json!({"a": "not a number", "b": 1}))
        .expect_err("a schema-violating request must fail");
    let e_gated = invoke(&registry, "tool.future", json!({"x": 1}))
        .expect_err("a version-gated capability must fail");
    let e_missing = invoke(&registry, "tool.not_registered", json!({}))
        .expect_err("an unknown capability must fail");
    let e_response = invoke(&registry, "tool.bad_response", json!({"x": 1}))
        .expect_err("a schema-violating response must fail");
    let e_plugin = invoke(&registry, "tool.word_count", json!({"text": "   "}))
        .expect_err("an empty word count must fail in the plugin");
    harness.check(
        "a caller payload that violates the schema is its own variant",
        matches!(e_payload, PluginError::CallerPayloadInvalid { .. }),
        e_payload.variant(),
    );
    harness.check(
        "a capability refused by version gating is its own variant",
        matches!(e_gated, PluginError::VersionGated { .. }),
        e_gated.variant(),
    );
    harness.check(
        "a capability that was never registered is its own variant",
        matches!(e_missing, PluginError::CapabilityNotFound { .. }),
        e_missing.variant(),
    );
    harness.check(
        "a response that violates the plugin's own published schema is its own variant",
        matches!(e_response, PluginError::ResponseViolation { .. }),
        e_response.variant(),
    );
    harness.check(
        "a plugin-side failure carries the plugin's own status",
        matches!(
            &e_plugin,
            PluginError::PluginFailed {
                status: Status::InvalidArgument,
                ..
            }
        ),
        e_plugin.variant(),
    );
    let variants = [
        e_payload.variant(),
        e_gated.variant(),
        e_missing.variant(),
        e_response.variant(),
        e_plugin.variant(),
    ];
    harness.check(
        "the five failure paths are distinguishable in code, not by message string",
        variants.iter().collect::<std::collections::HashSet<_>>().len() == 5,
        &format!("{variants:?}"),
    );
    harness.note(&format!(
        "taxonomy: {}; {}; {}; {}; {}",
        e_payload, e_gated, e_missing, e_response, e_plugin
    ));
    let abi_failure = abi
        .call_pending("tool.not_a_capability", b"{}", Duration::from_secs(2))
        .expect_err("an unknown ABI capability must fail");
    harness.check(
        "an ABI status maps onto the taxonomy with its status preserved",
        matches!(
            &abi_failure,
            PluginError::PluginFailed {
                status: Status::NotFound,
                ..
            }
        ),
        abi_failure.variant(),
    );

    // --- fix 3: descriptor size -------------------------------------------
    harness.section("fix 3: descriptor size");
    let by_id = abi.capability_list_bytes();
    let inline = abi
        .measure_inline_descriptor_bytes()
        .map_err(|err| format!("inline measurement: {err}"))?;
    let fetch_bytes = abi.schema_fetch_bytes();
    let fetch_calls = abi.schema_fetch_calls();
    harness.check(
        "the by-id listing is smaller than the inline listing",
        by_id < inline,
        &format!("by-id {by_id}, inline {inline}"),
    );
    harness.note(&format!(
        "descriptor trade-off: inline {inline} bytes in 1 call; by id {by_id} bytes + {fetch_bytes} \
         bytes of schema in {fetch_calls} extra call(s), fetched once at load"
    ));

    // --- fix 4: name squatting --------------------------------------------
    harness.section("fix 4: name squatting under namespacing");
    let ledger2: OrderLedger = Arc::new(Mutex::new(Vec::new()));
    let sink2: Sink = Arc::new(Mutex::new(Vec::new()));
    let mut namespaced = PluginRegistry::new(RoutingMode::Namespaced);
    namespaced.set_loader(make_loader(
        Arc::clone(&ledger2),
        Arc::clone(&sink2),
        "proto-plugin-cdylib",
    ));
    namespaced
        .load(&library_path, json!({}), &policy)
        .map_err(|err| format!("namespaced load: {err}"))?;
    namespaced
        .register(native::duplicate_echo_tools(), json!({}), &policy)
        .map_err(|err| format!("namespaced duplicate register: {err}"))?;
    let echo_cdylib = invoke(
        &namespaced,
        "proto-plugin-cdylib/tool.echo",
        json!({"message": "x"}),
    )
    .map(|p| json_of(&p)["echo"].as_str().map(str::to_owned))
    .map_err(|e| e.to_string())?;
    let echo_duplicate = invoke(
        &namespaced,
        "duplicate-echo-tools/tool.echo",
        json!({"message": "x"}),
    )
    .map(|p| json_of(&p)["echo"].as_str().map(str::to_owned))
    .map_err(|e| e.to_string())?;
    harness.check(
        "namespacing admits the plugin the global rule refused",
        echo_cdylib.as_deref() == Some("x") && echo_duplicate.as_deref() == Some("duplicate:x"),
        &format!("{echo_cdylib:?} vs {echo_duplicate:?}"),
    );
    harness.note(&format!(
        "namespaced routing keys: {}",
        namespaced.routing_keys().join(", ")
    ));
    harness.note(
        "global keys are bare capability names, so the caller needs no plugin id but a second \
         claimant is refused; namespaced keys are plugin_id/capability, so the caller names the \
         plugin and both claimants coexist",
    );

    // --- fix 5: feature gate placement ------------------------------------
    harness.section("fix 5: feature gate placement");
    harness.note(
        "the registry lives in proto-registry behind the `registry` feature; the primitives build \
         is `cargo build -p proto-registry --no-default-features` (no serde, no jsonschema) and the \
         registry builds are `cargo build -p proto-registry --features registry`. Both were run and \
         both compile; see FINDINGS2.md",
    );

    // --- Workstream 1: the async and lifecycle paths ----------------------
    harness.section("async: late completion, never inline");
    let echo_payload = payload(json!({"message": "pending"}));
    let (was_pending, result) = abi.probe_pending("tool.echo", &echo_payload);
    harness.check(
        "invoke returned Pending, never an inline result",
        was_pending,
        "the plugin completed inline",
    );
    harness.check(
        "the completion arrived later, through the host callback",
        result
            .as_ref()
            .map(|p| json_of(p)["echo"] == "pending")
            .unwrap_or(false),
        &format!("{result:?}"),
    );

    harness.section("async: progress before completion");
    let completed = invoke(
        &registry,
        "provider.complete",
        json!({"prompt": "hello"}),
    )
    .map_err(|e| e.to_string())?;
    let progress_lines: Vec<String> = abi
        .host_logs()
        .into_iter()
        .filter(|(_, line)| line.contains("host: progress for request"))
        .map(|(_, line)| line)
        .collect();
    harness.check(
        "two progress callbacks arrived before the completion, in order",
        progress_lines.len() >= 2
            && progress_lines[0].contains("\"step\":1")
            && progress_lines[1].contains("\"step\":2"),
        &format!("{progress_lines:?}"),
    );
    harness.check(
        "the completion followed the progress",
        json_of(&completed)["source"] == "proto-plugin-cdylib",
        &format!("{}", json_of(&completed)),
    );
    harness.check(
        "every progress payload was released by the plugin allocator",
        accounting(&abi)["live"] == 0 && accounting(&abi)["violations"] == 0,
        &format!("{}", accounting(&abi)),
    );

    harness.section("async: events");
    invoke(&registry, "tool.events", json!({"topic": "x"})).map_err(|e| e.to_string())?;
    let events = abi.events();
    harness.check(
        "a capability emitted an event that reached the host with topic and payload",
        events
            .iter()
            .any(|(topic, body)| topic == "tool.events" && !body.is_empty()),
        &format!("{events:?}"),
    );
    harness.check(
        "instance start emitted an event",
        events.iter().any(|(topic, _)| topic == "plugin.started"),
        &format!("{events:?}"),
    );

    harness.section("async: logs at each level");
    invoke(&registry, "tool.logs", json!({})).map_err(|e| e.to_string())?;
    let levels: std::collections::HashSet<i32> = abi
        .host_logs()
        .into_iter()
        .map(|(level, _)| level)
        .collect();
    harness.check(
        "logs arrived at every level",
        [0, 1, 2, 3].iter().all(|level| levels.contains(level)),
        &format!("{levels:?}"),
    );

    harness.section("async: cancellation");
    let (cancel_id, awaiting) = abi
        .begin(
            "tool.delay",
            &payload(json!({"delay_ms": 400, "tag": "cancelme"})),
            abi.instance(),
        )
        .map_err(|e| e.to_string())?;
    thread::sleep(Duration::from_millis(30));
    let start = Instant::now();
    let first = abi.cancel_raw(cancel_id);
    let second = abi.cancel_raw(cancel_id);
    while !abi.saw_log("observed cancel for request") && start.elapsed() < Duration::from_millis(500)
    {
        thread::sleep(Duration::from_millis(2));
    }
    let observed_ms = start.elapsed().as_millis();
    let late = awaiting.wait_timeout(Duration::from_millis(400));
    harness.check(
        "a cancel for a live request is accepted",
        first.is_ok(),
        &format!("{first:?}"),
    );
    harness.check(
        "a second cancel is idempotent",
        second.is_ok(),
        &format!("{second:?}"),
    );
    harness.check(
        "the plugin observed the token promptly while parked",
        abi.saw_log("observed cancel for request"),
        "the plugin never logged the cancel",
    );
    harness.check(
        "the plugin suppressed its completion and the host retired its pending record",
        late.is_none(),
        "the await resolved after a successful cancel",
    );
    harness.note(&format!(
        "the plugin observed the cancel {observed_ms} ms after the host requested it"
    ));

    let (race_id, race_awaiting) = abi
        .begin(
            "tool.races",
            &payload(json!({"delay_ms": 200, "tag": "race"})),
            abi.instance(),
        )
        .map_err(|e| e.to_string())?;
    thread::sleep(Duration::from_millis(30));
    let race_cancel = abi.cancel_raw(race_id);
    let race_late = race_awaiting.wait_timeout(Duration::from_millis(600));
    harness.check(
        "a completion that arrives after a successful cancel is dropped by the host",
        race_cancel.is_ok() && race_late.is_none() && abi.saw_log("dropped a completion for"),
        &format!("cancel={race_cancel:?} late={race_late:?}"),
    );
    harness.check(
        "the dropped completion's buffer was still released",
        accounting(&abi)["live"] == 0 && accounting(&abi)["violations"] == 0,
        &format!("{}", accounting(&abi)),
    );
    let unknown = abi.cancel_raw(9_999_999_999);
    harness.check(
        "cancel of an unknown request id returns not found",
        matches!(
            &unknown,
            Err(PluginError::PluginFailed {
                status: Status::NotFound,
                ..
            })
        ),
        &format!("{unknown:?}"),
    );

    harness.section("async: a completion that never arrives");
    let start = Instant::now();
    let never = abi.invoke_bounded(
        "tool.never",
        &payload(json!({"x": 1})),
        Duration::from_millis(300),
    );
    let never_elapsed = start.elapsed();
    harness.check(
        "the bounded wait timed out instead of hanging",
        never.is_err() && never_elapsed < Duration::from_secs(2),
        &format!("{never:?} after {never_elapsed:?}"),
    );
    harness.note(&format!(
        "the bounded wait returned after {} ms with {never:?}",
        never_elapsed.as_millis()
    ));

    harness.section("async: concurrency");
    let mut handles = Vec::new();
    for index in 0..8u32 {
        let registry = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            let call = Call::new();
            let message = format!("m{index}");
            let response = block_on(registry.invoke(
                "tool.echo",
                payload(json!({ "message": message })),
                &call,
            ))
            .expect("concurrent echo");
            json_of(&response)["echo"].as_str() == Some(message.as_str())
        }));
    }
    let all_matched = handles
        .into_iter()
        .all(|handle| handle.join().unwrap_or(false));
    harness.check(
        "concurrent in-flight calls to one instance and one capability all answer their own caller",
        all_matched,
        "a concurrent echo was delivered to the wrong caller",
    );

    let delays = [300u64, 50, 20, 150, 80];
    let mut handles = Vec::new();
    for (index, delay) in delays.into_iter().enumerate() {
        let registry = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            let call = Call::new();
            let tag = format!("t{index}");
            let response = block_on(registry.invoke(
                "tool.delay",
                payload(json!({ "delay_ms": delay, "tag": tag })),
                &call,
            ))
            .expect("delayed call");
            json_of(&response)["tag"].as_str() == Some(tag.as_str())
        }));
    }
    let out_of_order = handles
        .into_iter()
        .all(|handle| handle.join().unwrap_or(false));
    harness.check(
        "out-of-order completions are delivered to the right caller",
        out_of_order,
        "a delayed completion reached the wrong caller",
    );

    let abi_a = Arc::new(
        AbiPlugin::load(&library_path, "proto-plugin-cdylib-a", Arc::new(Mutex::new(Vec::new())))
            .map_err(|e| e.to_string())?,
    );
    let abi_b = Arc::new(
        AbiPlugin::load(&library_path, "proto-plugin-cdylib-b", Arc::new(Mutex::new(Vec::new())))
            .map_err(|e| e.to_string())?,
    );
    let (_, stats_a) = abi_a.probe_pending("tool.stats", &payload(json!({})));
    let (_, stats_b) = abi_b.probe_pending("tool.stats", &payload(json!({})));
    let address_a = stats_a
        .as_ref()
        .map(|p| json_of(p)["instance"].clone())
        .unwrap_or(Value::Null);
    let address_b = stats_b
        .as_ref()
        .map(|p| json_of(p)["instance"].clone())
        .unwrap_or(Value::Null);
    harness.check(
        "two instances of the same plugin have independent state",
        address_a != address_b && !address_a.is_null(),
        &format!("{address_a} vs {address_b}"),
    );

    let cdylib_thread = {
        let registry = Arc::clone(&registry);
        thread::spawn(move || {
            let call = Call::new();
            block_on(registry.invoke("tool.echo", payload(json!({"message": "x"})), &call)).is_ok()
        })
    };
    let native_thread = {
        let registry = Arc::clone(&registry);
        thread::spawn(move || {
            let call = Call::new();
            block_on(registry.invoke("tool.word_count", payload(json!({"text": "a b"})), &call))
                .is_ok()
        })
    };
    harness.check(
        "two different plugins are in flight together without crossing",
        cdylib_thread.join().unwrap_or(false) && native_thread.join().unwrap_or(false),
        "one of the two plugins failed under concurrency",
    );

    harness.section("async: instance lifecycle");
    let instance_one = abi.create_instance_handle().map_err(|e| e.to_string())?;
    let stats_one = abi
        .invoke_on(
            instance_one,
            "tool.stats",
            &payload(json!({})),
            Duration::from_secs(5),
        )
        .map_err(|e| e.to_string())?;
    let destroy_one = abi.destroy_instance_handle(instance_one);
    let destroy_again = abi.destroy_instance_handle(instance_one);
    let instance_two = abi.create_instance_handle().map_err(|e| e.to_string())?;
    let stats_two = abi
        .invoke_on(
            instance_two,
            "tool.stats",
            &payload(json!({})),
            Duration::from_secs(5),
        )
        .map_err(|e| e.to_string())?;
    harness.check(
        "an instance can be created, used, and destroyed",
        destroy_one == MetisStatus::Ok && json_of(&stats_one)["instance"].is_string(),
        &format!("stats={stats_one:?} destroy={destroy_one:?}"),
    );
    harness.check(
        "a double destroy is reported as not found",
        destroy_again == MetisStatus::NotFound,
        &format!("{destroy_again:?}"),
    );
    harness.check(
        "an instance can be created again on a reused address",
        json_of(&stats_two)["instance"].is_string(),
        &format!("{stats_two:?}"),
    );
    harness.note(&format!(
        "instance addresses: first {}, second {}",
        json_of(&stats_one)["instance"],
        json_of(&stats_two)["instance"]
    ));

    // --- the host-polled future seam (lane 1) ------------------------------
    harness.section("host-polled future seam");
    let future = abi.future_host(Arc::clone(&ledger));
    let stats_before = future_counters(&future);

    // 1. One capability driven end to end by the host as the only executor:
    // not ready on first poll, ready after the host's timer service answers,
    // and the payload arrives as a plugin-allocated ready buffer.
    let request_id = Call::new().id();
    let mut future_handle = future
        .invoke_future("tool.poll_future", br#"{"x":7,"delay_ms":0}"#, request_id)
        .map_err(|err| err.to_string())?;
    let first = future_handle
        .poll_once_result()
        .map_err(|err| err.to_string())?;
    harness.check(
        "the plugin future is not ready on its first poll",
        first.is_none(),
        &format!("first poll returned {first:?}"),
    );
    let delivered = future_handle
        .poll_to_ready(SEAM_DEADLINE)
        .map_err(|err| err.to_string())?;
    let delivered: Value = serde_json::from_slice(&delivered).map_err(|err| err.to_string())?;
    harness.check(
        "the host waker wakes the host poll loop and the future delivers its payload",
        delivered["y"] == 7,
        &format!("{delivered}"),
    );
    let drop_status = future_handle.drop_future();
    harness.check(
        "the host drops the future exactly once and the drop is legal",
        matches!(drop_status, Ok(proto_abi::MetisStatus::Ok)),
        &format!("{drop_status:?}"),
    );

    // 2. Runtime free on that path: the plugin's own counters show no thread
    // was spawned and every poll ran on one thread, which is the host's.
    let stats_after = future_counters(&future);
    let spawned_delta = stats_after["threads_spawned"]
        .as_u64()
        .unwrap_or(0)
        .saturating_sub(stats_before["threads_spawned"].as_u64().unwrap_or(0));
    harness.check(
        "the host-polled path spawned no plugin thread and no plugin runtime",
        spawned_delta == 0 && stats_after["poll_threads"].as_u64() == Some(1),
        &format!(
            "spawned delta {spawned_delta}, poll_threads {}",
            stats_after["poll_threads"]
        ),
    );

    // 3. The hand-written vtable, the C stand-in: no async-ffi anywhere in it.
    let request_id = Call::new().id();
    let mut future_handle = future
        .invoke_future("tool.hand_future", br#"{"x":11,"delay_ms":0}"#, request_id)
        .map_err(|err| err.to_string())?;
    let first = future_handle
        .poll_once_result()
        .map_err(|err| err.to_string())?;
    let delivered = future_handle
        .poll_to_ready(SEAM_DEADLINE)
        .map_err(|err| err.to_string())?;
    let delivered: Value = serde_json::from_slice(&delivered).map_err(|err| err.to_string())?;
    harness.check(
        "the hand-written vtable drives a capability with no async-ffi involved",
        first.is_none() && delivered["y"] == 11,
        &format!("first {first:?}, delivered {delivered}"),
    );
    let _ = future_handle.drop_future();

    // 4. Cancellation resolves the future as Cancelled, and the host drops only
    // after observing a poll return.
    let request_id = Call::new().id();
    let mut future_handle = future
        .invoke_future("tool.poll_future", br#"{"x":1,"delay_ms":300}"#, request_id)
        .map_err(|err| err.to_string())?;
    let _ = future_handle
        .poll_once_result()
        .map_err(|err| err.to_string())?;
    let cancel_status = future.cancel(request_id);
    let resolution = future_handle.poll_to_ready(Duration::from_secs(2));
    harness.check(
        "a cancel resolves the future as Cancelled instead of a mid-flight drop",
        cancel_status == proto_abi::MetisStatus::Ok
            && matches!(
                resolution,
                Err(proto_host::future_driver::FutureError::Cancelled)
            ),
        &format!("cancel {cancel_status:?}, resolution {resolution:?}"),
    );
    let _ = future_handle.drop_future();

    // 5. Waker clones balance: every clone the plugin made was released, so no
    // waker clone survives the futures that held it.
    let seam_stats = future_counters(&future);
    harness.check(
        "every waker clone the plugin made was released",
        seam_stats["waker_clones"] == seam_stats["waker_releases"],
        &format!(
            "clones {} releases {}",
            seam_stats["waker_clones"], seam_stats["waker_releases"]
        ),
    );
    harness.note(&format!("future seam counters: {seam_stats}"));

    // 6. The legacy Pending/complete path is untouched and still works next to
    // the future seam.
    let legacy = invoke(&registry, "tool.echo", json!({"message": "legacy"}))
        .map_err(|err| err.to_string())?;
    harness.check(
        "the legacy Pending/complete path still works alongside the future seam",
        json_of(&legacy)["echo"] == "legacy",
        &format!("{legacy:?}"),
    );

    // --- the host-polled future seam (lane 2: hazards) ---------------------
    {
        let accounting_query = || -> Value {
            let body = abi
                .call_pending("metis.accounting", b"{}", Duration::from_secs(5))
                .expect("the accounting call answers");
            serde_json::from_slice(&body).expect("accounting is JSON")
        };
        let probe = proto_host::future_driver::AccountingProbe::new(&accounting_query);
        harness.note(&format!(
            "plugin live buffers before the hazard run: {}",
            probe.live_buffers_for_diagnostics()
        ));
        proto_host::future_driver::run_hazards(harness, &future, &probe);
        harness.note(&format!(
            "plugin live buffers after the hazard run: {}",
            probe.live_buffers_for_diagnostics()
        ));
    }

    // --- the host-polled future seam (lane 3: versioning) ------------------
    harness.section("lane 3: prefix compatibility and append-only versioning");
    match proto_host::versioning::run(&library_path) {
        Ok(checks) => {
            for c in checks {
                harness.check(&c.name, c.ok, &c.detail);
            }
        }
        Err(err) => harness.check("lane 3: versioning checks ran", false, &err),
    }

    // --- typed method families --------------------------------------------
    harness.section("typed method families above the seam");
    let tools = Toolset::new(Arc::clone(&registry));
    let providers = ProviderSet::new(Arc::clone(&registry));
    let echoed = tools.echo(&EchoRequest {
        message: "typed".to_owned(),
    });
    harness.check(
        "a typed tool method reaches a cdylib capability over the erased seam",
        echoed.as_ref().map(|r| r.echo.as_str()) == Ok("typed"),
        &format!("{echoed:?}"),
    );
    let added = tools.add(&AddRequest { a: 20, b: 22 });
    harness.check(
        "a second typed tool method reaches another cdylib capability",
        added.as_ref().map(|r| r.sum) == Ok(42),
        &format!("{added:?}"),
    );
    let uppercased = tools.uppercase(&UppercaseRequest {
        text: "shout".to_owned(),
    });
    harness.check(
        "a third typed tool method, same family, same seam",
        uppercased.as_ref().map(|r| r.upper.as_str()) == Ok("SHOUT"),
        &format!("{uppercased:?}"),
    );
    let counted = tools.word_count(&WordCountRequest {
        text: "one two three".to_owned(),
    });
    harness.check(
        "the native in-process plugin answers through the same typed tool family",
        counted.as_ref().map(|r| r.words) == Ok(3),
        &format!("{counted:?}"),
    );
    let completed = providers.complete(&CompleteRequest {
        prompt: "hello".to_owned(),
    });
    harness.check(
        "a typed provider method family reaches a cdylib capability",
        completed.as_ref().map(|r| r.source.as_str()) == Ok("proto-plugin-cdylib"),
        &format!("{completed:?}"),
    );
    let summarized = providers.summarize(&SummarizeRequest {
        text: "alpha beta".to_owned(),
    });
    harness.check(
        "the native plugin answers through the same typed provider family",
        summarized.as_ref().map(|r| r.source.as_str()) == Ok("native-text-tools"),
        &format!("{summarized:?}"),
    );
    let wrong_kind = tools.handle::<CompleteRequest, proto_host::types::CompleteResponse>(
        "provider.complete",
    );
    harness.check(
        "a tool handle refuses a provider kind capability",
        matches!(&wrong_kind, Err(PluginError::Unsupported { .. })),
        "the kind was not enforced at handle construction",
    );
    let mismatch_error = tools
        .echo_mismatched(&MismatchedEchoRequest {
            text: "x".to_owned(),
        })
        .expect_err("the mismatched request must fail");
    harness.check(
        "a typed wrapper whose Rust fields do not match the published schema fails at runtime, not compile time",
        matches!(mismatch_error, PluginError::CallerPayloadInvalid { .. }),
        mismatch_error.variant(),
    );
    harness.note(&format!(
        "erasure cost: the mismatch compiled; only the schema caught it ({mismatch_error})"
    ));
    let typed_bad = tools.bad_response(&BadResponseRequest { x: 1 });
    harness.check(
        "the typed family surfaces a response violation as ResponseViolation",
        matches!(typed_bad, Err(PluginError::ResponseViolation { .. })),
        &format!("{typed_bad:?}"),
    );

    // --- what the registry costs ------------------------------------------
    harness.section("what the registry costs");
    for line in proto_host::bench::run(&registry) {
        println!("BENCH {line}");
        Harness::flush();
    }
    // The two executor baselines for the legacy callback path, re-measured by
    // this harness in this binary so the host-polled numbers below can be read
    // against all three known baselines at once.
    for line in proto_host::bench::executor_baselines(&registry, &abi) {
        println!("BENCH {line}");
        Harness::flush();
    }

    harness.section("host-polled future seam cost");
    for line in proto_host::future_bench::run(&future) {
        println!("BENCH {line}");
        Harness::flush();
    }
    let roundtrip = future_stats(&abi);
    let count = roundtrip["service_roundtrip_count"].as_f64().unwrap_or(0.0);
    let total = roundtrip["service_roundtrip_total_ns"].as_f64().unwrap_or(0.0);
    println!(
        "BENCH service_request timer round trip: last {} ns, avg {:.0} ns over {} samples",
        roundtrip["service_roundtrip_ns"],
        if count > 0.0 { total / count } else { 0.0 },
        roundtrip["service_roundtrip_count"]
    );
    Harness::flush();

    // --- shutdown and unload ordering -------------------------------------
    harness.section("shutdown and unload ordering");
    abi.fire("tool.straggler", &payload(json!({"delay_ms": 250})))
        .map_err(|e| e.to_string())?;
    let (_delay_id, delay_awaiting) = abi
        .begin(
            "tool.delay",
            &payload(json!({"delay_ms": 300, "tag": "joined"})),
            abi.instance(),
        )
        .map_err(|e| e.to_string())?;
    thread::sleep(Duration::from_millis(30));
    abi.shutdown().map_err(|e| e.to_string())?;
    let joined = delay_awaiting.wait_timeout(Duration::from_millis(500));
    harness.check(
        "an in-flight call completed before shutdown returned (the worker was joined)",
        joined.is_some(),
        "the in-flight call did not complete before shutdown returned",
    );
    harness.check(
        "shutdown joined the plugin's workers",
        abi.saw_log("shutdown joined"),
        "the plugin never logged the worker join",
    );
    thread::sleep(Duration::from_millis(450));
    harness.check(
        "a callback arriving after shutdown is logged and ignored",
        abi.saw_log("arrived after shutdown and was ignored"),
        "the straggler callback was not ignored",
    );
    let destroy_after = abi.destroy_instance_handle(abi.instance());
    harness.check(
        "destroy_instance after shutdown returns not found",
        destroy_after == MetisStatus::NotFound,
        &format!("{destroy_after:?}"),
    );
    let accounting_line = abi
        .host_logs()
        .into_iter()
        .filter_map(|(_, line)| parse_accounting_line(&line))
        .next_back();
    harness.check(
        "the plugin released every buffer it allocated",
        matches!(accounting_line, Some((allocs, frees, live, violations)) if allocs == frees && live == 0 && violations == 0),
        &format!("{accounting_line:?}"),
    );

    // --- teardown ----------------------------------------------------------
    harness.section("teardown");
    // The typed facades still hold a registry clone, and the registry holds the
    // plugin; every clone has to go before the library can unload.
    drop(tools);
    drop(providers);
    drop(registry);
    // The future driver holds the table and the timer thread. It drops before
    // the plugin is shut down and the library is unloaded, which is the ordering
    // the seam requires: no future and no waker clone outlives the library.
    drop(future);
    loaded.lock().unwrap().clear();
    drop(abi);
    let order = ledger.lock().unwrap().clone();
    let shutdown_pos = order.iter().position(|entry| entry == "shutdown returned");
    let unload_pos = order.iter().position(|entry| entry == "library unloaded");
    let future_pos = order.iter().position(|entry| entry == "future host dropped");
    harness.check(
        "the library was dropped only after shutdown had returned",
        matches!((shutdown_pos, unload_pos), (Some(s), Some(u)) if s < u),
        &format!("{order:?}"),
    );
    harness.check(
        "every future and waker was dropped before the library unloaded",
        matches!((future_pos, unload_pos), (Some(f), Some(u)) if f < u),
        &format!("{order:?}"),
    );
    harness.note(&format!("teardown order: {order:?}"));

    Ok(())
}

/// The cdylib sits next to the host binary, which is where cargo puts both.
fn library_path() -> Result<PathBuf, String> {
    if let Ok(explicit) = std::env::var("PROTO_PLUGIN_LIB") {
        return Ok(PathBuf::from(explicit));
    }
    let executable = std::env::current_exe().map_err(|err| format!("current_exe: {err}"))?;
    let directory = executable
        .parent()
        .ok_or_else(|| "the host binary has no parent directory".to_owned())?;
    let name = if cfg!(target_os = "macos") {
        "libproto_plugin_cdylib.dylib"
    } else if cfg!(target_os = "windows") {
        "proto_plugin_cdylib.dll"
    } else {
        "libproto_plugin_cdylib.so"
    };
    let path = directory.join(name);
    if !path.exists() {
        return Err(format!(
            "{} does not exist; build the whole workspace first, because `cargo run` does not \
             rebuild the cdylib member",
            path.display()
        ));
    }
    Ok(path)
}
