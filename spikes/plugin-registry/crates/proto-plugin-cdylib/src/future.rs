//! PROTOTYPE (throwaway). Not product code.
//!
//! The host-polled future seam, plugin side. This is the folded version: lane
//! 1's two future objects, lane 2's hazards, and lane 3's self-woken probe.
//!
//! Two implementations of the same `MetisFutureVtable`, both required:
//!
//! - the async-ffi object ([`FfiCtx`], lane 1): an ordinary `async` block
//!   wrapped in `async_ffi::FfiFuture` by the plugin adapter, so the author
//!   writes async Rust. Nothing from `async-ffi` crosses the boundary; the
//!   boundary only sees the opaque `ctx` and our vtable.
//! - the hand-written object ([`HandCtx`], lane 1): a plain phase machine over
//!   `ctx` plus the host waker vtable, with no `async-ffi` and no Rust
//!   `Future` anywhere. It stands in for a non-Rust implementer, and it is the
//!   object the lane 2 hazards and the lane 3 probe are expressed in.
//!
//! In neither object does the plugin spawn a thread or own a runtime on the
//! normal path. The plugin's `invoke_future` builds the object, the host polls
//! it, and the host's timer thread calls the plugin's `service_complete` to
//! resolve it, waking the host through the waker the host handed in. The host
//! is the only executor. The only threads in this file are the deliberate
//! hazard threads (a stale wake, a probe self-wake) and the blocking-poll
//! stand-in; none is part of the ordinary path.
//!
//! The waker bridge: the host passes a `MetisWaker` by value (borrowed, as the
//! spec says). To hand it to a Rust `Future`, this module clones it into an
//! owned handle and wraps it in a `std::task::Waker` whose raw vtable forwards
//! clone, wake, wake_by_ref and drop straight back to the host. Each clone is a
//! host `clone` call; each release is a host `wake` or `drop`. The counters
//! below prove the two balance.

use std::collections::HashMap;
use std::ffi::c_void;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::thread;
use std::time::{Duration, Instant};

use async_ffi::{FfiFuture, FutureExt};
use serde_json::json;

use proto_abi::{
    METIS_BUFFER_OWNER_HOST, METIS_BUFFER_OWNER_PLUGIN, MetisBuffer, MetisFutureHandle,
    MetisFutureVtable, MetisHostApiV1, MetisStatus, MetisWaker, metis_encode_payload,
    metis_payload_body,
};

use crate::{plugin_alloc, release_buffer};

/// How long the deliberately blocking poll blocks, comfortably past the host's
/// detection deadline.
const BLOCKING_POLL_MS: u64 = 800;

/// How long the stale-wake thread waits before waking its clone, comfortably
/// after the host has dropped the future.
const STALE_WAKE_MS: u64 = 250;

/// How long the self-woken probe waits before resolving.
const PROBE_DELAY_MS: u64 = 2;

// --- instrumentation -------------------------------------------------------
//
// The seam's own proof that the plugin is runtime free, and the numbers the
// cost section reads back. All process global, all reset never (the harness
// reads deltas if it cares).

static THREADS_SPAWNED: AtomicUsize = AtomicUsize::new(0);
static POLL_CALLS: AtomicUsize = AtomicUsize::new(0);
static POLL_FIRST_HASH: AtomicU64 = AtomicU64::new(0);
static POLL_OTHER_THREADS: AtomicUsize = AtomicUsize::new(0);
static WAKER_CLONES: AtomicUsize = AtomicUsize::new(0);
static WAKER_RELEASES: AtomicUsize = AtomicUsize::new(0);
static FUTURES_CREATED: AtomicUsize = AtomicUsize::new(0);
static FUTURES_DROPPED: AtomicUsize = AtomicUsize::new(0);
static SERVICE_ROUNDTRIP_NS: AtomicU64 = AtomicU64::new(0);
static SERVICE_ROUNDTRIP_TOTAL_NS: AtomicU64 = AtomicU64::new(0);
static SERVICE_ROUNDTRIP_COUNT: AtomicUsize = AtomicUsize::new(0);
static CANCEL_RESOLVED: AtomicUsize = AtomicUsize::new(0);

fn thread_hash() -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::thread::current().id().hash(&mut hasher);
    hasher.finish() | 1
}

fn record_poll() {
    POLL_CALLS.fetch_add(1, SeqCst);
    let me = thread_hash();
    match POLL_FIRST_HASH.compare_exchange(0, me, SeqCst, SeqCst) {
        Ok(_) => {}
        Err(first) => {
            if first != me {
                POLL_OTHER_THREADS.fetch_add(1, SeqCst);
            }
        }
    }
}

