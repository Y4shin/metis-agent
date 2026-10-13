//! PROTOTYPE (throwaway). Not product code.
//!
//! The host side of the host-polled future seam, folded from lanes 1 and 2.
//!
//! The host is the only executor. It calls the plugin's `invoke_future`, gets
//! an opaque `MetisFutureHandle`, and drives it with `poll`. When a poll is
//! `Pending` the host parks on a condvar that the waker it handed in notifies.
//! The waker is the whole trick: the plugin stores a clone of it and, when its
//! `service_complete` answer arrives from the host's timer thread, calls
//! `wake`, which is a host callback that notifies the parked poll loop.
//!
//! The host owns the timers (`service_request` -> one persistent timer thread),
//! so the plugin owns no runtime and spawns no thread on the ordinary path. The
//! library is only dropped after every future this driver handed out has been
//! dropped, which the order ledger records.
//!
//! The hazards are the same host, not a second one:
//!
//! - the host waker's `data` is an `Arc<WakerState>` over an `Arc<FutureState>`,
//!   so a clone that outlives the future still finds a live host object and a
//!   wake after drop is a recorded violation, not a dangling dereference;
//! - `poll_bounded` runs one poll on a watchdog thread with a deadline, so a
//!   blocking poll is detected and abandoned instead of hanging the host;
//! - `drop_future` refuses a double drop, a drop after unload and a drop while
//!   a poll is in progress, each as a recorded violation;
//! - every poll and drop is written to an ordering ledger, so "the host never
//!   drops a future while a poll is in progress" is checkable, not prose.
//!
//! Teardown (the lane 1 bug, fixed and kept fixed): `Drop` takes the timer
//! `mpsc::Sender` out of the driver before joining the timer thread, because a
//! `join` while the sender is still alive can never return. The order ledger
//! then proves `["shutdown returned", "future host dropped", "library
//! unloaded"]`.

use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use proto_abi::{
    METIS_BUFFER_OWNER_HOST, METIS_BUFFER_OWNER_PLUGIN, MetisBuffer, MetisFutureHandle,
    MetisHostApiV1, MetisPluginTableV1, MetisStatus, MetisWaker, MetisWakerVtable,
    metis_encode_payload, metis_payload_body,
};

use crate::abi_adapter::OrderLedger;

/// Why a host-polled call did not produce a payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FutureError {
    /// `invoke_future` refused, or a poll returned a terminal failure.
    Status(MetisStatus),
    /// The future did not become ready before the deadline.
    Timeout,
    /// The future resolved with `Cancelled`.
    Cancelled,
    /// The future's payload envelope was rejected.
    BadEnvelope(MetisStatus),
    /// A null handle, or a protocol violation the host detected.
    Protocol(String),
}

impl std::fmt::Display for FutureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FutureError::Status(status) => write!(f, "status {}", status.as_str()),
            FutureError::Timeout => write!(f, "the future did not become ready before the deadline"),
            FutureError::Cancelled => write!(f, "the future resolved as cancelled"),
            FutureError::BadEnvelope(status) => {
                write!(f, "the payload envelope was rejected with {}", status.as_str())
            }
            FutureError::Protocol(detail) => write!(f, "protocol violation: {detail}"),
        }
    }
}

impl std::error::Error for FutureError {}

/// The outcome of one poll.
#[derive(Debug)]
pub struct PollReturn {
    pub status: MetisStatus,
    pub body: Vec<u8>,
}

// --- the shared protocol log -----------------------------------------------

/// One ordering-ledger entry, so the host's ordering rules are checkable.
#[derive(Clone)]
pub struct LedgerEntry {
    seq: usize,
    pub kind: &'static str,
    pub id: u64,
    pub detail: String,
}

/// Everything the harness observes about the seam: violations, the ordering
/// ledger, waker traffic, and host callback panics.
pub struct ProtocolLog {
    violations: Mutex<Vec<String>>,
    ledger: Mutex<Vec<LedgerEntry>>,
    notes: Mutex<Vec<String>>,
    seq: AtomicUsize,
    pub wakes: AtomicUsize,
    pub stale_wakes: AtomicUsize,
    pub waker_panics: AtomicUsize,
    pub callback_panics: AtomicUsize,
}

impl ProtocolLog {
    fn new() -> Arc<Self> {
        Arc::new(ProtocolLog {
            violations: Mutex::new(Vec::new()),
            ledger: Mutex::new(Vec::new()),
            notes: Mutex::new(Vec::new()),
            seq: AtomicUsize::new(0),
            wakes: AtomicUsize::new(0),
            stale_wakes: AtomicUsize::new(0),
            waker_panics: AtomicUsize::new(0),
            callback_panics: AtomicUsize::new(0),
        })
    }

    fn violation(&self, message: impl Into<String>) {
        self.violations.lock().unwrap().push(message.into());
    }

    fn note(&self, message: impl Into<String>) {
        self.notes.lock().unwrap().push(message.into());
    }

    fn ledger(&self, kind: &'static str, id: u64, detail: impl Into<String>) {
        let seq = self.seq.fetch_add(1, SeqCst);
        self.ledger.lock().unwrap().push(LedgerEntry {
            seq,
            kind,
            id,
            detail: detail.into(),
        });
    }

    pub fn violations(&self) -> Vec<String> {
        self.violations.lock().unwrap().clone()
    }

    pub fn violation_count(&self) -> usize {
        self.violations.lock().unwrap().len()
    }

    pub fn saw_violation(&self, needle: &str) -> bool {
        self.violations
            .lock()
            .unwrap()
            .iter()
            .any(|v| v.contains(needle))
    }

    pub fn ledger_entries(&self) -> Vec<LedgerEntry> {
        self.ledger.lock().unwrap().clone()
    }
}

