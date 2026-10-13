//! One host runtime, eight plugin futures, no plugin thread.
//!
//! This lives in a binary of its own because the test harness starts a thread
//! per test: a thread count taken in a binary that runs other cases in parallel
//! would measure the harness rather than the plugin.

#[path = "support/mod.rs"]
mod support;

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};

use plugin_contract::abi::MetisStatus;
use plugin_runtime::{PluginCall, Registry};

use support::{BUDGET, decode, new_instance, request, thread_count};

#[test]
fn eight_plugin_futures_run_concurrently_with_no_plugin_thread() {
    let (host, instance) = new_instance();
    let mut registry = Registry::new();
    registry.register("fixture", Arc::clone(&instance)).unwrap();

    let before = thread_count();
    let mut calls: Vec<Pin<Box<PluginCall>>> = (0..8)
        .map(|index| {
            Box::pin(
                instance
                    .invoke("tool.pending", 1000 + index, &request("wait"))
                    .expect("the future starts"),
            )
        })
        .collect();

    let mut cx = Context::from_waker(Waker::noop());
    let mut ready = Vec::new();
    let start = Instant::now();
    while ready.len() < calls.len() {
        assert!(start.elapsed() < BUDGET, "the eight futures never finished");
        for (index, call) in calls.iter_mut().enumerate() {
            if ready.contains(&index) {
                continue;
            }
            if let Poll::Ready(result) = call.as_mut().poll(&mut cx) {
                let body = result.expect("the call succeeds");
                ready.push(index);
                assert_eq!(decode(&body).echo, "timer 20");
            }
        }
        // The host answers every service the futures asked for, after the polls
        // that parked them, which is the whole protocol.
        for service in host.pending_services() {
            let request_id = service.request_id;
            host.take_service(request_id);
            instance
                .complete_service(request_id, MetisStatus::Ok, br#"{"delay_ms":20}"#)
                .expect("the host answers the timer");
        }
        thread::sleep(Duration::from_millis(1));
    }
    let after = thread_count();
    assert_eq!(
        before, after,
        "eight concurrent plugin futures need no plugin thread and no second runtime"
    );
    assert_eq!(ready.len(), 8);
    drop(calls);
    assert!(instance.in_flight().is_empty(), "every future was dropped");
    let (clones, releases, late) = host.waker_counts();
    assert_eq!(clones, releases, "every waker clone was released");
    assert_eq!(late, 0, "no wake arrived after a drop in this case");

    let _ = registry.unregister("fixture");
}