fn record_service_roundtrip(request_at: Option<Instant>) {
    let Some(started) = request_at else {
        return;
    };
    let elapsed = Instant::now().duration_since(started).as_nanos() as u64;
    SERVICE_ROUNDTRIP_NS.store(elapsed, SeqCst);
    SERVICE_ROUNDTRIP_TOTAL_NS.fetch_add(elapsed, SeqCst);
    SERVICE_ROUNDTRIP_COUNT.fetch_add(1, SeqCst);
}

/// The counters the host reads back, through the host-polled `metis.future_counters`
/// capability or the legacy `metis.future_stats` one.
pub(crate) fn stats_json() -> serde_json::Value {
    let poll_threads = if POLL_CALLS.load(SeqCst) == 0 {
        0
    } else {
        1 + POLL_OTHER_THREADS.load(SeqCst)
    };
    json!({
        "threads_spawned": THREADS_SPAWNED.load(SeqCst),
        "poll_calls": POLL_CALLS.load(SeqCst),
        "poll_threads": poll_threads,
        "waker_clones": WAKER_CLONES.load(SeqCst),
        "waker_releases": WAKER_RELEASES.load(SeqCst),
        "futures_created": FUTURES_CREATED.load(SeqCst),
        "futures_dropped": FUTURES_DROPPED.load(SeqCst),
        "service_roundtrip_ns": SERVICE_ROUNDTRIP_NS.load(SeqCst),
        "service_roundtrip_total_ns": SERVICE_ROUNDTRIP_TOTAL_NS.load(SeqCst),
        "service_roundtrip_count": SERVICE_ROUNDTRIP_COUNT.load(SeqCst),
        "cancel_resolved": CANCEL_RESOLVED.load(SeqCst),
    })
}

/// The legacy worker path calls this at every `thread::spawn`, so the
/// future-path proof (no spawn) is measured against a counter the rest of the
/// library does move.
pub(crate) fn note_thread_spawned() {
    THREADS_SPAWNED.fetch_add(1, SeqCst);
}

// --- the waker bridge ------------------------------------------------------

unsafe fn clone_waker(waker: &MetisWaker) -> MetisWaker {
    if waker.vtable.is_null() {
        return MetisWaker::null();
    }
    WAKER_CLONES.fetch_add(1, SeqCst);
    // SAFETY: the vtable is the host's live one.
    unsafe { ((*waker.vtable).clone)(waker) }
}

unsafe fn release_waker(waker: MetisWaker) {
    if waker.vtable.is_null() {
        return;
    }
    WAKER_RELEASES.fetch_add(1, SeqCst);
    // SAFETY: `waker` is an owned clone; `wake` consumes and releases it.
    unsafe { ((*waker.vtable).wake)(waker) };
}

unsafe fn drop_waker(waker: MetisWaker) {
    if waker.vtable.is_null() {
        return;
    }
    WAKER_RELEASES.fetch_add(1, SeqCst);
    // SAFETY: `waker` is an owned clone; `drop` consumes and releases it.
    unsafe { ((*waker.vtable).drop)(waker) };
}

unsafe fn wake_waker_by_ref(waker: &MetisWaker) {
    if waker.vtable.is_null() {
        return;
    }
    // SAFETY: the vtable is the host's live one and the handle stays borrowed.
    unsafe { ((*waker.vtable).wake_by_ref)(waker) };
}

/// One owned host waker, as the data of a `std::task::Waker`.
struct OwnedWaker {
    waker: MetisWaker,
}

// SAFETY: every operation forwards to the host's own thread-safe vtable.
unsafe impl Send for OwnedWaker {}
unsafe impl Sync for OwnedWaker {}

unsafe fn raw_waker_clone(data: *const ()) -> RawWaker {
    // SAFETY: `data` is an `OwnedWaker` box owned by the raw waker.
    let owned = unsafe { &*(data as *const OwnedWaker) };
    // SAFETY: `owned` lives for the call.
    let cloned = unsafe { clone_waker(&owned.waker) };
    raw_waker(cloned)
}

unsafe fn raw_waker_wake(data: *const ()) {
    // SAFETY: a `wake` consumes the box.
    let owned = unsafe { Box::from_raw(data as *mut OwnedWaker) };
    // SAFETY: consuming call; releases the owned clone.
    unsafe { release_waker(owned.waker) };
}

unsafe fn raw_waker_wake_by_ref(data: *const ()) {
    // SAFETY: `data` is a live `OwnedWaker`.
    let owned = unsafe { &*(data as *const OwnedWaker) };
    // SAFETY: borrowed call.
    unsafe { wake_waker_by_ref(&owned.waker) };
}