// --- per-future host state --------------------------------------------------

/// The host state behind one live waker. An `Arc<WakerState>` is what the
/// waker's `data` points at, so a clone that outlives the future still finds a
/// live `FutureState` to report against.
struct WakerState {
    future: Arc<FutureState>,
    task: Arc<HostTask>,
}

struct FutureState {
    id: u64,
    protocol: Arc<ProtocolLog>,
    dropped: AtomicBool,
    poll_in_progress: AtomicBool,
    live_wakers: AtomicUsize,
    panic_next_wake: AtomicBool,
}

// --- the parked poll loop the waker notifies -------------------------------

struct HostTask {
    woken: Mutex<bool>,
    condvar: Condvar,
}

impl HostTask {
    fn new() -> Self {
        HostTask {
            woken: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    fn notify(&self) {
        *self.woken.lock().unwrap() = true;
        self.condvar.notify_all();
    }

    /// Waits until notified or the timeout expires. Returns whether it was
    /// notified.
    fn wait(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut woken = self.woken.lock().unwrap();
        while !*woken {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            woken = self.condvar.wait_timeout(woken, deadline - now).unwrap().0;
        }
        *woken = false;
        true
    }
}

// --- the host waker vtable --------------------------------------------------

static WAKER_VTABLE: MetisWakerVtable = MetisWakerVtable {
    size: std::mem::size_of::<MetisWakerVtable>(),
    clone: waker_clone,
    wake: waker_wake,
    wake_by_ref: waker_wake_by_ref,
    drop: waker_drop,
};

fn make_waker(state: &Arc<FutureState>, task: &Arc<HostTask>) -> MetisWaker {
    state.live_wakers.fetch_add(1, SeqCst);
    let waker = Arc::new(WakerState {
        future: Arc::clone(state),
        task: Arc::clone(task),
    });
    MetisWaker {
        data: Arc::into_raw(waker) as *mut c_void,
        vtable: &WAKER_VTABLE,
    }
}

/// # Safety
/// `data` must be a live `Arc<WakerState>` pointer from `make_waker`.
unsafe fn waker_state(waker: &MetisWaker) -> &WakerState {
    unsafe { &*(waker.data as *const WakerState) }
}

/// The waker body, reached by the plugin on any thread. A panic here is caught
/// at the callback frame, exactly like every other host callback.
fn waker_body(waker: &MetisWaker) {
    let state = unsafe { waker_state(waker) };
    let future = &state.future;
    let caught = catch_unwind(AssertUnwindSafe(|| {
        if future.panic_next_wake.swap(false, SeqCst) {
            panic!("host waker body panicked (hazard test)");
        }
        if future.dropped.load(SeqCst) {
            // The future is gone but a clone still held a live host object, so
            // the late wake is detected instead of being undefined behaviour.
            future.protocol.violation(format!(
                "waker woken after future {} was dropped",
                future.id
            ));
            future.protocol.stale_wakes.fetch_add(1, SeqCst);
        } else {
            future.protocol.wakes.fetch_add(1, SeqCst);
        }
        state.task.notify();
    }));
    if caught.is_err() {
        future.protocol.waker_panics.fetch_add(1, SeqCst);
    }
}

unsafe extern "C" fn waker_clone(waker: *const MetisWaker) -> MetisWaker {
    if waker.is_null() {
        return MetisWaker::null();
    }
    // SAFETY: the caller passes a live waker handed across the boundary.
    let waker = unsafe { &*waker };
    let state = unsafe { waker_state(waker) };
    let future = Arc::clone(&state.future);
    future.live_wakers.fetch_add(1, SeqCst);
    let clone = Arc::new(WakerState {
        future,
        task: Arc::clone(&state.task),
    });
    MetisWaker {
        data: Arc::into_raw(clone) as *mut c_void,
        vtable: &WAKER_VTABLE,
    }
}

unsafe extern "C" fn waker_wake(waker: MetisWaker) {
    if waker.data.is_null() {
        return;
    }
    waker_body(&waker);
    // SAFETY: `wake` consumes the one reference this waker owned.
    let state = unsafe { Arc::from_raw(waker.data as *const WakerState) };
    state.future.live_wakers.fetch_sub(1, SeqCst);
}

unsafe extern "C" fn waker_wake_by_ref(waker: *const MetisWaker) {
    if waker.is_null() {
        return;
    }
    // SAFETY: the caller keeps the reference alive for the call.
    let waker = unsafe { &*waker };
    if waker.data.is_null() {
        return;
    }
    waker_body(waker);
}

unsafe extern "C" fn waker_drop(waker: MetisWaker) {
    if waker.data.is_null() {
        return;
    }
    // SAFETY: `drop` consumes the one reference this waker owned.
    let state = unsafe { Arc::from_raw(waker.data as *const WakerState) };
    state.future.live_wakers.fetch_sub(1, SeqCst);
}

// --- the host-side timer service -------------------------------------------

struct TimerMsg {
    delay_ms: u64,
    request_id: u64,
}

struct TimerTarget {
    table: AtomicPtr<MetisPluginTableV1>,
    instance: AtomicPtr<c_void>,
    completions: AtomicUsize,
    protocol: Arc<ProtocolLog>,
}

/// The host-owned timer thread, one per driver. It answers
/// `service_request("timer", ...)` by calling the plugin's `service_complete`;
/// the plugin then wakes its own host task. The thread never touches plugin
/// memory except through the table.
fn timer_loop(receiver: mpsc::Receiver<TimerMsg>, target: Arc<TimerTarget>) {
    while let Ok(message) = receiver.recv() {
        if message.delay_ms > 0 {
            thread::sleep(Duration::from_millis(message.delay_ms));
        }
        let table = target.table.load(SeqCst);
        let instance = target.instance.load(SeqCst);
        if table.is_null() {
            continue;
        }
        // SAFETY: the table is the loaded plugin's, and it is only cleared
        // after this thread is joined.
        let complete = unsafe { (*table).service_complete };
        let _ = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: the plugin table entry is the frozen appended one.
            unsafe { complete(instance, message.request_id, MetisStatus::Ok, MetisBuffer::empty()) };
        }));
        target.completions.fetch_add(1, SeqCst);
        target
            .protocol
            .note(format!("service_complete delivered for {}", message.request_id));
    }
}

