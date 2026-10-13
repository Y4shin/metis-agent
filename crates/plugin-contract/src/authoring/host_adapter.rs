//! The host side of the translation: `MetisHostApiV1` on one side, an awaitable
//! completion on the other.
//!
//! A host that drives a plugin registers the pending record *before* it calls
//! `invoke` (D11), then awaits the completion the plugin's `complete` callback
//! resolves. Three obligations come with the callbacks:
//!
//! - every plugin-owned buffer that arrives is released through the plugin's
//!   own `release_buffer`, including one this adapter decides to drop because a
//!   cancel won the race (D6, D12);
//! - every callback body is a boundary, because a callback is itself an
//!   `extern "C"` frame reached from plugin code and a panic that escaped it
//!   would unwind into plugin frames (D7 as amended);
//! - a host never waits unbounded, because a completion that never arrives
//!   looks exactly like a hang (mechanics spike finding).
//!
//! The adapter is the shape the authoring spike measured, ported: a slot per
//! pending request, a callback table pointing back at it through `user_data`.

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

use crate::abi::{
    METIS_BUFFER_OWNER_HOST, METIS_BUFFER_OWNER_PLUGIN, METIS_LOG_INFO, METIS_LOG_WARN,
    MetisBuffer, MetisHostApiV1, MetisPluginTableV1, MetisStatus,
};
use crate::payload::{metis_encode_payload, metis_payload_body};

/// One completion as the host sees it.
///
/// The body has already been copied out of the plugin's buffer, which has been
/// released by the time this exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completed {
    /// The status the plugin reported.
    pub status: MetisStatus,
    /// The envelope-stripped body, empty for a status that carries none.
    pub body: Vec<u8>,
}

struct SlotState {
    value: Option<Completed>,
    wakers: Vec<Waker>,
}

struct Slot {
    state: Mutex<SlotState>,
    condvar: Condvar,
}

impl Slot {
    fn new() -> Self {
        Slot {
            state: Mutex::new(SlotState {
                value: None,
                wakers: Vec::new(),
            }),
            condvar: Condvar::new(),
        }
    }

    fn set(&self, completed: Completed) {
        let mut state = self.state.lock().unwrap();
        state.value = Some(completed);
        for waker in state.wakers.drain(..) {
            waker.wake();
        }
        self.condvar.notify_all();
    }