unsafe fn raw_waker_drop(data: *const ()) {
    // SAFETY: a drop consumes the box; releasing the clone.
    let owned = unsafe { Box::from_raw(data as *mut OwnedWaker) };
    unsafe { drop_waker(owned.waker) };
}

static STD_WAKER_VTABLE: RawWakerVTable =
    RawWakerVTable::new(raw_waker_clone, raw_waker_wake, raw_waker_wake_by_ref, raw_waker_drop);

fn raw_waker(owned: MetisWaker) -> RawWaker {
    let boxed = Box::new(OwnedWaker { waker: owned });
    RawWaker::new(Box::into_raw(boxed) as *const (), &STD_WAKER_VTABLE)
}

/// Wraps the borrowed host waker in a Rust `Waker` that owns exactly one host
/// clone and forwards every operation back to the host.
fn host_waker_to_std(waker: MetisWaker) -> Waker {
    // SAFETY: cloning the borrowed handle gives an owned one.
    let owned = unsafe { clone_waker(&waker) };
    // SAFETY: `raw_waker` owns the box and the vtable is sound.
    unsafe { Waker::from_raw(raw_waker(owned)) }
}

/// A host waker made `Send` so a plugin hazard thread may wake it. The host
/// waker is explicitly valid on any thread, and the reference it carries is a
/// host `Arc`, so this is the plugin side of that same promise.
struct SendWaker(MetisWaker);
// SAFETY: the host waker's contract is that `wake` may be called from any
// thread; the data behind it is a host refcount, not thread-affine state.
unsafe impl Send for SendWaker {}

impl SendWaker {
    /// Wakes through the vtable, consuming the clone. Defined as a method so a
    /// spawned closure captures the `Send` wrapper and not its pointer field.
    fn wake(self) {
        if !self.0.vtable.is_null() {
            // SAFETY: `wake` consumes the waker; the clone is the plugin's.
            unsafe { ((*self.0.vtable).wake)(self.0) }
        }
    }
}

// --- request issuing -------------------------------------------------------

/// Issues one `service_request` for a timer. The request payload is a
/// plugin-owned buffer, which the host releases.
unsafe fn issue_timer_request(host: *const MetisHostApiV1, request_id: u64, delay_ms: u64) {
    if host.is_null() {
        return;
    }
    // SAFETY: `host` is the table the host passed to `invoke_future`.
    let host = unsafe { &*host };
    let covered =
        host.size >= core::mem::offset_of!(MetisHostApiV1, service_request) + size_of::<usize>();
    if !covered {
        return;
    }
    let body = serde_json::to_vec(&json!({ "delay_ms": delay_ms })).unwrap_or_default();
    let buffer = plugin_alloc(&metis_encode_payload(&body));
    let service = b"timer";
    // SAFETY: the callback signature is the frozen appended one.
    let _ = unsafe {
        (host.service_request)(
            host.user_data,
            service.as_ptr(),
            service.len(),
            request_id,
            buffer,
        )
    };
}

// --- future registry -------------------------------------------------------

#[derive(Clone)]
enum PendingFuture {
    Ffi(Arc<FfiShared>),
    Hand(Arc<Mutex<HandState>>),
}

fn pending() -> &'static Mutex<HashMap<u64, PendingFuture>> {
    static PENDING: OnceLock<Mutex<HashMap<u64, PendingFuture>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolves a future with `Cancelled` and wakes its host task. The future is
/// dropped by the host afterwards, never mid-flight.
pub(crate) fn cancel(request_id: u64) -> bool {
    let entry = pending().lock().unwrap().get(&request_id).cloned();
    match entry {
        Some(PendingFuture::Ffi(shared)) => {
            *shared.done.lock().unwrap() = Some((MetisStatus::Cancelled, Vec::new()));
            if let Some(waker) = shared.waker.lock().unwrap().take() {
                waker.wake();
            }
            CANCEL_RESOLVED.fetch_add(1, SeqCst);
            true
        }
        Some(PendingFuture::Hand(state)) => {
            let waker = {
                let mut state = state.lock().unwrap();
                // Mark the token; a cancel-aware future observes it on its next
                // poll and resolves as `Cancelled`. A future that ignores the
                // token stays pending, which is the hazard the host detects by
                // deadline.
                state.cancelled = true;
                state.waker.take()
            };
            if let Some(waker) = waker {
                // SAFETY: an owned clone; `wake` consumes and releases it.
                unsafe { release_waker(waker) };
            }
            CANCEL_RESOLVED.fetch_add(1, SeqCst);
            true
        }
        None => false,
    }
}

// --- the async-ffi future (lane 1, ergonomic path) -------------------------