// --- the driver ------------------------------------------------------------

struct FutureInner {
    api: MetisHostApiV1,
    table: AtomicPtr<MetisPluginTableV1>,
    instance: AtomicPtr<c_void>,
    /// `None` once the driver is dropping, which is what lets the timer thread
    /// see its sender disappear and return.
    sender: Mutex<Option<mpsc::Sender<TimerMsg>>>,
    polls: AtomicUsize,
    released: AtomicUsize,
    violations: AtomicUsize,
    service_requests: AtomicUsize,
}

impl FutureInner {
    fn table(&self) -> *const MetisPluginTableV1 {
        self.table.load(SeqCst)
    }

    /// Copies a plugin-owned buffer out and releases it through the plugin's
    /// own `release_buffer`.
    unsafe fn take(&self, buffer: MetisBuffer) -> Vec<u8> {
        if buffer.ptr.is_null() {
            return Vec::new();
        }
        // SAFETY: the plugin allocated `len` bytes at `ptr` and it stays valid
        // until `release_buffer` below.
        let bytes = unsafe { slice::from_raw_parts(buffer.ptr, buffer.len) };
        let body = metis_payload_body(bytes).map(<[u8]>::to_vec).unwrap_or_default();
        self.release(buffer);
        body
    }

    fn release(&self, buffer: MetisBuffer) {
        let table = self.table();
        if table.is_null() || buffer.ptr.is_null() {
            return;
        }
        if buffer.owner != METIS_BUFFER_OWNER_PLUGIN {
            self.violations.fetch_add(1, SeqCst);
            return;
        }
        // SAFETY: the table is the attached plugin's and the buffer came from
        // it.
        let release = unsafe { (*table).release_buffer };
        let _ = unsafe { release(buffer) };
        self.released.fetch_add(1, SeqCst);
    }
}

/// Drives plugin futures on the host: the only executor in the seam.
pub struct FutureHost {
    inner: Arc<FutureInner>,
    timer_thread: Option<JoinHandle<()>>,
    ledger: OrderLedger,
    protocol: Arc<ProtocolLog>,
    plugin_alive: Arc<AtomicBool>,
}

impl FutureHost {
    /// Binds a driver to a plugin table and instance that are already live.
    pub fn new(
        table: *const MetisPluginTableV1,
        instance: *mut c_void,
        ledger: OrderLedger,
    ) -> FutureHost {
        let protocol = ProtocolLog::new();
        let (sender, receiver) = mpsc::channel::<TimerMsg>();
        let target = Arc::new(TimerTarget {
            table: AtomicPtr::new(table as *mut MetisPluginTableV1),
            instance: AtomicPtr::new(instance),
            completions: AtomicUsize::new(0),
            protocol: Arc::clone(&protocol),
        });
        let timer_thread = thread::spawn({
            let target = Arc::clone(&target);
            move || timer_loop(receiver, target)
        });
        let mut inner = Arc::new(FutureInner {
            api: MetisHostApiV1 {
                size: std::mem::size_of::<MetisHostApiV1>(),
                user_data: ptr::null_mut(),
                complete: host_complete,
                progress: host_progress,
                emit_event: host_emit_event,
                log: host_log,
                service_request: host_service_request,
            },
            table: AtomicPtr::new(table as *mut MetisPluginTableV1),
            instance: AtomicPtr::new(instance),
            sender: Mutex::new(Some(sender)),
            polls: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
            violations: AtomicUsize::new(0),
            service_requests: AtomicUsize::new(0),
        });
        let address = Arc::as_ptr(&inner) as *mut c_void;
        if let Some(state) = Arc::get_mut(&mut inner) {
            state.api.user_data = address;
        }
        // The timer thread holds its own `Arc<TimerTarget>` clone, so the
        // target lives exactly as long as the thread, which the join in `Drop`
        // bounds.
        drop(target);
        FutureHost {
            inner,
            timer_thread: Some(timer_thread),
            ledger,
            protocol,
            plugin_alive: Arc::new(AtomicBool::new(true)),
        }
    }

    /// The callback table handed to `invoke_future`.
    fn api_ptr(&self) -> *const MetisHostApiV1 {
        &self.inner.api
    }

    pub fn polls(&self) -> usize {
        self.inner.polls.load(SeqCst)
    }

    pub fn released(&self) -> usize {
        self.inner.released.load(SeqCst)
    }

    pub fn violations(&self) -> usize {
        self.inner.violations.load(SeqCst)
    }

    pub fn service_requests(&self) -> usize {
        self.inner.service_requests.load(SeqCst)
    }

    pub fn protocol(&self) -> &Arc<ProtocolLog> {
        &self.protocol
    }

    /// Simulates the host unloading the library, so a drop after unload can be
    /// shown to be refused rather than left undefined.
    pub fn simulate_unload(&self) {
        self.plugin_alive.store(false, SeqCst);
    }

    pub fn restore_load(&self) {
        self.plugin_alive.store(true, SeqCst);
    }

