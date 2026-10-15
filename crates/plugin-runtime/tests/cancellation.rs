//! Cancellation as it was decided: the caller owns the deadline and the drop.
//!
//! The runtime holds no timer and no default deadline. Cancelling marks the
//! plugin's token through the frozen table entry, an id the host does not hold
//! is answered as `NotFound` without the plugin being called, a call dropped
//! before the plugin acknowledged its cancel is recorded on the ordering ledger
//! and the instance keeps serving, and dropping a call releases what the host
//! still held for it so no completion is ever delivered into a released
//! context. The instance-wide form is a loop over the instance's own in-flight
//! set, and it waits for nothing: the caller polls and drops.

#[path = "support/mod.rs"]
mod support;

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use plugin_contract::abi::MetisStatus;
use plugin_runtime::{CallError, Host, PluginCall};

use support::{new_instance, request};

/// Polls one call once.
///
/// The waker is the no-op one: every poll here is either ready or parked on
/// something the test drives itself, so a wake has nothing to schedule. A real
/// daemon passes its own task waker, which is the same interface.
fn poll_once(call: &mut PluginCall) -> Poll<Result<Vec<u8>, CallError>> {
    let mut context = Context::from_waker(Waker::noop());
    Pin::new(call).poll(&mut context)
}

/// Whether the plugin's own record shows that a cancel reached it.
///
/// The authoring adapter logs an accepted cancel through the host callback, so
/// this is the plugin side of "the host did or did not call the plugin".
fn plugin_saw_cancel(host: &Host, request_id: u64) -> bool {
    host.logs()
        .iter()
        .any(|line| line.contains(&format!("cancel accepted for request {request_id}")))
}

#[test]
fn cancelling_an_unknown_or_finished_id_answers_not_found_without_calling_the_plugin() {
    let (host, instance) = new_instance();
    let (other_host, other) = new_instance();

    // An id nobody holds at all.
    assert_eq!(
        instance.cancel(999).unwrap_err(),
        CallError::Status(MetisStatus::NotFound),
        "an id the host does not hold answers the frozen not-found status"
    );
    assert!(
        !plugin_saw_cancel(&host, 999),
        "the plugin was not called: {:?}",
        host.logs()
    );

    // An id another instance holds. The plugin's own call registry is
    // process-wide, so entering the plugin with this id would mark the other
    // instance's token: the per-instance in-flight set is what refuses it.
    let mut live = other
        .invoke("tool.cancellable", 7, &request("cancel"))
        .expect("the future starts");
    assert!(matches!(poll_once(&mut live), Poll::Pending));
    assert_eq!(
        instance.cancel(7).unwrap_err(),
        CallError::Status(MetisStatus::NotFound),
        "an id another instance holds is not an id this instance holds"
    );
    assert!(
        !plugin_saw_cancel(&other_host, 7),
        "another instance's cancel never reached the plugin: {:?}",
        other_host.logs()
    );
    assert!(
        matches!(poll_once(&mut live), Poll::Pending),
        "another instance's token is still unmarked"
    );

    // A live id here: a first and a second cancel are both accepted.
    let mut call = instance
        .invoke("tool.cancellable", 12, &request("cancel"))
        .expect("the future starts");
    assert!(matches!(poll_once(&mut call), Poll::Pending));
    instance.cancel(12).expect("the first cancel is accepted");
    instance
        .cancel(12)
        .expect("a second cancel of a live id is accepted");
    match poll_once(&mut call) {
        Poll::Ready(Err(CallError::Status(MetisStatus::Cancelled))) => {}
        Poll::Ready(Err(error)) => panic!("the cancel still resolves the call: {error:?}"),
        Poll::Ready(Ok(_)) => panic!("the cancel still resolves the call as cancelled"),
        Poll::Pending => panic!("the cancel still resolves the call"),
    }
    drop(call);

    // An already finished id: the call is gone, so the host no longer holds it.
    assert_eq!(
        instance.cancel(12).unwrap_err(),
        CallError::Status(MetisStatus::NotFound),
        "an already finished call is not an id the host holds"
    );
    drop(live);
}

#[test]
fn a_cancelled_call_reaches_its_caller_as_the_frozen_cancelled_status() {
    let (host, instance) = new_instance();
    let mut call = instance
        .invoke("tool.cancellable", 11, &request("cancel"))
        .expect("the future starts");
    assert!(
        matches!(poll_once(&mut call), Poll::Pending),
        "the cancellable capability parks on its token"
    );

    instance
        .cancel(11)
        .expect("the host holds the id, so the plugin marks its token");
    assert!(
        plugin_saw_cancel(&host, 11),
        "the accepted cancel reached the plugin: {:?}",
        host.logs()
    );

    match poll_once(&mut call) {
        Poll::Ready(Err(CallError::Status(MetisStatus::Cancelled))) => {}
        Poll::Ready(Err(error)) => panic!("a cancelled call resolves as cancelled: {error:?}"),
        Poll::Ready(Ok(_)) => panic!("a cancelled call resolves as cancelled, not as an answer"),
        Poll::Pending => panic!("a cancelled call resolves on its next poll"),
    }
    drop(call);
    assert!(
        !host
            .ordering()
            .iter()
            .any(|event| event.contains("unacknowledged")),
        "the plugin acknowledged the cancel, so nothing is recorded: {:?}",
        host.ordering()
    );
}