struct FfiShared {
    request_id: u64,
    delay_ms: u64,
    request_host: *const MetisHostApiV1,
    requested: AtomicBool,
    done: Mutex<Option<(MetisStatus, Vec<u8>)>>,
    waker: Mutex<Option<Waker>>,
    request_at: Mutex<Option<Instant>>,
}

// SAFETY: the host table is valid for the call and every field is otherwise
// `Send`; the futures map is keyed by the host's unique request id.
unsafe impl Send for FfiShared {}
unsafe impl Sync for FfiShared {}

struct ServiceWait {
    shared: Arc<FfiShared>,
}

impl Future for ServiceWait {
    type Output = Result<(), MetisStatus>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let shared = &self.shared;
        if let Some((status, _body)) = shared.done.lock().unwrap().take() {
            return if status == MetisStatus::Ok {
                Poll::Ready(Ok(()))
            } else {
                Poll::Ready(Err(status))
            };
        }
        // Register the waker before issuing the request, so an answer that
        // arrives immediately still finds a task to wake.
        *shared.waker.lock().unwrap() = Some(context.waker().clone());
        if !shared.requested.swap(true, SeqCst) {
            *shared.request_at.lock().unwrap() = Some(Instant::now());
            // SAFETY: the host table is the one passed to `invoke_future`.
            unsafe { issue_timer_request(shared.request_host, shared.request_id, shared.delay_ms) };
        }
        Poll::Pending
    }
}

struct FfiCtx {
    shared: Arc<FfiShared>,
    future: Option<FfiFuture<Result<Vec<u8>, MetisStatus>>>,
    terminal: bool,
}

enum FfiKind {
    Timer,
    Ready,
}

fn make_ffi(
    kind: FfiKind,
    request_id: u64,
    host: *const MetisHostApiV1,
    x: i64,
    delay_ms: u64,
    shim: bool,
) -> MetisFutureHandle {
    let shared = Arc::new(FfiShared {
        request_id,
        delay_ms,
        request_host: host,
        requested: AtomicBool::new(false),
        done: Mutex::new(None),
        waker: Mutex::new(None),
        request_at: Mutex::new(None),
    });
    let future: FfiFuture<Result<Vec<u8>, MetisStatus>> = match kind {
        FfiKind::Timer => {
            let shared = Arc::clone(&shared);
            async move {
                ServiceWait { shared }.await?;
                serde_json::to_vec(&json!({ "y": x })).map_err(|_| MetisStatus::Internal)
            }
            .into_ffi()
        }
        FfiKind::Ready => async move {
            serde_json::to_vec(&json!({ "y": x })).map_err(|_| MetisStatus::Internal)
        }
        .into_ffi(),
    };
    let ctx = Box::new(FfiCtx {
        shared: Arc::clone(&shared),
        future: Some(future),
        terminal: false,
    });
    pending()
        .lock()
        .unwrap()
        .insert(request_id, PendingFuture::Ffi(shared));
    FUTURES_CREATED.fetch_add(1, SeqCst);
    MetisFutureHandle {
        ctx: Box::into_raw(ctx) as *mut c_void,
        vtable: if shim { &FFI_VTABLE } else { &FFI_VTABLE_NO_SHIM },
    }
}

// SAFETY: `ctx` is an `FfiCtx` from `make_ffi`, live until `ffi_drop`.
unsafe fn ffi_poll_body(
    ctx: *mut c_void,
    waker: MetisWaker,
    out_value: *mut MetisBuffer,
) -> MetisStatus {
    if ctx.is_null() || out_value.is_null() {
        return MetisStatus::InvalidArgument;
    }
    record_poll();
    // SAFETY: the caller guarantees the ctx and out pointer.
    let ctx = unsafe { &mut *(ctx as *mut FfiCtx) };
    if ctx.terminal {
        return MetisStatus::Internal;
    }
    let std_waker = host_waker_to_std(waker);
    let mut cx = Context::from_waker(&std_waker);
    let Some(future) = ctx.future.as_mut() else {
        return MetisStatus::Internal;
    };
    // SAFETY: the future lives in the boxed ctx, so it is pinned for its whole
    // life and never moves.
    let pinned = unsafe { Pin::new_unchecked(future) };
    match Future::poll(pinned, &mut cx) {
        Poll::Ready(Ok(body)) => {
            ctx.terminal = true;
            // SAFETY: the host owns the out parameter and releases the buffer.
            unsafe { *out_value = plugin_alloc(&metis_encode_payload(&body)) };
            MetisStatus::Ok
        }
        Poll::Ready(Err(status)) => {
            ctx.terminal = true;
            status
        }
        Poll::Pending => MetisStatus::Pending,
    }
}