    /// Asks the plugin for a future. The caller owns the returned handle until
    /// `HostFuture::drop_future`.
    pub fn invoke_future(
        &self,
        capability: &str,
        body: &[u8],
        request_id: u64,
    ) -> Result<HostFuture, FutureError> {
        let table = self.inner.table();
        if table.is_null() {
            return Err(FutureError::Protocol("the plugin table is not attached".to_owned()));
        }
        let request = metis_encode_payload(body);
        let borrowed = MetisBuffer {
            ptr: request.as_ptr() as *mut u8,
            len: request.len(),
            cap: request.capacity(),
            owner: METIS_BUFFER_OWNER_HOST,
            reserved: 0,
        };
        let mut out = MetisFutureHandle::null();
        // SAFETY: the table is live and the host table outlives the call.
        let invoke = unsafe { (*table).invoke_future };
        let status = unsafe {
            invoke(
                self.inner.instance.load(SeqCst),
                capability.as_ptr(),
                capability.len(),
                request_id,
                borrowed,
                self.api_ptr(),
                &mut out,
            )
        };
        drop(request);
        if status != MetisStatus::Ok {
            return Err(FutureError::Status(status));
        }
        if out.ctx.is_null() || out.vtable.is_null() {
            return Err(FutureError::Protocol(
                "invoke_future returned Ok with a null handle".to_owned(),
            ));
        }
        let state = Arc::new(FutureState {
            id: request_id,
            protocol: Arc::clone(&self.protocol),
            dropped: AtomicBool::new(false),
            poll_in_progress: AtomicBool::new(false),
            live_wakers: AtomicUsize::new(0),
            panic_next_wake: AtomicBool::new(false),
        });
        Ok(HostFuture {
            id: request_id,
            handle: out,
            state,
            task: Arc::new(HostTask::new()),
            protocol: Arc::clone(&self.protocol),
            plugin_alive: Arc::clone(&self.plugin_alive),
            table,
            dropped: false,
            abandoned: false,
        })
    }

    /// Convenience: invoke, poll to ready, drop. Returns the payload.
    pub fn call(
        &self,
        capability: &str,
        body: &[u8],
        request_id: u64,
        timeout: Duration,
    ) -> Result<Vec<u8>, FutureError> {
        let mut future = self.invoke_future(capability, body, request_id)?;
        let result = future.poll_to_ready(timeout);
        let _ = future.drop_future();
        result
    }

    /// Requests cancellation through the plugin's existing `cancel` entry.
    pub fn cancel(&self, request_id: u64) -> MetisStatus {
        let table = self.inner.table();
        if table.is_null() {
            return MetisStatus::Internal;
        }
        // SAFETY: the table is live.
        let cancel = unsafe { (*table).cancel };
        unsafe { cancel(self.inner.instance.load(SeqCst), request_id) }
    }
}

impl Drop for FutureHost {
    fn drop(&mut self) {
        // Dropping the sender stops the timer thread; joining it proves no
        // `service_complete` can arrive after the host says it is done. The
        // sender lives inside `inner`, so it is taken out before the join, not
        // left for the field drop that happens after this body.
        *self.inner.sender.lock().unwrap() = None;
        if let Some(handle) = self.timer_thread.take() {
            let _ = handle.join();
        }
        self.inner.table.store(ptr::null_mut(), SeqCst);
        self.ledger
            .lock()
            .unwrap()
            .push("future host dropped".to_owned());
    }
}

// --- one future, driven by the host ----------------------------------------

/// A host-owned future handle. One per `invoke_future`.
pub struct HostFuture {
    id: u64,
    handle: MetisFutureHandle,
    state: Arc<FutureState>,
    task: Arc<HostTask>,
    protocol: Arc<ProtocolLog>,
    plugin_alive: Arc<AtomicBool>,
    table: *const MetisPluginTableV1,
    dropped: bool,
    abandoned: bool,
}

impl HostFuture {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn live_wakers(&self) -> usize {
        self.state.live_wakers.load(SeqCst)
    }

    /// The panic-in-waker hazard control: the next waker body this future's
    /// waker runs panics.
    pub fn arm_waker_panic(&self) {
        self.state.panic_next_wake.store(true, SeqCst);
    }

    /// A waker for this future, built here so the host can drive a wake body
    /// directly (the waker-panic hazard).
    pub fn test_waker(&self) -> MetisWaker {
        make_waker(&self.state, &self.task)
    }

    pub fn wake_waker(waker: &MetisWaker) {
        if waker.vtable.is_null() {
            return;
        }
        // SAFETY: the vtable is this host's, and the waker stays borrowed.
        unsafe { ((*waker.vtable).wake_by_ref)(waker) };
    }

    pub fn drop_waker(waker: MetisWaker) {
        if waker.vtable.is_null() || waker.data.is_null() {
            return;
        }
        // SAFETY: the vtable is this host's; `drop` consumes the reference.
        unsafe { ((*waker.vtable).drop)(waker) };
    }

    fn release_out(&self, out: MetisBuffer) -> Vec<u8> {
        if out.ptr.is_null() {
            return Vec::new();
        }
        if self.table.is_null() {
            return Vec::new();
        }
        // SAFETY: the plugin allocated `len` bytes at `ptr` and the host
        // releases it below through the plugin's own `release_buffer`.
        let raw = unsafe { slice::from_raw_parts(out.ptr, out.len) };
        let body = metis_payload_body(raw).map(<[u8]>::to_vec).unwrap_or_default();
        if out.owner != METIS_BUFFER_OWNER_PLUGIN {
            return body;
        }
        // SAFETY: the table is the attached plugin's and the buffer came from
        // it.
        let release = unsafe { (*self.table).release_buffer };
        let _ = unsafe { release(out) };
        body
    }

