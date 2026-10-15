//! The plugin's side of the future seam: a boxed author future behind the
//! `MetisFutureHandle` the host polls.
//!
//! This is the crate's own replacement for `async-ffi`, and it is deliberately
//! small. The part that matters is the waker bridge: the host hands a
//! `MetisWaker` to `poll`, and this module wraps one clone of it in a
//! `std::task::Waker` whose every operation forwards back to the host's vtable.
//! The author's future therefore sees an ordinary Rust executor interface, and
//! the host sees an ordinary C poll.
//!
//! Panic containment is here too, because this module owns both boundary
//! frames. A panic inside the author's `poll` becomes a terminal `Internal`, and
//! a panic inside the author's destructor is caught in the same way, so no path
//! from a plugin future reaches `process::abort`. That is the one measured
//! reason this crate does not use `async-ffi`: a destructor panic inside its
//! future aborts the process for a plugin that returned `Ok`.
//!
//! There is no executor here and no thread. The host polls, and this module
//! only answers.

use std::collections::HashSet;
use std::ffi::c_void;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use crate::abi::{
    MetisBuffer, MetisFutureDropFn, MetisFutureHandle, MetisFuturePollFn, MetisFutureVtable,
    MetisStatus, MetisWaker,
};
use crate::authoring::alloc::{plugin_alloc, violation};
use crate::authoring::traits::PluginError;
use crate::payload::metis_encode_payload;

/// What an author's erased future resolves to.
///
/// Two futures cross this boundary: a capability answers with a response body,
/// and the initialisation entry resolves with nothing but its success, because
/// its out parameters were filled when the entry returned. One enum keeps one
/// context, one vtable and one panic-containment path for both.
pub(crate) enum FutureOutcome {
    /// A capability answered with these response body bytes.
    Response(Vec<u8>),
    /// An instance finished initialising.
    Initialised,
}

/// What the adapter boxes: the author's future, erased.
pub(crate) type ErasedFuture =
    Pin<Box<dyn Future<Output = Result<FutureOutcome, PluginError>> + Send + 'static>>;

// --- the waker bridge ------------------------------------------------------

fn clone_waker(waker: &MetisWaker) -> MetisWaker {
    if waker.vtable.is_null() {
        return MetisWaker::null();
    }
    // SAFETY: the vtable is the host's live one, and a borrowed waker may be
    // cloned; the clone is one owning waker.
    unsafe { ((*waker.vtable).clone)(waker) }
}

/// Releases an owned clone by waking it, which is how the `wake` entry is
/// defined to consume its argument.
fn wake_waker(waker: MetisWaker) {
    if waker.vtable.is_null() {
        return;
    }
    // SAFETY: `waker` is an owned clone; `wake` consumes it.
    unsafe { ((*waker.vtable).wake)(waker) };
}

fn wake_waker_by_ref(waker: &MetisWaker) {
    if waker.vtable.is_null() {
        return;
    }
    // SAFETY: the vtable is the host's live one and the handle stays borrowed.
    unsafe { ((*waker.vtable).wake_by_ref)(waker) };
}

fn drop_waker(waker: MetisWaker) {
    if waker.vtable.is_null() {
        return;
    }
    // SAFETY: `waker` is an owned clone; `drop` consumes it without waking.
    unsafe { ((*waker.vtable).drop)(waker) };
}

/// One owned host waker, as the data of a `std::task::Waker`.
///
/// It has no `Drop` impl on purpose: the destructor is the `drop` half of the
/// raw vtable below, which moves the owned waker out before the box is
/// deallocated, so each clone is released exactly once and through the entry
/// the host chose.
struct OwnedWaker {
    waker: MetisWaker,
}

// SAFETY: every operation forwards to the host's own thread-safe vtable, whose
// contract is that `wake` may be called from any thread.
unsafe impl Send for OwnedWaker {}
unsafe impl Sync for OwnedWaker {}

unsafe fn raw_waker_clone(data: *const ()) -> RawWaker {
    // SAFETY: `data` is an `OwnedWaker` box owned by the raw waker.
    let owned = unsafe { &*(data as *const OwnedWaker) };
    raw_waker(clone_waker(&owned.waker))
}

unsafe fn raw_waker_wake(data: *const ()) {
    // SAFETY: a `wake` consumes the box.
    let owned = unsafe { Box::from_raw(data as *mut OwnedWaker) };
    let waker = owned.waker;
    wake_waker(waker);
}

unsafe fn raw_waker_wake_by_ref(data: *const ()) {
    // SAFETY: `data` is a live `OwnedWaker` and the handle stays borrowed.
    let owned = unsafe { &*(data as *const OwnedWaker) };
    wake_waker_by_ref(&owned.waker);
}

unsafe fn raw_waker_drop(data: *const ()) {
    // SAFETY: a `drop` consumes the box.
    let owned = unsafe { Box::from_raw(data as *mut OwnedWaker) };
    let waker = owned.waker;
    drop_waker(waker);
}

static STD_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
    raw_waker_clone,
    raw_waker_wake,
    raw_waker_wake_by_ref,
    raw_waker_drop,
);

fn raw_waker(owned: MetisWaker) -> RawWaker {
    let boxed = Box::new(OwnedWaker { waker: owned });
    RawWaker::new(Box::into_raw(boxed) as *const (), &STD_WAKER_VTABLE)
}

/// Wraps the borrowed host waker in a `std::task::Waker` that owns exactly one
/// host clone and forwards every operation back to the host.
fn host_waker_to_std(waker: &MetisWaker) -> Waker {
    // SAFETY: `raw_waker` owns the box, and the vtable above is sound.
    unsafe { Waker::from_raw(raw_waker(clone_waker(waker))) }
}

// --- the future context ----------------------------------------------------