/// The shim: the boundary catches a panic in the author's future and turns it
/// into `Internal`, leaving the future terminal.
unsafe extern "C" fn ffi_poll_shim(
    ctx: *mut c_void,
    waker: MetisWaker,
    out_value: *mut MetisBuffer,
) -> MetisStatus {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the arguments come from the host's poll call.
        unsafe { ffi_poll_body(ctx, waker, out_value) }
    }));
    caught.unwrap_or(MetisStatus::Internal)
}

/// The same poll without the `catch_unwind` shim, so the shim's share is
/// measurable rather than argued.
unsafe extern "C" fn ffi_poll_no_shim(
    ctx: *mut c_void,
    waker: MetisWaker,
    out_value: *mut MetisBuffer,
) -> MetisStatus {
    // SAFETY: the arguments come from the host's poll call.
    unsafe { ffi_poll_body(ctx, waker, out_value) }
}

unsafe extern "C" fn ffi_drop(ctx: *mut c_void) -> MetisStatus {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        if ctx.is_null() {
            return MetisStatus::InvalidArgument;
        }
        // SAFETY: the ctx came from `make_ffi` and this runs exactly once.
        let ctx = unsafe { Box::from_raw(ctx as *mut FfiCtx) };
        FUTURES_DROPPED.fetch_add(1, SeqCst);
        pending().lock().unwrap().remove(&ctx.shared.request_id);
        // Dropping the box drops the `FfiFuture`, the async block's `Arc`, and
        // any stored waker clone, releasing every host reference it held.
        drop(ctx);
        MetisStatus::Ok
    }));
    caught.unwrap_or(MetisStatus::Internal)
}

static FFI_VTABLE: MetisFutureVtable = MetisFutureVtable {
    size: size_of::<MetisFutureVtable>(),
    poll: ffi_poll_shim,
    drop: ffi_drop,
};

static FFI_VTABLE_NO_SHIM: MetisFutureVtable = MetisFutureVtable {
    size: size_of::<MetisFutureVtable>(),
    poll: ffi_poll_no_shim,
    drop: ffi_drop,
};

// --- the hand-written future (lane 1, C stand-in) --------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum HandMode {
    /// Issues a host `service_request` timer, then answers `{ "y": x }`.
    Timer,
    /// Answers `{ "y": x }` on the first poll.
    AlwaysReady,
    /// Returns the seam counters as the payload. It is host-polled, so reading
    /// the counters does not itself spawn a thread, which is what makes the
    /// "no plugin thread" delta a real zero.
    Counters,
    /// Lane 3's versioning probe: self-woken after a short delay, answers
    /// `{ "echo": capability, "request_id": id, "request_bytes": n }`.
    Probe,
    /// Lane 2: the author's `poll` panics, so the shim's `catch_unwind` is
    /// driven and the future becomes terminal `Internal`.
    PanicPoll,
    /// Lane 2: the future holds a plugin buffer and its author `drop` panics;
    /// the drop shim must still release the held buffer.
    PanicDrop,
    /// Lane 2: the poll blocks, so the host's watchdog must detect it.
    Blocking,
    /// Lane 2: a clone of the host waker wakes after the future was dropped.
    StaleWake,
    /// Lane 2: pending until cancelled, then resolves as `Cancelled`.
    CancelAware,
    /// Lane 2: pending forever, ignoring the cancel token.
    IgnoreCancel,
    /// Lane 2's ready stand-in for the cost loops.
    Echo,
}

struct HandState {
    mode: HandMode,
    x: i64,
    delay_ms: u64,
    request_id: u64,
    request_bytes: usize,
    request_host: *const MetisHostApiV1,
    requested: bool,
    cancelled: bool,
    stale_started: bool,
    result: Option<MetisStatus>,
    waker: Option<MetisWaker>,
    request_at: Option<Instant>,
}

// SAFETY: only the host table pointer is not `Send`; it is valid for the call
// and the map is keyed by the host's unique request id.
unsafe impl Send for HandState {}
unsafe impl Sync for HandState {}

impl HandState {
    fn response_body(&self) -> Vec<u8> {
        let value = match self.mode {
            HandMode::Probe => json!({
                "echo": "metis.future_probe",
                "request_id": self.request_id,
                "request_bytes": self.request_bytes,
            }),
            _ => json!({ "y": self.x }),
        };
        serde_json::to_vec(&value).unwrap_or_default()
    }
}

struct HandCtx {
    shared: Arc<Mutex<HandState>>,
    request_id: u64,
    mode: HandMode,
    /// A plugin buffer the future still holds; the drop shim releases it even
    /// when the author's `Drop` panics (the PanicDrop hazard).
    held: Option<MetisBuffer>,
    terminal: bool,
}