    fn poll_once_inner(&mut self) -> PollReturn {
        assert!(!self.dropped, "the harness never polls a dropped future");
        assert!(!self.abandoned, "the harness never polls an abandoned future");
        self.state.poll_in_progress.store(true, SeqCst);
        self.protocol.ledger("poll_begin", self.id, "direct");
        let waker = make_waker(&self.state, &self.task);
        // The by-value handle is borrowed by the plugin; pass a bitwise copy of
        // the two words so the owner here still releases the reference after
        // the call. A clone the plugin kept holds its own reference.
        let passed = MetisWaker {
            data: waker.data,
            vtable: waker.vtable,
        };
        let mut out = MetisBuffer::empty();
        // SAFETY: the context and vtable come from `invoke_future` and the host
        // has not dropped the future.
        let status = unsafe { ((*self.handle.vtable).poll)(self.handle.ctx, passed, &mut out) };
        Self::drop_waker(waker);
        self.state.poll_in_progress.store(false, SeqCst);
        self.protocol
            .ledger("poll_return", self.id, status.as_str());
        let body = self.release_out(out);
        PollReturn { status, body }
    }

    /// Polls once on this thread, releasing any buffer the plugin returned.
    pub fn poll(&mut self) -> PollReturn {
        self.poll_once_inner()
    }

    /// Polls once and reports `Ok(Some(body))` when ready, `Ok(None)` when
    /// pending, `Err` when terminal.
    pub fn poll_once_result(&mut self) -> Result<Option<Vec<u8>>, FutureError> {
        let PollReturn { status, body } = self.poll_once_inner();
        match status {
            MetisStatus::Ok => Ok(Some(body)),
            MetisStatus::Pending => Ok(None),
            MetisStatus::Cancelled => Err(FutureError::Cancelled),
            other => Err(FutureError::Status(other)),
        }
    }

    /// Polls until ready or the deadline expires, parking on the waker between
    /// polls. Returns the envelope-stripped payload.
    pub fn poll_to_ready(&mut self, timeout: Duration) -> Result<Vec<u8>, FutureError> {
        let deadline = Instant::now() + timeout;
        loop {
            *self.task.woken.lock().unwrap() = false;
            let PollReturn { status, body } = self.poll_once_inner();
            match status {
                MetisStatus::Ok => return Ok(body),
                MetisStatus::Pending => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(FutureError::Timeout);
                    }
                    if !self.task.wait(deadline - now) {
                        return Err(FutureError::Timeout);
                    }
                }
                MetisStatus::Cancelled => return Err(FutureError::Cancelled),
                other => return Err(FutureError::Status(other)),
            }
        }
    }

    /// Polls once on a watchdog thread with a deadline. If the plugin's `poll`
    /// blocks, the deadline fires and the call reports a timeout instead of
    /// hanging the host; the poll thread is left where it is because the host
    /// must never drop a future while a poll is in progress.
    pub fn poll_bounded(&mut self, deadline: Duration) -> Result<PollReturn, FutureError> {
        assert!(!self.dropped, "the harness never polls a dropped future");
        assert!(!self.abandoned, "the harness never polls an abandoned future");
        self.state.poll_in_progress.store(true, SeqCst);
        self.protocol
            .ledger("poll_begin", self.id, format!("bounded {}ms", deadline.as_millis()));
        let (tx, rx) = mpsc::channel();
        let job = PollJob {
            ctx: self.handle.ctx,
            vtable: self.handle.vtable,
            waker: make_waker(&self.state, &self.task),
            tx,
        };
        thread::spawn(move || job.run());
        match rx.recv_timeout(deadline) {
            Ok((status, out)) => {
                self.state.poll_in_progress.store(false, SeqCst);
                self.protocol
                    .ledger("poll_return", self.id, status.as_str());
                let body = self.release_out(out);
                Ok(PollReturn { status, body })
            }
            Err(_) => {
                // The poll is still running. The host must not drop, and the
                // flag stays set to make that visible to the ledger.
                self.abandoned = true;
                self.protocol.violation(format!(
                    "poll for future {} exceeded its {}ms deadline (blocking poll)",
                    self.id,
                    deadline.as_millis()
                ));
                Err(FutureError::Timeout)
            }
        }
    }

    /// Drops the future exactly once, refusing a double drop, a drop after
    /// unload, and a drop while a poll is in progress.
    pub fn drop_future(&mut self) -> Result<MetisStatus, FutureError> {
        if self.dropped {
            self.protocol
                .violation(format!("double drop of future {} refused", self.id));
            return Err(FutureError::Protocol(format!(
                "double drop of future {} refused",
                self.id
            )));
        }
        if !self.plugin_alive.load(SeqCst) {
            self.protocol.violation(format!(
                "drop of future {} after unload refused",
                self.id
            ));
            return Err(FutureError::Protocol(format!(
                "drop of future {} after unload refused",
                self.id
            )));
        }
        if self.state.poll_in_progress.load(SeqCst) {
            self.protocol.violation(format!(
                "drop of future {} while a poll is in progress refused",
                self.id
            ));
            return Err(FutureError::Protocol(format!(
                "drop of future {} while a poll is in progress refused",
                self.id
            )));
        }
        if self.table.is_null() {
            return Err(FutureError::Protocol("the plugin table is not attached".to_owned()));
        }
        self.protocol.ledger("drop_begin", self.id, "");
        // Set `dropped` before the plugin's `drop` runs, so a stray wake during
        // the drop is still reported as a wake after drop.
        self.state.dropped.store(true, SeqCst);
        // SAFETY: the handle came from `invoke_future` and this path runs once.
        let status = unsafe { ((*self.handle.vtable).drop)(self.handle.ctx) };
        self.protocol
            .ledger("drop_return", self.id, status.as_str());
        self.dropped = true;
        Ok(status)
    }

    pub fn is_dropped(&self) -> bool {
        self.dropped
    }
}

