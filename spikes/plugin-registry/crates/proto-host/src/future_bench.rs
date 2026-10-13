//! PROTOTYPE (throwaway). Not product code.
//!
//! Release measurements for the host-polled future seam. Same process, no warm
//! up discipline, so read the numbers as orders of magnitude, as the round two
//! benchmark is read.
//!
//! Three things are measured the spec asks for:
//!
//! - the full host-polled path per call: `invoke_future`, a host timer
//!   `service_request` round trip, a poll to ready, and drop, averaged and at
//!   the tail;
//! - the `catch_unwind` shim's share, by running a ready-on-first-poll future
//!   with the shim vtable and with the no-shim vtable;
//! - future creation versus reuse, on one hand-written always-ready vtable, so
//!   the only difference is whether the object is built and destroyed per call
//!   or polled again.
//!
//! The `service_request` timer round trip is measured by the plugin itself (see
//! `metis.future_stats`) and reported by the caller after this runs.

use std::time::{Duration, Instant};

use proto_registry::Call;
use serde_json::json;

use crate::future_driver::FutureHost;

const LATENCY_ITERATIONS: u32 = 2_000;
const READY_ITERATIONS: u32 = 20_000;
/// A short, bounded wait: a broken seam fails the call in seconds rather than
/// hanging the benchmark.
const CALL_DEADLINE: Duration = Duration::from_secs(2);

fn per_iteration(iterations: u32, mut body: impl FnMut()) -> f64 {
    let started = Instant::now();
    for _ in 0..iterations {
        body();
    }
    started.elapsed().as_nanos() as f64 / f64::from(iterations)
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let index = (((sorted.len() - 1) as f64) * p).round() as usize;
    sorted[index]
}

/// Runs the loops and returns one line per measurement.
pub fn run(future: &FutureHost) -> Vec<String> {
    let mut lines = Vec::new();

    // (a) The full host-polled path, per call, including the host timer.
    let body = serde_json::to_vec(&json!({ "x": 7, "delay_ms": 0 })).expect("encodable");
    let mut samples: Vec<u64> = Vec::with_capacity(LATENCY_ITERATIONS as usize);
    let started = Instant::now();
    for _ in 0..LATENCY_ITERATIONS {
        let request_id = Call::new().id();
        let sample = Instant::now();
        // The response is `{ "y": 7 }`; the value is checked by the seam checks.
        let _ = future
            .call("tool.poll_future", &body, request_id, CALL_DEADLINE)
            .expect("the host-polled call answers");
        samples.push(sample.elapsed().as_nanos() as u64);
    }
    let wall = started.elapsed();
    let average = samples.iter().sum::<u64>() as f64 / samples.len() as f64;
    samples.sort_unstable();
    lines.push(format!(
        "host-polled path, full (invoke_future + host timer service_request + poll + drop): \
         avg {average:.0} ns ({avg_us:.2} us), p50 {p50} ns, p95 {p95} ns, p99 {p99} ns, \
         max {max} ns, {LATENCY_ITERATIONS} calls in {wall:.1?}",
        avg_us = average / 1000.0,
        p50 = percentile(&samples, 0.50),
        p95 = percentile(&samples, 0.95),
        p99 = percentile(&samples, 0.99),
        max = samples.last().copied().unwrap_or(0),
    ));

    // (b) The catch_unwind shim's share: same ready-on-first-poll future, one
    // vtable with the shim and one without.
    let ready_body = serde_json::to_vec(&json!({ "x": 1 })).expect("encodable");
    let with_shim = per_iteration(READY_ITERATIONS, || {
        let request_id = Call::new().id();
        let _ = future
            .call("tool.future_ready", &ready_body, request_id, CALL_DEADLINE)
            .expect("the shim future answers");
    });
    let no_shim = per_iteration(READY_ITERATIONS, || {
        let request_id = Call::new().id();
        let _ = future
            .call(
                "tool.future_ready_noshim",
                &ready_body,
                request_id,
                CALL_DEADLINE,
            )
            .expect("the no-shim future answers");
    });
    let shim_share = with_shim - no_shim;
    lines.push(format!(
        "catch_unwind shim, ready-on-first-poll future (invoke + 1 poll + drop): shim \
         {with_shim:.0} ns/call, no shim {no_shim:.0} ns/call, shim share {shim_share:.0} ns/call \
         ({:.0}% of the with-shim call)",
        100.0 * shim_share / with_shim.max(1.0)
    ));

    // (c) Future creation versus reuse, both on the hand-written always-ready
    // vtable, so the only difference is the object lifecycle.
    let reuse_body = serde_json::to_vec(&json!({ "x": 1 })).expect("encodable");
    let creation = per_iteration(READY_ITERATIONS, || {
        let request_id = Call::new().id();
        let _ = future
            .call("tool.future_reuse", &reuse_body, request_id, CALL_DEADLINE)
            .expect("create+poll+drop answers");
    });
    let request_id = Call::new().id();
    let mut reuse_handle = future
        .invoke_future("tool.future_reuse", &reuse_body, request_id)
        .expect("reuse invoke_future");
    let reuse = per_iteration(READY_ITERATIONS, || {
        let _ = reuse_handle
            .poll_to_ready(CALL_DEADLINE)
            .expect("reuse poll answers");
    });
    let _ = reuse_handle.drop_future();
    lines.push(format!(
        "future creation versus reuse (same hand-written vtable): create+poll+drop {creation:.0} \
         ns/call, reuse (poll only) {reuse:.0} ns/call, lifecycle overhead {:.0} ns/call",
        creation - reuse
    ));

    lines
}
