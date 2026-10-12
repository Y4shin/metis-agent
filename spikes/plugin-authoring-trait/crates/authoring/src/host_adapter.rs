//! The host side of the hand-written translation.
//!
//! `MetisHostApiV1` on one side, an awaitable call for the consumer on the
//! other. The consumer registers a pending record before calling (D11), awaits
//! a future the `complete` callback resolves, parses the D4 header, and lets
//! the adapter release every plugin-allocated buffer (D6).
//!
//! Every callback is itself an `extern "C"` frame reached from plugin code, so
//! each one is wrapped in `catch_unwind` (D7 as amended by the mechanics spike).

use std::collections::HashMap;
use std::ffi::c_void;
use std::future::Future;
use std::mem;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::slice;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use spike_abi::*;

/// One completion as the consumer sees it: body already copied out of the
/// plugin buffer, which has been released by then.
#[derive(Debug, Clone)]
pub struct Completed {
    pub status: MetisStatus,
    pub body: Vec<u8>,
}

struct SlotState {
    value: Option<Completed>,
    wakers: Vec<Waker>,
}

/// The consumer's half of the pending path: where the `complete` callback parks
/// the result and how the awaiting task is woken.
pub struct Slot {
    state: Mutex<SlotState>,
    condvar: Condvar,
}

impl Slot {
    fn new() -> Self {
        Slot { state: Mutex::new(SlotState { value: None, wakers: Vec::new() }), condvar: Condvar::new() }
    }

    fn set(&self, completed: Completed) {
        let mut state = self.state.lock().unwrap();
        state.value = Some(completed);
        for waker in state.wakers.drain(..) {
            waker.wake();
        }
        self.condvar.notify_all();
    }

    /// Blocking wait with a deadline, for the checks that expect nothing to
    /// arrive. A host must never wait unbounded: an unbounded wait on a
    /// completion that never comes hangs forever (mechanics spike finding).
    pub fn wait_timeout(&self, timeout: Duration) -> Option<Completed> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(value) = state.value.take() {
                return Some(value);
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            state = self.condvar.wait_timeout(state, deadline - now).unwrap().0;
        }
    }
}

/// The consumer's awaiting task. Polls the slot and parks the task's waker in it
/// until the plugin's callback fills the slot.
pub struct Await(Arc<Slot>);