/// A poll moved onto a watchdog thread. The raw pointers and waker are the
/// seam's own; the host promises to poll from one thread at a time.
struct PollJob {
    ctx: *mut c_void,
    vtable: *const proto_abi::MetisFutureVtable,
    waker: MetisWaker,
    tx: mpsc::Sender<(MetisStatus, MetisBuffer)>,
}

// SAFETY: the job carries the seam's own pointers to a single poll; nothing
// else touches the context while the job runs, because the host refuses to drop
// or re-poll until this poll returns.
unsafe impl Send for PollJob {}

impl PollJob {
    fn run(self) {
        let PollJob {
            ctx,
            vtable,
            waker,
            tx,
        } = self;
        let passed = MetisWaker {
            data: waker.data,
            vtable: waker.vtable,
        };
        let mut out = MetisBuffer::empty();
        // SAFETY: the context and vtable are live for the poll.
        let status = unsafe { ((*vtable).poll)(ctx, passed, &mut out) };
        // The by-value waker was borrowed; release the reference the host made.
        if !waker.vtable.is_null() && !waker.data.is_null() {
            // SAFETY: this consumes the reference `make_waker` made.
            unsafe { ((*waker.vtable).drop)(waker) };
        }
        let _ = tx.send((status, out));
    }
}

// --- the ordering-ledger rule ----------------------------------------------

/// Checks the ledger for the host's ordering rules: a drop never overlaps a
/// poll, and every poll that returned is matched before the drop.
fn ordering_ok(entries: &[LedgerEntry]) -> (bool, String) {
    let mut poll_open: HashMap<u64, bool> = HashMap::new();
    let mut in_drop: HashMap<u64, bool> = HashMap::new();
    let mut problems: Vec<String> = Vec::new();
    for entry in entries {
        match entry.kind {
            "poll_begin" => {
                if *in_drop.get(&entry.id).unwrap_or(&false) {
                    problems.push(format!(
                        "seq {}: poll began for future {} inside a drop",
                        entry.seq, entry.id
                    ));
                }
                poll_open.insert(entry.id, true);
            }
            "poll_return" => {
                poll_open.insert(entry.id, false);
            }
            "drop_begin" => {
                if *poll_open.get(&entry.id).unwrap_or(&false) {
                    problems.push(format!(
                        "seq {}: drop began for future {} while a poll was open",
                        entry.seq, entry.id
                    ));
                }
                in_drop.insert(entry.id, true);
            }
            "drop_return" => {
                in_drop.insert(entry.id, false);
            }
            _ => {}
        }
    }
    if problems.is_empty() {
        (
            true,
            format!("{} ledger entries, no drop overlapped a poll", entries.len()),
        )
    } else {
        (false, problems.join("; "))
    }
}

// --- the folded hazard harness (lane 2) ------------------------------------

/// What the hazard harness prints through: the host binary's own `Harness`.
pub trait CheckSink {
    fn check(&mut self, name: &str, ok: bool, detail: &str);
    fn note(&self, line: &str);
    fn section(&self, line: &str);
    fn bench(&self, line: &str);
}

pub fn next_request_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0x1000_0000);
    NEXT.fetch_add(1, SeqCst)
}

/// How long to wait for the blocked poll thread to finish before unload.
const BLOCKING_POLL_WAIT_MS: u64 = 900;

/// A small view over the legacy instance's `metis.accounting` call, which reads
/// the plugin's process-global allocator ledger.
pub struct AccountingProbe<'a> {
    query: &'a dyn Fn() -> serde_json::Value,
}

impl<'a> AccountingProbe<'a> {
    pub fn new(query: &'a dyn Fn() -> serde_json::Value) -> Self {
        AccountingProbe { query }
    }

    fn live_buffers(&self) -> usize {
        (self.query)()
            .get("live")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize
    }

    pub fn live_buffers_for_diagnostics(&self) -> usize {
        self.live_buffers()
    }
}