fn make_hand(
    mode: HandMode,
    request_id: u64,
    host: *const MetisHostApiV1,
    x: i64,
    delay_ms: u64,
    request_bytes: usize,
) -> MetisFutureHandle {
    let shared = Arc::new(Mutex::new(HandState {
        mode,
        x,
        delay_ms,
        request_id,
        request_bytes,
        request_host: host,
        requested: false,
        cancelled: false,
        stale_started: false,
        result: None,
        waker: None,
        request_at: None,
    }));
    let ctx = Box::new(HandCtx {
        shared: Arc::clone(&shared),
        request_id,
        mode,
        held: None,
        terminal: false,
    });
    pending()
        .lock()
        .unwrap()
        .insert(request_id, PendingFuture::Hand(shared));
    FUTURES_CREATED.fetch_add(1, SeqCst);
    MetisFutureHandle {
        ctx: Box::into_raw(ctx) as *mut c_void,
        vtable: &HAND_VTABLE,
    }
}

/// A plain state machine over the opaque ctx plus the host waker vtable. No
/// `async`, no `async-ffi`, no Rust `Future`.
unsafe fn hand_poll_body(
    ctx: *mut c_void,
    waker: MetisWaker,
    out_value: *mut MetisBuffer,
) -> MetisStatus {
    if ctx.is_null() || out_value.is_null() {
        return MetisStatus::InvalidArgument;
    }
    record_poll();
    // SAFETY: the caller guarantees the ctx and out pointer.
    let ctx = unsafe { &mut *(ctx as *mut HandCtx) };
    if ctx.terminal {
        return MetisStatus::Internal;
    }
    match ctx.mode {
        HandMode::PanicPoll => panic!("author future panicked inside poll (hazard test)"),
        HandMode::Blocking => {
            // Deliberately blocks the poll. A real plugin must not do this; the
            // host detects it rather than hanging. It allocates nothing, so the
            // abandoned poll leaves no buffer behind.
            thread::sleep(Duration::from_millis(BLOCKING_POLL_MS));
            return MetisStatus::Pending;
        }
        _ => {}
    }
    let mut state = ctx.shared.lock().unwrap();
    match state.mode {
        HandMode::AlwaysReady | HandMode::Echo => {
            let body = state.response_body();
            drop(state);
            // SAFETY: the host owns the out parameter and releases the buffer.
            unsafe { *out_value = plugin_alloc(&metis_encode_payload(&body)) };
            // Deliberately not terminal: this is the reuse vtable, ready on
            // every poll so the create-versus-reuse cost can be measured.
            return MetisStatus::Ok;
        }
        HandMode::Counters => {
            let body = serde_json::to_vec(&stats_json()).unwrap_or_default();
            drop(state);
            // SAFETY: as above.
            unsafe { *out_value = plugin_alloc(&metis_encode_payload(&body)) };
            return MetisStatus::Ok;
        }
        HandMode::PanicDrop => {
            if ctx.held.is_none() {
                ctx.held = Some(plugin_alloc(&metis_encode_payload(b"held-by-future")));
            }
        }
        HandMode::StaleWake => {
            if !state.stale_started {
                state.stale_started = true;
                if !waker.vtable.is_null() {
                    // SAFETY: the waker is valid for the call; the clone is
                    // moved to the hazard thread, which wakes it after the
                    // host has dropped the future.
                    let clone = SendWaker(unsafe { clone_waker(&waker) });
                    thread::spawn(move || {
                        thread::sleep(Duration::from_millis(STALE_WAKE_MS));
                        clone.wake();
                    });
                }
            }
        }
        _ => {}
    }
    if state.result.is_some() {
        let status = state.result.take().unwrap();
        if status == MetisStatus::Ok {
            let body = state.response_body();
            drop(state);
            // SAFETY: the host owns the out parameter and releases the buffer.
            unsafe { *out_value = plugin_alloc(&metis_encode_payload(&body)) };
            ctx.terminal = true;
            return MetisStatus::Ok;
        }
        drop(state);
        ctx.terminal = true;
        return status;
    }
    if state.mode == HandMode::CancelAware && state.cancelled {
        drop(state);
        ctx.terminal = true;
        return MetisStatus::Cancelled;
    }
    // Not ready: keep the freshest host waker and issue work exactly once. The
    // refresh replaces an older clone, releasing it.
    if let Some(old) = state.waker.take() {
        // SAFETY: an owned clone.
        unsafe { drop_waker(old) };
    }
    // SAFETY: cloning the borrowed host handle gives an owned one.
    state.waker = Some(unsafe { clone_waker(&waker) });
    let (issue, host, delay) = if state.requested {
        (None, std::ptr::null(), 0u64)
    } else {
        state.requested = true;
        state.request_at = Some(Instant::now());
        (Some(state.mode), state.request_host, state.delay_ms)
    };
    let shared = Arc::clone(&ctx.shared);
    drop(state);
    match issue {
        Some(HandMode::Timer) => {
            // SAFETY: the host table is the one passed to `invoke_future`.
            unsafe { issue_timer_request(host, ctx.request_id, delay) };
        }
        Some(HandMode::Probe) => {
            // A stand-in for a host service the plugin could not issue: the
            // plugin self-resolves after a short delay and wakes the host. The
            // clone is taken from the state so the wake still resolves the
            // future the host is polling.
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(PROBE_DELAY_MS));
                let waker = {
                    let mut state = shared.lock().unwrap();
                    state.result = Some(MetisStatus::Ok);
                    state.waker.take()
                };
                if let Some(waker) = waker {
                    // SAFETY: an owned clone; `wake` consumes and releases it.
                    unsafe { release_waker(waker) };
                }
            });
        }
        _ => {}
    }
    MetisStatus::Pending
}