impl Future for Await {
    type Output = Completed;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Completed> {
        let mut state = self.0.state.lock().unwrap();
        match state.value.take() {
            Some(value) => Poll::Ready(value),
            None => {
                state.wakers.push(context.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// The host the consumer drives the plugin through.
///
/// It owns the host table, so it must not move while a plugin thread could call
/// a callback: `new` leaks one adapter and hands out a `'static` borrow instead
/// of inventing a lifetime.
pub struct HostAdapter {
    api: MetisHostApiV1,
    plugin_table: AtomicPtr<MetisPluginTableV1>,
    slots: Mutex<HashMap<u64, Arc<Slot>>>,
    logs: Mutex<Vec<String>>,
    host_allocs: AtomicUsize,
    released: AtomicUsize,
    completions: AtomicUsize,
    progress_callbacks: AtomicUsize,
    event_callbacks: AtomicUsize,
    late_completions: AtomicUsize,
    inline_returns: AtomicUsize,
    pending_returns: AtomicUsize,
    pending_with_buffer: AtomicUsize,
    callback_panics: AtomicUsize,
}

impl HostAdapter {
    pub fn new() -> &'static HostAdapter {
        let mut boxed = Box::new(HostAdapter {
            api: MetisHostApiV1 {
                size: mem::size_of::<MetisHostApiV1>(),
                user_data: std::ptr::null_mut(),
                complete: host_complete,
                progress: host_progress,
                emit_event: host_emit_event,
                log: host_log,
            },
            plugin_table: AtomicPtr::new(std::ptr::null_mut()),
            slots: Mutex::new(HashMap::new()),
            logs: Mutex::new(Vec::new()),
            host_allocs: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
            completions: AtomicUsize::new(0),
            progress_callbacks: AtomicUsize::new(0),
            event_callbacks: AtomicUsize::new(0),
            late_completions: AtomicUsize::new(0),
            inline_returns: AtomicUsize::new(0),
            pending_returns: AtomicUsize::new(0),
            pending_with_buffer: AtomicUsize::new(0),
            callback_panics: AtomicUsize::new(0),
        });
        boxed.api.user_data = (&*boxed) as *const HostAdapter as *mut c_void;
        Box::leak(boxed)
    }

    /// The table handed to the plugin at `create_instance`.
    pub fn api(&self) -> *const MetisHostApiV1 {
        &self.api
    }

    /// Tells the adapter which plugin table to release plugin buffers through.
    pub fn attach(&self, table: *const MetisPluginTableV1) {
        self.plugin_table.store(table as *mut MetisPluginTableV1, SeqCst);
    }

    /// D11: the pending record is created before the plugin is called. The
    /// consumer holds the returned future and awaits it.
    pub fn register(&self, request_id: u64) -> Await {
        let slot = Arc::new(Slot::new());
        self.slots.lock().unwrap().insert(request_id, Arc::clone(&slot));
        Await(slot)
    }

    /// Calls `invoke` with a host-allocated request buffer. The buffer is valid
    /// only for the call and is released here, by its allocator (D6).
    ///
    /// # Safety
    ///
    /// `instance` must be the live instance handle for `capability`.
    pub unsafe fn invoke(
        &self,
        instance: *mut c_void,
        capability: &str,
        request_id: u64,
        body: &[u8],
    ) -> MetisStatus {
        let table = self.plugin_table.load(SeqCst);
        if table.is_null() {
            return MetisStatus::Internal;
        }
        let request = metis_encode_payload(body);
        self.host_allocs.fetch_add(1, SeqCst);
        let borrowed = MetisBuffer {
            ptr: request.as_ptr() as *mut u8,
            len: request.len(),
            cap: request.capacity(),
            owner: METIS_BUFFER_OWNER_HOST,
            reserved: 0,
        };
        let mut out = MetisBuffer::empty();
        let invoke_fn = unsafe { (*table).invoke };
        let status = unsafe {
            invoke_fn(
                instance,
                capability.as_ptr(),
                capability.len(),
                request_id,
                borrowed,
                self.api(),
                &mut out,
            )
        };
        if status == MetisStatus::Ok {
            // Inline completion is out of scope for this spike. The adapter
            // still has to dispose of the plugin buffer correctly, and the
            // counter proves the path was never taken.
            self.inline_returns.fetch_add(1, SeqCst);
            let _ = self.read_and_release(out);
        }
        if status == MetisStatus::Pending && !out.ptr.is_null() {
            // The other half of "always pending": a pending return must not
            // carry a buffer.
            self.pending_with_buffer.fetch_add(1, SeqCst);
            self.log_line("host: a pending return carried a buffer".to_string());
        }
        if status == MetisStatus::Pending {
            self.pending_returns.fetch_add(1, SeqCst);
        }
        drop(request);
        status
    }

    /// Cancels a request. A successful cancel also retires the host's pending
    /// record, because no completion is guaranteed after it (D12, revised by
    /// the mechanics spike).
    ///
    /// # Safety
    ///
    /// `instance` must be the live instance handle.
    pub unsafe fn cancel(&self, instance: *mut c_void, request_id: u64) -> MetisStatus {
        let table = self.plugin_table.load(SeqCst);
        if table.is_null() {
            return MetisStatus::Internal;
        }
        let cancel_fn = unsafe { (*table).cancel };
        let status = unsafe { cancel_fn(instance, request_id) };
        if status == MetisStatus::Ok {
            self.slots.lock().unwrap().remove(&request_id);
        }
        status
    }

    /// Blocking, bounded wait for the checks that expect no completion.
    pub fn wait_timeout(&self, request_id: u64, timeout: Duration) -> Option<Completed> {
        let slot = self.slots.lock().unwrap().get(&request_id).cloned();
        slot.and_then(|slot| slot.wait_timeout(timeout))
    }

    /// Copies a plugin payload out, validating the D4 header, then releases the
    /// buffer through the plugin's own allocator (D6).
    pub fn read_and_release(&self, buffer: MetisBuffer) -> Result<Vec<u8>, String> {
        if buffer.ptr.is_null() {
            return Err("null buffer from the plugin".to_string());
        }
        // SAFETY: the plugin allocated this buffer with length `len`.
        let bytes = unsafe { slice::from_raw_parts(buffer.ptr, buffer.len) };
        let body = match metis_payload_body(bytes) {
            Ok(body) => body.to_vec(),
            Err(status) => {
                self.release_plugin_buffer(buffer);
                return Err(format!("payload header rejected with {}", status.as_str()));
            }
        };
        self.release_plugin_buffer(buffer);
        Ok(body)
    }

    fn release_plugin_buffer(&self, buffer: MetisBuffer) -> MetisStatus {
        let table = self.plugin_table.load(SeqCst);
        if table.is_null() {
            return MetisStatus::Internal;
        }
        if buffer.owner != METIS_BUFFER_OWNER_PLUGIN {
            self.log_line(format!("host: refusing to release a buffer with owner={}", buffer.owner));
            return MetisStatus::InvalidArgument;
        }
        if buffer.ptr.is_null() {
            return MetisStatus::Ok;
        }
        let release = unsafe { (*table).release_buffer };
        let status = unsafe { release(buffer) };
        if status == MetisStatus::Ok {
            self.released.fetch_add(1, SeqCst);
        } else {
            self.log_line(format!("host: release_buffer returned {}", status.as_str()));
        }
        status
    }

    // --- observation, for the consumer's checks ---------------------------

    pub fn log_line(&self, line: String) {
        self.logs.lock().unwrap().push(line);
    }

    pub fn logs(&self) -> Vec<String> {
        self.logs.lock().unwrap().clone()
    }

    pub fn saw_log(&self, needle: &str) -> bool {
        self.logs().iter().any(|line| line.contains(needle))
    }

    pub fn account(&self) -> Option<HashMap<String, usize>> {
        let line = self.logs().into_iter().find(|line| line.contains("SPIKE_TRAIT_ACCOUNT"))?;
        let mut fields = HashMap::new();
        for token in line.split_whitespace() {
            if let Some((key, value)) = token.split_once('=') {
                fields.insert(key.to_string(), value.parse().unwrap_or(usize::MAX));
            }
        }
        Some(fields)
    }

    pub fn host_allocs(&self) -> usize {
        self.host_allocs.load(SeqCst)
    }

    pub fn released(&self) -> usize {
        self.released.load(SeqCst)
    }

    pub fn completions(&self) -> usize {
        self.completions.load(SeqCst)
    }

    pub fn progress_callbacks(&self) -> usize {
        self.progress_callbacks.load(SeqCst)
    }

    pub fn event_callbacks(&self) -> usize {
        self.event_callbacks.load(SeqCst)
    }

    pub fn late_completions(&self) -> usize {
        self.late_completions.load(SeqCst)
    }

    pub fn inline_returns(&self) -> usize {
        self.inline_returns.load(SeqCst)
    }

    pub fn pending_with_buffer(&self) -> usize {
        self.pending_with_buffer.load(SeqCst)
    }

    pub fn pending_returns(&self) -> usize {
        self.pending_returns.load(SeqCst)
    }

    pub fn callback_panics(&self) -> usize {
        self.callback_panics.load(SeqCst)
    }
}

// --- host callbacks --------------------------------------------------------

unsafe extern "C" fn host_complete(
    user_data: *mut c_void,
    request_id: u64,
    status: MetisStatus,
    payload: MetisBuffer,
) {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user_data` is the leaked adapter for the whole run.
        let adapter = unsafe { &*(user_data as *const HostAdapter) };
        adapter.completions.fetch_add(1, SeqCst);
        let body = if payload.ptr.is_null() {
            Vec::new()
        } else {
            // SAFETY: the plugin allocated this buffer with length `len`.
            let bytes = unsafe { slice::from_raw_parts(payload.ptr, payload.len) };
            match metis_payload_body(bytes) {
                Ok(body) => body.to_vec(),
                Err(status) => {
                    adapter.log_line(format!(
                        "host: completion payload header rejected with {}",
                        status.as_str()
                    ));
                    Vec::new()
                }
            }
        };
        // D6 applies even when the payload is discarded.
        let released = adapter.release_plugin_buffer(payload);
        let slot = adapter.slots.lock().unwrap().remove(&request_id);
        match slot {
            Some(slot) => slot.set(Completed { status, body }),
            None => {
                // D12: a completion after a successful cancel is dropped, but
                // its buffer still goes back through the plugin's allocator.
                adapter.late_completions.fetch_add(1, SeqCst);
                adapter.log_line(format!(
                    "host: dropped a late completion for {request_id} (released={})",
                    released == MetisStatus::Ok
                ));
            }
        }
    }));
    if caught.is_err() {
        // The callback is itself a boundary (D7, amended): count it and leave.
        let adapter = unsafe { &*(user_data as *const HostAdapter) };
        adapter.callback_panics.fetch_add(1, SeqCst);
    }
}

unsafe extern "C" fn host_progress(user_data: *mut c_void, request_id: u64, payload: MetisBuffer) {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        let adapter = unsafe { &*(user_data as *const HostAdapter) };
        adapter.progress_callbacks.fetch_add(1, SeqCst);
        let body = if payload.ptr.is_null() {
            Vec::new()
        } else {
            let bytes = unsafe { slice::from_raw_parts(payload.ptr, payload.len) };
            metis_payload_body(bytes).map(|body| body.to_vec()).unwrap_or_default()
        };
        adapter.log_line(format!(
            "host: progress {request_id} {}",
            String::from_utf8_lossy(&body)
        ));
        adapter.release_plugin_buffer(payload);
    }));
    if caught.is_err() {
        let adapter = unsafe { &*(user_data as *const HostAdapter) };
        adapter.callback_panics.fetch_add(1, SeqCst);
    }
}

unsafe extern "C" fn host_emit_event(
    user_data: *mut c_void,
    topic: *const u8,
    topic_len: usize,
    payload: MetisBuffer,
) {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        let adapter = unsafe { &*(user_data as *const HostAdapter) };
        adapter.event_callbacks.fetch_add(1, SeqCst);
        let topic = if topic.is_null() {
            String::new()
        } else {
            String::from_utf8_lossy(unsafe { slice::from_raw_parts(topic, topic_len) }).into_owned()
        };
        let body = if payload.ptr.is_null() {
            Vec::new()
        } else {
            let bytes = unsafe { slice::from_raw_parts(payload.ptr, payload.len) };
            metis_payload_body(bytes).map(|body| body.to_vec()).unwrap_or_default()
        };
        adapter.log_line(format!("host: event {topic} {}", String::from_utf8_lossy(&body)));
        adapter.release_plugin_buffer(payload);
    }));
    if caught.is_err() {
        let adapter = unsafe { &*(user_data as *const HostAdapter) };
        adapter.callback_panics.fetch_add(1, SeqCst);
    }
}

unsafe extern "C" fn host_log(user_data: *mut c_void, _level: i32, message: *const u8, message_len: usize) {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        let adapter = unsafe { &*(user_data as *const HostAdapter) };
        let message = if message.is_null() {
            String::new()
        } else {
            String::from_utf8_lossy(unsafe { slice::from_raw_parts(message, message_len) }).into_owned()
        };
        adapter.log_line(message);
    }));
    if caught.is_err() {
        let adapter = unsafe { &*(user_data as *const HostAdapter) };
        adapter.callback_panics.fetch_add(1, SeqCst);
    }
}