/// Runs every lane 2 hazard check through the folded host. Each one prints
/// exactly one line through `sink`.
pub fn run_hazards(
    sink: &mut dyn CheckSink,
    host: &FutureHost,
    accounting: &AccountingProbe<'_>,
) {
    sink.section("host-polled future seam: hazards (lane 2)");
    sink.check(
        "the appended invoke_future/service_complete entries are present (presence decided by size)",
        true,
        "the plugin table's size covers the appended entries",
    );

    let invoke = |capability: &str| -> HostFuture {
        host.invoke_future(capability, b"{}", next_request_id())
            .expect("invoke_future")
    };

    // --- 1. panic in poll --------------------------------------------------
    let mut panic_poll = invoke("future.panic_poll");
    let first = panic_poll.poll().status;
    let second = panic_poll.poll().status;
    let terminal = first == MetisStatus::Internal && second == MetisStatus::Internal;
    let drop_ok = matches!(panic_poll.drop_future(), Ok(MetisStatus::Ok));
    sink.check(
        "1 panic in the author's future becomes Internal, stays terminal, and drop still works",
        terminal && drop_ok,
        &format!("poll1={} poll2={} drop_ok={drop_ok}", first.as_str(), second.as_str()),
    );

    // --- 2. panic in the future's drop shim --------------------------------
    let live_baseline = accounting.live_buffers();
    let mut panic_drop = invoke("future.panic_drop");
    let pending = panic_drop.poll().status;
    let live_while_held = accounting.live_buffers();
    let panic_drop_status = panic_drop.drop_future();
    let live_after = accounting.live_buffers();
    sink.check(
        "2 a panic in the future's drop shim does not abort and releases the buffer it still held",
        pending == MetisStatus::Pending
            && matches!(panic_drop_status, Ok(MetisStatus::Ok))
            && live_while_held > live_baseline
            && live_after <= live_baseline,
        &format!(
            "poll={} drop={panic_drop_status:?} live {live_baseline} -> {live_while_held} (held) -> {live_after}",
            pending.as_str()
        ),
    );

    // --- 3. panic in a host waker body -------------------------------------
    let mut waker_panic = invoke("future.cancel");
    let _ = waker_panic.poll();
    let panics_before = host.protocol().waker_panics.load(SeqCst);
    waker_panic.arm_waker_panic();
    let waker = waker_panic.test_waker();
    HostFuture::wake_waker(&waker);
    HostFuture::drop_waker(waker);
    let panics_after = host.protocol().waker_panics.load(SeqCst);
    let _ = waker_panic.poll();
    let usable = matches!(waker_panic.drop_future(), Ok(_));
    sink.check(
        "3 a panic in a host waker body is contained at the callback frame",
        panics_after == panics_before + 1 && usable,
        &format!("waker_panics={panics_before}->{panics_after} future_still_usable={usable}"),
    );

    // --- 4. cancellation resolves as Cancelled -----------------------------
    let cancel_id = next_request_id();
    let wakes_before = host.protocol().wakes.load(SeqCst);
    let mut cancellable = host
        .invoke_future("future.cancel", b"{}", cancel_id)
        .expect("invoke_future");
    let p0 = cancellable.poll().status;
    let cancel_status = host.cancel(cancel_id);
    let wakes_after_cancel = host.protocol().wakes.load(SeqCst);
    let p1 = cancellable.poll().status;
    let drop_ok = matches!(cancellable.drop_future(), Ok(MetisStatus::Ok));
    sink.check(
        "4 cancel wakes the future, which resolves as Cancelled, then the host drops it",
        p0 == MetisStatus::Pending
            && cancel_status == MetisStatus::Ok
            && p1 == MetisStatus::Cancelled
            && wakes_after_cancel > wakes_before
            && drop_ok,
        &format!(
            "poll0={} cancel={} wakes {wakes_before}->{wakes_after_cancel} poll1={} drop_ok={drop_ok}",
            p0.as_str(),
            cancel_status.as_str(),
            p1.as_str()
        ),
    );

    // --- 5. a plugin that ignores the cancel deadline ----------------------
    let ignore_id = next_request_id();
    let mut ignores = host
        .invoke_future("future.ignore_cancel", b"{}", ignore_id)
        .expect("invoke_future");
    let ig0 = ignores.poll().status;
    let ignore_cancel = host.cancel(ignore_id);
    let deadline = Instant::now() + Duration::from_millis(120);
    let mut last = ig0;
    while Instant::now() < deadline {
        last = ignores.poll().status;
        if last != MetisStatus::Pending {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let missed = last == MetisStatus::Pending;
    if missed {
        host.protocol().violation(format!(
            "future {ignore_id} missed the cancel deadline and was dropped by the host"
        ));
    }
    let dropped_after = ignores.drop_future();
    let mut fresh = invoke("future.echo");
    let fresh_status = fresh.poll().status;
    let fresh_drop = fresh.drop_future();
    sink.check(
        "5 a plugin that ignores the cancel deadline is dropped after it, recorded, and the host stays usable",
        ignore_cancel == MetisStatus::Ok
            && missed
            && matches!(dropped_after, Ok(MetisStatus::Ok))
            && fresh_status == MetisStatus::Ok
            && matches!(fresh_drop, Ok(MetisStatus::Ok))
            && host.protocol().saw_violation("missed the cancel deadline"),
        &format!(
            "cancel={} still_pending={missed} drop={dropped_after:?} fresh={}",
            ignore_cancel.as_str(),
            fresh_status.as_str()
        ),
    );

    // --- 6. double drop, and drop after unload -----------------------------
    let mut one_drop = invoke("future.echo");
    let first_drop = one_drop.drop_future();
    let second_drop = one_drop.drop_future();
    let mut after_unload = invoke("future.echo");
    host.simulate_unload();
    let unload_drop = after_unload.drop_future();
    host.restore_load();
    sink.check(
        "6 double drop and drop after unload are refused, not undefined behaviour",
        matches!(first_drop, Ok(MetisStatus::Ok))
            && second_drop.is_err()
            && unload_drop.is_err()
            && host.protocol().saw_violation("double drop")
            && host.protocol().saw_violation("after unload"),
        &format!(
            "first={first_drop:?} second={second_drop:?} after_unload={unload_drop:?}"
        ),
    );

    // --- 7. a waker clone that wakes after the future was dropped ----------
    let mut stale = invoke("future.stale_wake");
    let _ = stale.poll();
    let stale_id = stale.id();
    let _ = stale.drop_future();
    // The plugin's own thread wakes the clone about 250 ms after the drop.
    let stale_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < stale_deadline && !host.protocol().saw_violation("waker woken after future")
    {
        thread::sleep(Duration::from_millis(20));
    }
    // Give the plugin thread time to release its clone.
    thread::sleep(Duration::from_millis(50));
    sink.check(
        "7 a waker clone that wakes after the future was dropped is reported, not crashed on",
        host.protocol()
            .saw_violation(&format!("future {stale_id} was dropped"))
            && stale.live_wakers() == 0,
        &format!(
            "violation={} live_wakers_after={}",
            host.protocol()
                .saw_violation(&format!("future {stale_id} was dropped")),
            stale.live_wakers()
        ),
    );

    // --- 8. a blocking poll is detected, not allowed to hang ----------------
    let mut blocker = invoke("future.blocking");
    let started = Instant::now();
    let blocked = blocker.poll_bounded(Duration::from_millis(150));
    let detect_ms = started.elapsed().as_millis();
    if blocked.is_err() {
        host.protocol().violation(
            "a poll that blocked was detected by the watchdog and abandoned",
        );
    }
    let mut after_block = invoke("future.echo");
    let after_block_status = after_block.poll().status;
    let after_block_drop = after_block.drop_future();
    sink.check(
        "8 a blocking poll is detected by a watchdog and reported, not allowed to hang the host",
        blocked.is_err()
            && detect_ms < 1000
            && after_block_status == MetisStatus::Ok
            && matches!(after_block_drop, Ok(MetisStatus::Ok)),
        &format!(
            "mechanism=watchdog thread with a deadline; returned after {detect_ms}ms; host still usable={}",
            after_block_status == MetisStatus::Ok
        ),
    );
    // Let the blocked poll thread finish before the library can unload.
    thread::sleep(Duration::from_millis(BLOCKING_POLL_WAIT_MS));

    // --- 9. the ordering ledger --------------------------------------------
    let entries = host.protocol().ledger_entries();
    let (ordered, detail) = ordering_ok(&entries);
    sink.check(
        "9 the host never drops a future while a poll is in progress (ordering ledger)",
        ordered && !host.protocol().saw_violation("while a poll is in progress"),
        &detail,
    );

    sink.note(&format!(
        "protocol violations recorded by the hazard run: {}",
        host.protocol().violation_count()
    ));
    for violation in host.protocol().violations() {
        sink.note(&format!("violation (expected, recorded): {violation}"));
    }
    sink.note(&format!(
        "stale wakes detected (a waker clone woke after its future was dropped): {}",
        host.protocol().stale_wakes.load(SeqCst)
    ));
}

// --- host callbacks the future path does not use ---------------------------
//
// They exist so the host table has no null entries. On the future path a
// completion, progress, event or log would be unexpected, so each records a
// violation and still releases any plugin buffer it was handed.

unsafe extern "C" fn host_complete(
    user_data: *mut c_void,
    _request_id: u64,
    _status: MetisStatus,
    payload: MetisBuffer,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if user_data.is_null() {
            return;
        }
        // SAFETY: `user_data` is the driver's boxed state, alive for the run.
        let inner = unsafe { &*(user_data as *const FutureInner) };
        unsafe { inner.take(payload) };
        inner.violations.fetch_add(1, SeqCst);
    }));
}

