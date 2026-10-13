//! PROTOTYPE (throwaway). Not product code.
//!
//! What the registry costs per call. A microbenchmark loop: same process, no
//! warm up discipline, so read the numbers as orders of magnitude.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use proto_registry::{Call, Payload, PluginRegistry, block_on};

use crate::abi_adapter::AbiPlugin;

const VALIDATION_ITERATIONS: u32 = 20_000;
const CALL_ITERATIONS: u32 = 2_000;
const CONCURRENCY: usize = 8;
const CONCURRENT_CALLS: u32 = 200;

/// Runs the loops and returns one line per measurement.
pub fn run(registry: &Arc<PluginRegistry>) -> Vec<String> {
    let mut lines = Vec::new();
    let route = registry
        .route("tool.echo")
        .expect("tool.echo is registered by the cdylib");
    let request = Payload::from_json(&json!({"message": "hello prototype"})).expect("encodable");
    let response =
        Payload::from_json(&json!({"echo": "hello prototype", "length": 15})).expect("encodable");

    let compiles = 50;
    let started = Instant::now();
    for _ in 0..compiles {
        let _ = jsonschema::validator_for(route.request_schema()).expect("compiles");
    }
    let compile_us = started.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(compiles);
    lines.push(format!(
        "schema compile: {compile_us:.0} us per schema ({routes} routed capabilities => {schemas} \
         schemas compiled at load, about {total:.1} ms)",
        routes = registry.len(),
        schemas = registry.len() * 2,
        total = compile_us * (registry.len() * 2) as f64 / 1000.0,
    ));

    let request_ns = per_iteration(VALIDATION_ITERATIONS, || {
        let _ = route.validate_request(&request);
    });
    let response_ns = per_iteration(VALIDATION_ITERATIONS, || {
        let _ = route.validate_response(&response);
    });
    lines.push(format!(
        "request validation: {request_ns:.0} ns per call (small object, 1 string field)"
    ));
    lines.push(format!(
        "response validation: {response_ns:.0} ns per call (small object, 2 fields)"
    ));
    lines.push(format!(
        "both directions: {:.0} ns per call, the number the registry adds to every routed call",
        request_ns + response_ns
    ));

    let serde_ns = per_iteration(VALIDATION_ITERATIONS, || {
        let encoded = serde_json::to_vec(&json!({"message": "hello prototype"})).expect("encodable");
        let _: serde_json::Value = serde_json::from_slice(&encoded).expect("decodable");
    });
    lines.push(format!(
        "baseline: serde JSON encode plus parse of the same payload, {serde_ns:.0} ns, which \
         validation sits on top of"
    ));

    let word_count_bytes =
        serde_json::to_vec(&json!({"text": "one two three"})).expect("encodable");
    let native_ns = per_iteration(CALL_ITERATIONS, || {
        let call = Call::new();
        let _ = block_on(registry.invoke(
            "tool.word_count",
            Payload::from_bytes(word_count_bytes.clone()),
            &call,
        ))
        .expect("the native plugin answers");
    });
    lines.push(format!(
        "registry.invoke end to end, native in process plugin, both directions validated: \
         {native_ns:.0} ns per call"
    ));

    let echo_bytes =
        serde_json::to_vec(&json!({"message": "hello prototype"})).expect("encodable");
    let abi_ns = per_iteration(CALL_ITERATIONS, || {
        let call = Call::new();
        let _ = block_on(registry.invoke(
            "tool.echo",
            Payload::from_bytes(echo_bytes.clone()),
            &call,
        ))
        .expect("the cdylib answers");
    });
    lines.push(format!(
        "async completion latency, cdylib through the C table and the landed HostAdapter: \
         {abi_ns:.0} ns per call ({:.1} us), one worker thread round trip",
        abi_ns / 1000.0
    ));
    lines.push(format!(
        "validation share: {:.0}% of the native call, {:.0}% of the cdylib call",
        100.0 * (request_ns + response_ns) / native_ns,
        100.0 * (request_ns + response_ns) / abi_ns,
    ));

    // Concurrency: N threads, K calls each, one capability, one instance.
    let started = Instant::now();
    let mut handles = Vec::new();
    for worker in 0..CONCURRENCY {
        let registry = Arc::clone(registry);
        handles.push(thread::spawn(move || {
            for call_index in 0..CONCURRENT_CALLS {
                let call = Call::new();
                let message = format!("w{worker}c{call_index}");
                let payload = Payload::from_json(&json!({ "message": message })).expect("encodable");
                let body = block_on(registry.invoke("tool.echo", payload, &call))
                    .expect("the cdylib answers under load");
                let response: serde_json::Value = body.to_json().expect("JSON");
                assert_eq!(response["echo"], message, "a completion reached the wrong caller");
            }
        }));
    }
    for handle in handles {
        handle.join().expect("worker thread");
    }
    let elapsed = started.elapsed();
    let total = (CONCURRENCY as u32 * CONCURRENT_CALLS) as f64;
    lines.push(format!(
        "concurrency: {CONCURRENCY} threads x {CONCURRENT_CALLS} calls = {total:.0} in-flight \
         completions, all matched to their caller, {:.0} ns per call wall clock, {elapsed:.1?} total",
        elapsed.as_nanos() as f64 / total
    ));

    lines
}

fn per_iteration(iterations: u32, mut body: impl FnMut()) -> f64 {
    let started = Instant::now();
    for _ in 0..iterations {
        body();
    }
    started.elapsed().as_nanos() as f64 / f64::from(iterations)
}

/// The two executor baselines for the legacy callback path, re-measured by the
/// same harness in the same binary. The plugin's per-call work goes through the
/// SDK-injected `Executor` seam, so the only difference between the two lines
/// is the executor the plugin has installed: the bounded pool (the default,
/// round two's 5.4 us figure) or the thread-per-call fallback (the 26 us
/// figure). The host-polled future seam is measured separately and does not use
/// either.
const BASELINE_ITERATIONS: u32 = 1_000;

pub fn executor_baselines(registry: &Arc<PluginRegistry>, abi: &AbiPlugin) -> Vec<String> {
    let mut lines = Vec::new();
    let echo_bytes =
        serde_json::to_vec(&json!({ "message": "hello prototype" })).expect("encodable");
    for (mode, label) in [("thread", "thread-per-call"), ("pool", "pooled")] {
        let switch = format!(r#"{{"mode":"{mode}","workers":4}}"#);
        abi.call_pending("metis.executor", switch.as_bytes(), Duration::from_secs(10))
            .unwrap_or_else(|err| panic!("could not switch the plugin executor to {mode}: {err}"));
        let ns = per_iteration(BASELINE_ITERATIONS, || {
            let call = Call::new();
            let _ = block_on(registry.invoke(
                "tool.echo",
                Payload::from_bytes(echo_bytes.clone()),
                &call,
            ))
            .expect("the cdylib answers");
        });
        lines.push(format!(
            "legacy callback path, {label} executor: {ns:.0} ns per call ({:.1} us), one worker \
             round trip, {BASELINE_ITERATIONS} calls",
            ns / 1000.0
        ));
    }
    lines
}