unsafe extern "C" fn hand_poll(
    ctx: *mut c_void,
    waker: MetisWaker,
    out_value: *mut MetisBuffer,
) -> MetisStatus {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the arguments come from the host's poll call.
        unsafe { hand_poll_body(ctx, waker, out_value) }
    }));
    caught.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn hand_drop(ctx: *mut c_void) -> MetisStatus {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        if ctx.is_null() {
            return MetisStatus::InvalidArgument;
        }
        // SAFETY: the ctx came from `make_hand` and this runs exactly once.
        let mut ctx = unsafe { Box::from_raw(ctx as *mut HandCtx) };
        FUTURES_DROPPED.fetch_add(1, SeqCst);
        // Take what the shim owns out before the author's `Drop` runs, so a
        // panic inside that `Drop` cannot lose the buffer or the waker.
        let held = ctx.held.take();
        let stored = {
            if let Ok(mut state) = ctx.shared.lock() {
                state.waker.take()
            } else {
                None
            }
        };
        let mode = ctx.mode;
        // The author's `Drop` is a boundary (hazard PanicDrop panics here).
        let author_drop = catch_unwind(AssertUnwindSafe(|| {
            if mode == HandMode::PanicDrop {
                panic!("author future panicked inside drop (hazard test)");
            }
        }));
        // Release the buffer the future still held, whatever the author did.
        if let Some(buffer) = held {
            let _ = release_buffer(buffer);
        }
        if let Some(waker) = stored {
            // SAFETY: an owned clone.
            unsafe { drop_waker(waker) };
        }
        pending().lock().unwrap().remove(&ctx.request_id);
        drop(ctx);
        // The author-drop panic is contained here; `drop` remains a success and
        // the future is gone, so the host sees no abort and no double free.
        let _ = author_drop;
        MetisStatus::Ok
    }));
    caught.unwrap_or(MetisStatus::Internal)
}

static HAND_VTABLE: MetisFutureVtable = MetisFutureVtable {
    size: size_of::<MetisFutureVtable>(),
    poll: hand_poll,
    drop: hand_drop,
};

// --- the exported entries --------------------------------------------------

