//! The waker lifecycle, driven against the real fixture cdylib.
//!
//! The host hands the plugin's poll one owning waker reference per poll, and the
//! plugin's clones are the plugin's. What these tests assert is the accounting,
//! because the accounting is the evidence: every reference the host handed out
//! is released, a wake that arrives after the future was dropped is recorded
//! against the call it belongs to, and a reference that was already consumed is
//! not released twice.

#[path = "support/mod.rs"]
mod support;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use plugin_contract::abi::MetisStatus;
use plugin_runtime::{CallError, Host, Instance, PluginCall};

use support::{new_instance, request};

/// Polls one call once.
///
/// The waker is the no-op one: every poll here is either ready or parked on a
/// host service the test answers itself, so there is nothing for a wake to
/// schedule. A real daemon passes its own task waker, which is the same
/// interface.
fn poll_once(call: &mut PluginCall) -> Poll<Result<Vec<u8>, CallError>> {
    let mut context = Context::from_waker(Waker::noop());
    Pin::new(call).poll(&mut context)
}

/// Every owning waker reference the host handed out was released.
fn assert_references_balanced(host: &Host, what: &str) {
    let (handed_out, released, _) = host.waker_counts();
    assert!(handed_out > 0, "{what}: no waker reference was handed out");
    assert_eq!(
        handed_out, released,
        "{what}: every owning reference the host handed out is released"
    );
}

/// The references balance and no wake arrived late, which is what a call that is
/// not the wake-after-drop case must show.
fn assert_balanced(host: &Host, what: &str) {
    assert_references_balanced(host, what);
    let (_, _, late_wakes) = host.waker_counts();
    assert_eq!(late_wakes, 0, "{what}: no wake arrived after a drop");
}

/// Drives one `tool.pending` call to its answer: the plugin parks, the host
/// answers the timer, and the second poll resolves.
fn drive_pending(host: &Host, instance: &Arc<Instance>, request_id: u64) {
    let mut call = instance
        .invoke("tool.pending", request_id, &request("wait"))
        .expect("the future starts");
    assert!(
        matches!(poll_once(&mut call), Poll::Pending),
        "the timer capability parks on its first poll"
    );
    let service = host
        .take_service(request_id)
        .expect("the plugin asked for a timer");
    assert_eq!(service.service, "timer");
    instance
        .complete_service(request_id, MetisStatus::Ok, br#"{"delay_ms":20}"#)
        .expect("the plugin accepts the answer");
    match poll_once(&mut call) {
        Poll::Ready(Ok(body)) => assert_eq!(support::decode(&body).echo, "timer 20"),
        Poll::Ready(Err(error)) => panic!("the answered timer resolves the call: {error:?}"),
        Poll::Pending => panic!("the answered timer resolves the call"),
    }
    drop(call);
}

#[test]
fn a_completed_poll_releases_the_reference_the_host_handed_to_it() {
    let (host, instance) = new_instance();
    let mut call = instance
        .invoke("tool.echo", 11, &request("ready"))
        .expect("the future starts");

    match poll_once(&mut call) {
        Poll::Ready(Ok(body)) => assert_eq!(support::decode(&body).echo, "ready"),
        Poll::Ready(Err(error)) => panic!("the echo capability answers: {error:?}"),
        Poll::Pending => panic!("the echo capability answers on its first poll"),
    }

    // The accounting is symmetric: one owning reference for the poll the host
    // ran, one for the clone the bridge takes to build the task waker, and one
    // release for each of them.
    let (handed_out, _, _) = host.waker_counts();
    assert_eq!(
        handed_out, 2,
        "one reference for the poll and one for the bridge's clone"
    );
    // The poll return is the release, so this holds before the future is
    // dropped: a poll that parks or fails cannot leak its reference.
    assert_balanced(&host, "a completed poll");
    drop(call);
    assert_balanced(&host, "a full call");
}

#[test]
fn a_full_call_a_cancelled_call_and_an_abandoned_call_all_balance() {
    let (host, instance) = new_instance();

    // A full call: two polls, the second after the host answers the timer.
    drive_pending(&host, &instance, 21);
    assert_balanced(&host, "a full call");

    // A cancelled call: the plugin parks on its cancellation token and resolves
    // as cancelled.
    let mut call = instance
        .invoke("tool.cancellable", 22, &request("cancel"))
        .expect("the future starts");
    assert!(matches!(poll_once(&mut call), Poll::Pending));
    instance.cancel(22).expect("the plugin accepts the cancel");
    match poll_once(&mut call) {
        Poll::Ready(Err(CallError::Status(MetisStatus::Cancelled))) => {}
        Poll::Ready(Err(error)) => panic!("the cancelled call resolves as cancelled: {error:?}"),
        Poll::Ready(Ok(_)) => panic!("the cancelled call resolves as cancelled"),
        Poll::Pending => panic!("a cancelled call resolves on its next poll"),
    }
    drop(call);
    assert_balanced(&host, "a cancelled call");

    // An abandoned call: the plugin parks on a service the host never answers,
    // and the caller drops the future instead.
    let mut call = instance
        .invoke("tool.pending", 23, &request("abandon"))
        .expect("the future starts");
    assert!(matches!(poll_once(&mut call), Poll::Pending));
    assert!(
        host.take_service(23).is_some(),
        "the abandoned call left its timer request outstanding"
    );
    drop(call);
    assert_balanced(&host, "an abandoned call");
}

#[test]
fn a_wake_after_the_future_was_dropped_names_the_call_it_belongs_to() {
    let (host, instance) = new_instance();

    // A call that keeps one clone of the waker it is polled with and stays
    // pending. The clone belongs to the plugin, so it outlives the call.
    let mut deferred = instance
        .invoke("tool.deferred", 41, &request("hold"))
        .expect("the future starts");
    assert!(matches!(poll_once(&mut deferred), Poll::Pending));
    drop(deferred);
    assert!(
        host.wakes_after_drop().is_empty(),
        "no wake has been delivered yet"
    );

    // A later call wakes that kept clone. The call it belongs to is gone, so the
    // record has to name it.
    let mut woken = instance
        .invoke("tool.wake_deferred", 42, &request("wake"))
        .expect("the future starts");
    match poll_once(&mut woken) {
        Poll::Ready(Ok(body)) => {
            assert_eq!(support::decode(&body).echo, "woke the deferred call")
        }
        Poll::Ready(Err(error)) => panic!("the waker capability answers: {error:?}"),
        Poll::Pending => panic!("the waker capability answers on its first poll"),
    }

    assert_eq!(
        host.wakes_after_drop(),
        vec![41],
        "the record names the call whose future was dropped"
    );
    assert_eq!(
        host.waker_counts().2,
        1,
        "one wake arrived after a drop, and it is counted"
    );
    assert!(
        !host
            .ordering()
            .iter()
            .any(|event| event.contains("after the future was dropped")),
        "a counted violation, not an ordering-ledger entry: {:?}",
        host.ordering()
    );

    // The late wake consumed the reference the plugin was keeping, so the
    // accounting closes with it.
    assert_references_balanced(&host, "a wake after the future was dropped");
}