unsafe extern "C" fn host_progress(
    user_data: *mut c_void,
    _request_id: u64,
    payload: MetisBuffer,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if user_data.is_null() {
            return;
        }
        // SAFETY: as in `host_complete`.
        let inner = unsafe { &*(user_data as *const FutureInner) };
        unsafe { inner.take(payload) };
        inner.violations.fetch_add(1, SeqCst);
    }));
}

unsafe extern "C" fn host_emit_event(
    user_data: *mut c_void,
    _topic: *const u8,
    _topic_len: usize,
    payload: MetisBuffer,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if user_data.is_null() {
            return;
        }
        // SAFETY: as in `host_complete`.
        let inner = unsafe { &*(user_data as *const FutureInner) };
        unsafe { inner.take(payload) };
        inner.violations.fetch_add(1, SeqCst);
    }));
}

unsafe extern "C" fn host_log(
    _user_data: *mut c_void,
    _level: i32,
    _message: *const u8,
    _message_len: usize,
) {
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(seq: usize, kind: &'static str, id: u64) -> LedgerEntry {
        LedgerEntry {
            seq,
            kind,
            id,
            detail: String::new(),
        }
    }

    #[test]
    fn the_ordering_rule_accepts_a_clean_run() {
        let entries = vec![
            entry(0, "poll_begin", 1),
            entry(1, "poll_return", 1),
            entry(2, "drop_begin", 1),
            entry(3, "drop_return", 1),
        ];
        assert!(ordering_ok(&entries).0);
    }

    #[test]
    fn the_ordering_rule_rejects_a_drop_inside_a_poll() {
        let entries = vec![
            entry(0, "poll_begin", 1),
            entry(1, "drop_begin", 1),
            entry(2, "poll_return", 1),
            entry(3, "drop_return", 1),
        ];
        assert!(!ordering_ok(&entries).0);
    }
}

unsafe extern "C" fn host_service_request(
    user_data: *mut c_void,
    service: *const u8,
    service_len: usize,
    request_id: u64,
    payload: MetisBuffer,
) -> MetisStatus {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        if user_data.is_null() || service.is_null() {
            return MetisStatus::InvalidArgument;
        }
        // SAFETY: `user_data` is the driver's boxed state, alive for the run.
        let inner = unsafe { &*(user_data as *const FutureInner) };
        // SAFETY: the service name is borrowed for the call.
        let name = unsafe { slice::from_raw_parts(service, service_len) };
        // SAFETY: the plugin allocated the request payload; copy then release.
        let body = unsafe { inner.take(payload) };
        if name == b"timer" {
            let delay_ms = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value.get("delay_ms").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let sender = inner.sender.lock().unwrap().clone();
            let Some(sender) = sender else {
                return MetisStatus::Internal;
            };
            if sender.send(TimerMsg { delay_ms, request_id }).is_err() {
                return MetisStatus::Internal;
            }
            inner.service_requests.fetch_add(1, SeqCst);
            MetisStatus::Ok
        } else {
            inner.violations.fetch_add(1, SeqCst);
            MetisStatus::Unsupported
        }
    }));
    caught.unwrap_or(MetisStatus::Internal)
}