/// Appended plugin table entry: build a host-polled future for a capability.
pub(crate) unsafe extern "C" fn invoke_future(
    instance: *mut c_void,
    capability: *const u8,
    capability_len: usize,
    request_id: u64,
    payload: MetisBuffer,
    host: *const MetisHostApiV1,
    out: *mut MetisFutureHandle,
) -> MetisStatus {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null()
            || capability.is_null()
            || out.is_null()
            || host.is_null()
            || payload.owner != METIS_BUFFER_OWNER_HOST
        {
            return MetisStatus::InvalidArgument;
        }
        // SAFETY: the host passes a UTF-8 capability name for the call.
        let capability = match std::str::from_utf8(unsafe {
            std::slice::from_raw_parts(capability, capability_len)
        }) {
            Ok(name) => name,
            Err(_) => return MetisStatus::InvalidArgument,
        };
        // SAFETY: the host owns the request buffer for the call.
        let raw = unsafe { std::slice::from_raw_parts(payload.ptr, payload.len) };
        let body = match metis_payload_body(raw) {
            Ok(body) => body,
            Err(status) => return status,
        };
        #[derive(serde::Deserialize)]
        struct Request {
            #[serde(default)]
            x: i64,
            #[serde(default)]
            delay_ms: u64,
        }
        let request: Request = match serde_json::from_slice(body) {
            Ok(request) => request,
            Err(_) => return MetisStatus::InvalidArgument,
        };
        let request_bytes = body.len();
        let handle = match capability {
            "tool.poll_future" => make_ffi(
                FfiKind::Timer,
                request_id,
                host,
                request.x,
                request.delay_ms,
                true,
            ),
            "tool.future_ready" => make_ffi(
                FfiKind::Ready,
                request_id,
                host,
                request.x,
                request.delay_ms,
                true,
            ),
            "tool.future_ready_noshim" => make_ffi(
                FfiKind::Ready,
                request_id,
                host,
                request.x,
                request.delay_ms,
                false,
            ),
            "tool.hand_future" => make_hand(
                HandMode::Timer,
                request_id,
                host,
                request.x,
                request.delay_ms,
                request_bytes,
            ),
            "tool.future_reuse" => make_hand(
                HandMode::AlwaysReady,
                request_id,
                host,
                request.x,
                request.delay_ms,
                request_bytes,
            ),
            // Host-internal, deliberately not advertised: the harness reads the
            // seam counters through the seam itself, so the read spawns nothing.
            "metis.future_counters" => make_hand(
                HandMode::Counters,
                request_id,
                host,
                0,
                0,
                request_bytes,
            ),
            // Lane 3's versioning probe: self-woken, no host service needed, so
            // an older host table (no `service_request`) still drives it.
            "metis.future_probe" => make_hand(
                HandMode::Probe,
                request_id,
                host,
                request.x,
                request.delay_ms,
                request_bytes,
            ),
            // Lane 2 hazards, expressed in the same hand-written object.
            "future.echo" => make_hand(
                HandMode::Echo,
                request_id,
                host,
                request.x,
                request.delay_ms,
                request_bytes,
            ),
            "future.timer" => make_hand(
                HandMode::Timer,
                request_id,
                host,
                request.x,
                request.delay_ms,
                request_bytes,
            ),
            "future.panic_poll" => make_hand(
                HandMode::PanicPoll,
                request_id,
                host,
                request.x,
                request.delay_ms,
                request_bytes,
            ),
            "future.panic_drop" => make_hand(
                HandMode::PanicDrop,
                request_id,
                host,
                request.x,
                request.delay_ms,
                request_bytes,
            ),
            "future.blocking" => make_hand(
                HandMode::Blocking,
                request_id,
                host,
                request.x,
                request.delay_ms,
                request_bytes,
            ),
            "future.stale_wake" => make_hand(
                HandMode::StaleWake,
                request_id,
                host,
                request.x,
                request.delay_ms,
                request_bytes,
            ),
            "future.cancel" => make_hand(
                HandMode::CancelAware,
                request_id,
                host,
                request.x,
                request.delay_ms,
                request_bytes,
            ),
            "future.ignore_cancel" => make_hand(
                HandMode::IgnoreCancel,
                request_id,
                host,
                request.x,
                request.delay_ms,
                request_bytes,
            ),
            _ => return MetisStatus::NotFound,
        };
        // SAFETY: the host owns the out parameter.
        unsafe { *out = handle };
        MetisStatus::Ok
    }));
    caught.unwrap_or(MetisStatus::Internal)
}

/// Appended plugin table entry: the host's answer to a `service_request`.
pub(crate) unsafe extern "C" fn service_complete(
    _instance: *mut c_void,
    request_id: u64,
    status: MetisStatus,
    payload: MetisBuffer,
) -> MetisStatus {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        if payload.owner == METIS_BUFFER_OWNER_PLUGIN && !payload.ptr.is_null() {
            // A plugin-allocated service payload is released by the plugin.
            let _ = release_buffer(payload);
        }
        let entry = pending().lock().unwrap().get(&request_id).cloned();
        match entry {
            Some(PendingFuture::Ffi(shared)) => {
                record_service_roundtrip(*shared.request_at.lock().unwrap());
                *shared.done.lock().unwrap() = Some((status, Vec::new()));
                if let Some(waker) = shared.waker.lock().unwrap().take() {
                    waker.wake();
                }
                MetisStatus::Ok
            }
            Some(PendingFuture::Hand(state)) => {
                let waker = {
                    let mut state = state.lock().unwrap();
                    record_service_roundtrip(state.request_at);
                    state.result = Some(status);
                    state.waker.take()
                };
                if let Some(waker) = waker {
                    // SAFETY: an owned clone; `wake` consumes and releases it.
                    unsafe { release_waker(waker) };
                }
                MetisStatus::Ok
            }
            None => MetisStatus::NotFound,
        }
    }));
    caught.unwrap_or(MetisStatus::Internal)
}