/// One in-flight future, as the host sees it: a poll entry and a drop entry.
struct FutureCtx {
    /// Taken when the future resolves, so a terminal context holds no future
    /// and its destructor cannot run twice.
    future: Option<ErasedFuture>,
    /// The status the future became terminal with, if it has.
    terminal: Option<MetisStatus>,
    /// The request id, so dropping the context can leave the adapter's
    /// cancellation registry.
    request_id: u64,
    /// Called with the request id when the context is dropped, so the registry
    /// the adapter keeps does not outlive the call.
    on_release: fn(u64),
}

/// The contexts this plugin has handed out and not seen back, so a second drop
/// of one is a recorded violation rather than a second `Box::from_raw`.
fn live_contexts() -> &'static Mutex<HashSet<usize>> {
    static LIVE: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Boxes one author future and hands it to the host.
pub(crate) fn new_handle(
    future: ErasedFuture,
    request_id: u64,
    on_release: fn(u64),
) -> MetisFutureHandle {
    let ctx = Box::new(FutureCtx {
        future: Some(future),
        terminal: None,
        request_id,
        on_release,
    });
    let raw = Box::into_raw(ctx);
    live_contexts().lock().unwrap().insert(raw as usize);
    MetisFutureHandle {
        ctx: raw as *mut c_void,
        vtable: &FUTURE_VTABLE,
    }
}

fn future_poll_body(
    ctx: *mut c_void,
    waker: MetisWaker,
    out_value: *mut MetisBuffer,
) -> MetisStatus {
    if ctx.is_null() || out_value.is_null() {
        return MetisStatus::InvalidArgument;
    }
    // SAFETY: the caller (the vtable below) guarantees `ctx` came from
    // `new_handle` and that it stays alive until `drop`.
    let ctx = unsafe { &mut *(ctx as *mut FutureCtx) };
    if let Some(status) = ctx.terminal {
        return match status {
            // A second ready answer cannot exist: the buffer was handed to the
            // host once and is the host's to release.
            MetisStatus::Ok => {
                violation("a terminal future that already returned Ok was polled again");
                MetisStatus::Internal
            }
            terminal => terminal,
        };
    }
    let std_waker = host_waker_to_std(&waker);
    let mut context = Context::from_waker(&std_waker);
    let Some(future) = ctx.future.as_mut() else {
        violation("a future context was polled with no future left in it");
        return MetisStatus::Internal;
    };
    // SAFETY: the future lives in the boxed context, so it never moves, and the
    // context is only reachable through this one `&mut`.
    let pinned = unsafe { Pin::new_unchecked(future) };
    match Future::poll(pinned, &mut context) {
        Poll::Ready(Ok(outcome)) => {
            ctx.terminal = Some(MetisStatus::Ok);
            ctx.future = None;
            let buffer = match outcome {
                // The response body is wrapped in the envelope and handed to
                // the host, which releases it through the table's
                // `release_buffer`.
                FutureOutcome::Response(body) => plugin_alloc(&metis_encode_payload(&body)),
                // Initialisation has no value to hand back here: the entry
                // returned the instance and the future handle already, and this
                // `Ok` is what makes the instance valid.
                FutureOutcome::Initialised => MetisBuffer::empty(),
            };
            // SAFETY: the host owns the out parameter and releases the buffer
            // through the table's `release_buffer`.
            unsafe { *out_value = buffer };
            MetisStatus::Ok
        }
        Poll::Ready(Err(error)) => {
            let status = crate::authoring::plugin_adapter::metis_status(&error);
            ctx.terminal = Some(status);
            ctx.future = None;
            status
        }
        Poll::Pending => MetisStatus::Pending,
    }
}

/// The drop entry: releases the context, and with it the author's future.
///
/// A panicking destructor is caught here and reported as `Internal`, which is
/// what keeps a plugin's `Drop` bug from reaching `process::abort`.
fn future_drop_body(ctx: *mut c_void) -> MetisStatus {
    if ctx.is_null() {
        return MetisStatus::InvalidArgument;
    }
    if !live_contexts().lock().unwrap().remove(&(ctx as usize)) {
        // The host is required to drop a future exactly once. A second drop of
        // the same context is a protocol violation, and it is reported instead
        // of being taken back into a `Box` a second time.
        violation("a future context that was already dropped was dropped again");
        return MetisStatus::NotFound;
    }
    // SAFETY: `ctx` came from `new_handle`, and the check above makes this path
    // run once per context.
    let ctx = unsafe { Box::from_raw(ctx as *mut FutureCtx) };
    (ctx.on_release)(ctx.request_id);
    // Dropping the context drops the author's future, and any waker clone the
    // future stored is released here through the host's own `drop` entry.
    drop(ctx);
    MetisStatus::Ok
}

/// The exported poll: the boundary catches a panic inside the author's future
/// and turns it into a terminal `Internal`.
unsafe extern "C" fn future_poll(
    ctx: *mut c_void,
    waker: MetisWaker,
    out_value: *mut MetisBuffer,
) -> MetisStatus {
    catch_unwind(AssertUnwindSafe(|| future_poll_body(ctx, waker, out_value)))
        .unwrap_or(MetisStatus::Internal)
}

/// The exported drop: a panic inside the author's destructor is contained here
/// rather than allowed to abort the process.
unsafe extern "C" fn future_drop(ctx: *mut c_void) -> MetisStatus {
    catch_unwind(AssertUnwindSafe(|| future_drop_body(ctx))).unwrap_or(MetisStatus::Internal)
}

/// The one future vtable this crate installs. It carries no `size` field: the
/// handle is the only route to it.
static FUTURE_VTABLE: MetisFutureVtable = MetisFutureVtable {
    poll: future_poll as MetisFuturePollFn,
    drop: future_drop as MetisFutureDropFn,
};