    /// Blocking, bounded wait.
    fn wait_timeout(&self, timeout: Duration) -> Option<Completed> {
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

/// The awaiting half of the pending path: what [`HostAdapter::register`]
/// returns.
///
/// It is a future, so a host on a real executor awaits it, and it also carries
/// the blocking bounded wait a synchronous host uses.
pub struct Await(Arc<Slot>);

impl Await {
    /// Waits for the completion, giving up after `timeout` and returning
    /// `None`.
    pub fn wait_timeout(&self, timeout: Duration) -> Option<Completed> {
        self.0.wait_timeout(timeout)
    }
}

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

struct Inner {
    api: MetisHostApiV1,
    plugin: AtomicPtr<MetisPluginTableV1>,
    slots: Mutex<HashMap<u64, Arc<Slot>>>,
    events: Mutex<Vec<(String, Vec<u8>)>>,
    logs: Mutex<Vec<(i32, String)>>,
    callback_panics: AtomicUsize,
}

/// The host side of the translation, boxed so the callback table it hands to the
/// plugin keeps its address.
///
/// It must outlive every call and every plugin thread that may call back into
/// it: keep it alive until the instance has been shut down.
pub struct HostAdapter {
    inner: Box<Inner>,
}

impl Default for HostAdapter {
    fn default() -> Self {
        HostAdapter::new()
    }
}

impl HostAdapter {
    /// Builds the adapter and its callback table.
    pub fn new() -> Self {
        let mut inner = Box::new(Inner {
            api: MetisHostApiV1 {
                size: mem::size_of::<MetisHostApiV1>(),
                user_data: std::ptr::null_mut(),
                complete: host_complete,
                progress: host_progress,
                emit_event: host_emit_event,
                log: host_log,
            },
            plugin: AtomicPtr::new(std::ptr::null_mut()),
            slots: Mutex::new(HashMap::new()),
            events: Mutex::new(Vec::new()),
            logs: Mutex::new(Vec::new()),
            callback_panics: AtomicUsize::new(0),
        });
        // The box keeps this address for as long as the adapter lives, which is
        // what lets the plugin hand it back as `user_data`.
        inner.api.user_data = (&*inner) as *const Inner as *mut c_void;
        HostAdapter { inner }
    }

    /// The callback table to hand to `create_instance` and `invoke`.
    pub fn api(&self) -> *const MetisHostApiV1 {
        &self.inner.api
    }

    /// Records which plugin table owns the buffers that arrive, so they can be
    /// released through the plugin's own `release_buffer` (D6).
    pub fn attach(&self, table: *const MetisPluginTableV1) {
        self.inner
            .plugin
            .store(table as *mut MetisPluginTableV1, SeqCst);
    }

    /// Registers a pending request and hands back its awaitable completion.
    ///
    /// D11: register before `invoke`, so a completion that overtakes the return
    /// still finds its request.
    pub fn register(&self, request_id: u64) -> Await {
        let slot = Arc::new(Slot::new());
        self.inner
            .slots
            .lock()
            .unwrap()
            .insert(request_id, Arc::clone(&slot));
        Await(slot)
    }

    /// Calls `invoke` with a host-allocated request buffer.
    ///
    /// The buffer is valid only for the call and is released here, by its
    /// allocator (D6).
    ///
    /// # Safety
    ///
    /// `instance` must be the live instance handle for this plugin.
    pub unsafe fn invoke(
        &self,
        instance: *mut c_void,
        capability: &str,
        request_id: u64,
        body: &[u8],
    ) -> MetisStatus {
        let table = self.inner.plugin.load(SeqCst);
        if table.is_null() {
            return MetisStatus::Internal;
        }
        let request = metis_encode_payload(body);
        let borrowed = MetisBuffer {
            ptr: request.as_ptr() as *mut u8,
            len: request.len(),
            cap: request.capacity(),
            owner: METIS_BUFFER_OWNER_HOST,
            reserved: 0,
        };
        let mut out = MetisBuffer::empty();
        // SAFETY: the caller guarantees a live instance, and the table is the
        // one `attach` recorded.
        let invoke = unsafe { (*table).invoke };
        let status = unsafe {
            invoke(
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
            // The trait path never finishes inline; a direct ABI implementer
            // can, and its payload is still the host's to release.
            let _ = self.read_and_release(out);
        }
        drop(request);
        status
    }

    /// Requests cancellation of a pending request.
    ///
    /// A successful cancel retires the host's pending record too: no completion
    /// is guaranteed after it, and ignoring late callbacks alone would leak one
    /// record per cancelled request (D12 as amended).
    ///
    /// # Safety
    ///
    /// `instance` must be the live instance handle.
    pub unsafe fn cancel(&self, instance: *mut c_void, request_id: u64) -> MetisStatus {
        let table = self.inner.plugin.load(SeqCst);
        if table.is_null() {
            return MetisStatus::Internal;
        }
        // SAFETY: the caller guarantees a live instance, and the table is the
        // one `attach` recorded.
        let cancel = unsafe { (*table).cancel };
        let status = unsafe { cancel(instance, request_id) };
        if status == MetisStatus::Ok {
            self.inner.slots.lock().unwrap().remove(&request_id);
        }
        status
    }

    /// Copies a plugin payload out, validating the envelope, then releases the
    /// buffer through the plugin's own allocator (D4, D6).
    pub fn read_and_release(&self, buffer: MetisBuffer) -> Result<Vec<u8>, String> {
        if buffer.ptr.is_null() {
            return Ok(Vec::new());
        }
        // SAFETY: the plugin allocated `len` bytes at `ptr`, and the buffer
        // stays valid until `release_buffer` is called below.
        let bytes = unsafe { slice::from_raw_parts(buffer.ptr, buffer.len) };
        let body = metis_payload_body(bytes).map(<[u8]>::to_vec);
        self.release_plugin_buffer(buffer);
        body.map_err(|status| format!("the payload envelope was rejected with {}", status.as_str()))
    }

    /// Releases a plugin-owned buffer through the plugin's `release_buffer`.
    pub fn release_plugin_buffer(&self, buffer: MetisBuffer) -> MetisStatus {
        let table = self.inner.plugin.load(SeqCst);
        if table.is_null() {
            return MetisStatus::Internal;
        }
        if buffer.owner != METIS_BUFFER_OWNER_PLUGIN {
            self.inner.logs.lock().unwrap().push((
                METIS_LOG_WARN,
                format!(
                    "host: refusing to release a buffer with owner={}",
                    buffer.owner
                ),
            ));
            return MetisStatus::InvalidArgument;
        }
        if buffer.ptr.is_null() {
            return MetisStatus::Ok;
        }
        // SAFETY: the table is the attached plugin's, and the buffer came from
        // that plugin.
        let release = unsafe { (*table).release_buffer };
        unsafe { release(buffer) }
    }

    /// The log lines the plugin sent, with their levels.
    pub fn logs(&self) -> Vec<(i32, String)> {
        self.inner.logs.lock().unwrap().clone()
    }

    /// Whether any log line contains `needle`.
    pub fn saw_log(&self, needle: &str) -> bool {
        self.logs().iter().any(|(_, line)| line.contains(needle))
    }

    /// The events the plugin sent: topic and envelope-stripped body.
    pub fn events(&self) -> Vec<(String, Vec<u8>)> {
        self.inner.events.lock().unwrap().clone()
    }

    /// How many callback bodies panicked and were caught at their own frame.
    pub fn callback_panics(&self) -> usize {
        self.inner.callback_panics.load(SeqCst)
    }
}

// --- host callbacks --------------------------------------------------------
//
// Each callback releases its plugin buffer *before* the catch. The release must
// happen exactly once, and the allocator reports a second one as a violation
// (D6), so it is not something to retry from a catch path. The bookkeeping that
// follows is still a boundary: a panic there is caught at the callback's own
// frame (D7 as amended) and turned into an internal-error completion, so the
// caller is never parked on a completion that will not arrive.

unsafe extern "C" fn host_complete(
    user_data: *mut c_void,
    request_id: u64,
    status: MetisStatus,
    payload: MetisBuffer,
) {
    // SAFETY: `user_data` is the adapter's boxed state, alive for the whole run,
    // and this callback may run on any plugin thread (D12).
    let inner = unsafe { &*(user_data as *const Inner) };
    let body = unsafe { take(inner, payload) };
    let caught = catch_unwind(AssertUnwindSafe(|| {
        let slot = inner.slots.lock().unwrap().remove(&request_id);
        match slot {
            Some(slot) => slot.set(Completed { status, body }),
            // A completion that arrives after a successful cancel is dropped,
            // and its buffer has already been released (D6, D12).
            None => drop_late(inner, request_id),
        }
    }));
    if caught.is_err() {
        inner.callback_panics.fetch_add(1, SeqCst);
        if let Some(slot) = inner.slots.lock().unwrap().remove(&request_id) {
            slot.set(Completed {
                status: MetisStatus::Internal,
                body: Vec::new(),
            });
        }
    }
}

unsafe extern "C" fn host_progress(user_data: *mut c_void, request_id: u64, payload: MetisBuffer) {
    // SAFETY: as in `host_complete`.
    let inner = unsafe { &*(user_data as *const Inner) };
    let body = unsafe { take(inner, payload) };
    let caught = catch_unwind(AssertUnwindSafe(|| {
        inner.logs.lock().unwrap().push((
            METIS_LOG_INFO,
            format!(
                "host: progress for request {request_id}: {}",
                String::from_utf8_lossy(&body)
            ),
        ));
    }));
    if caught.is_err() {
        inner.callback_panics.fetch_add(1, SeqCst);
    }
}

unsafe extern "C" fn host_emit_event(
    user_data: *mut c_void,
    topic: *const u8,
    topic_len: usize,
    payload: MetisBuffer,
) {
    // SAFETY: as in `host_complete`.
    let inner = unsafe { &*(user_data as *const Inner) };
    // SAFETY: the topic is borrowed from the plugin for the duration of the call
    // (D6).
    let topic = if topic.is_null() {
        String::new()
    } else {
        String::from_utf8_lossy(unsafe { slice::from_raw_parts(topic, topic_len) }).into_owned()
    };
    let body = unsafe { take(inner, payload) };
    let caught = catch_unwind(AssertUnwindSafe(|| {
        inner.events.lock().unwrap().push((topic, body));
    }));
    if caught.is_err() {
        inner.callback_panics.fetch_add(1, SeqCst);
    }
}

unsafe extern "C" fn host_log(
    user_data: *mut c_void,
    level: i32,
    message: *const u8,
    message_len: usize,
) {
    // SAFETY: as in `host_complete`.
    let inner = unsafe { &*(user_data as *const Inner) };
    // SAFETY: a log message is borrowed, not allocated, so there is nothing to
    // release (D6).
    let message = if message.is_null() {
        String::new()
    } else {
        String::from_utf8_lossy(unsafe { slice::from_raw_parts(message, message_len) }).into_owned()
    };
    let caught = catch_unwind(AssertUnwindSafe(|| {
        inner.logs.lock().unwrap().push((level, message));
    }));
    if caught.is_err() {
        inner.callback_panics.fetch_add(1, SeqCst);
    }
}

/// Copies a plugin payload out and releases it, exactly once.
///
/// # Safety
///
/// `inner` must be the adapter's live state, and `buffer` the plugin buffer the
/// callback received.
unsafe fn take(inner: &Inner, buffer: MetisBuffer) -> Vec<u8> {
    if buffer.ptr.is_null() {
        return Vec::new();
    }
    // SAFETY: the plugin allocated `len` bytes at `ptr`, and the buffer stays
    // valid until it is released below.
    let bytes = unsafe { slice::from_raw_parts(buffer.ptr, buffer.len) };
    let body = match metis_payload_body(bytes) {
        Ok(body) => body.to_vec(),
        Err(status) => {
            inner.logs.lock().unwrap().push((
                METIS_LOG_WARN,
                format!(
                    "host: the payload envelope was rejected with {}",
                    status.as_str()
                ),
            ));
            Vec::new()
        }
    };
    unsafe { release(inner, buffer) };
    body
}

/// Releases a plugin-owned buffer through the plugin's `release_buffer`.
///
/// # Safety
///
/// `inner` must be the adapter's live state, and `buffer` a plugin-owned buffer
/// the plugin allocated.
unsafe fn release(inner: &Inner, buffer: MetisBuffer) {
    let table = inner.plugin.load(SeqCst);
    if table.is_null() || buffer.ptr.is_null() {
        return;
    }
    if buffer.owner != METIS_BUFFER_OWNER_PLUGIN {
        inner.logs.lock().unwrap().push((
            METIS_LOG_WARN,
            format!(
                "host: refusing to release a buffer with owner={}",
                buffer.owner
            ),
        ));
        return;
    }
    // SAFETY: the table is the attached plugin's, and the buffer came from that
    // plugin.
    let release = unsafe { (*table).release_buffer };
    let status = unsafe { release(buffer) };
    if status != MetisStatus::Ok {
        inner.logs.lock().unwrap().push((
            METIS_LOG_WARN,
            format!("host: release_buffer returned {}", status.as_str()),
        ));
    }
}

fn drop_late(inner: &Inner, request_id: u64) {
    inner.logs.lock().unwrap().push((
        METIS_LOG_INFO,
        format!("host: dropped a completion for {request_id}; its record had already been retired"),
    ));
}
