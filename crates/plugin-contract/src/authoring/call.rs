//! The per-request context and the instance-level host routes.
//!
//! Both exist only because the ABI is asynchronous and out of band. A plain
//! Rust capability method would return its result; here the result arrives
//! through a host callback, progress is a separate callback, a request has an
//! id the host invents, and a cancel arrives on another thread and has to
//! reach a plugin parked on I/O. Everything the ABI forced onto the authoring
//! surface is collected here, so the trait itself stays free of it.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use serde::Serialize;

use crate::abi::{
    METIS_LOG_DEBUG, METIS_LOG_ERROR, METIS_LOG_INFO, METIS_LOG_WARN, MetisBuffer, MetisHostApiV1,
    MetisStatus,
};
use crate::authoring::alloc::plugin_alloc;
use crate::authoring::traits::PluginError;
use crate::payload::metis_encode_payload;

/// A raw pointer the adapter vouches for.
///
/// Callbacks may come from any plugin thread (D12), so the host table travels
/// to the workers; the adapter asserts that the host keeps it alive for the
/// instance's lifetime. The pointer is only reachable through `get`, so a
/// closure or an async block that uses the host table captures the whole
/// `SendPtr` (which is `Send`) and not the bare pointer (which is not).
pub(crate) struct SendPtr<T>(*const T);

// Manual impls: the derived ones would demand `T: Clone` or `T: Copy`, which the
// ABI types are not.
impl<T> Clone for SendPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for SendPtr<T> {}

// SAFETY: the adapter only builds one from a pointer the host guarantees is
// valid and immutable for the instance's lifetime, and the callbacks that
// receive it are required to be thread-safe (D12).
unsafe impl<T> Send for SendPtr<T> {}
unsafe impl<T> Sync for SendPtr<T> {}

impl<T> SendPtr<T> {
    pub(crate) fn new(ptr: *const T) -> Self {
        SendPtr(ptr)
    }

    pub(crate) fn get(self) -> *const T {
        self.0
    }
}

/// The diagnostic level of a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Development detail.
    Debug,
    /// Ordinary progress.
    Info,
    /// Something recoverable went wrong.
    Warn,
    /// Something failed.
    Error,
}

impl LogLevel {
    /// The frozen `METIS_LOG_*` level this build sends (D7).
    pub(crate) const fn wire(self) -> i32 {
        match self {
            LogLevel::Debug => METIS_LOG_DEBUG,
            LogLevel::Info => METIS_LOG_INFO,
            LogLevel::Warn => METIS_LOG_WARN,
            LogLevel::Error => METIS_LOG_ERROR,
        }
    }
}

/// The routes to the host that outlive a single call: events and logs.
///
/// A capability method receives a [`Call`], which adds the request id, the
/// cancellation token, and the progress route. The lifecycle hooks receive a
/// `Host`, because there is no request to attach them to.
///
/// Every payload handed to [`Host::emit_event`] is serialised here, wrapped in
/// the payload envelope, and given a buffer the *host* releases (D4, D6): the
/// author writes a topic and a value.
#[derive(Clone, Copy)]
pub struct Host {
    api: SendPtr<MetisHostApiV1>,
}

impl Host {
    pub(crate) fn from_raw(api: *const MetisHostApiV1) -> Self {
        Host {
            api: SendPtr::new(api),
        }
    }

    /// Sends a plugin-originated event: a topic, and a payload serialised from
    /// `payload`.
    pub fn emit_event<T>(&self, topic: &str, payload: &T) -> Result<(), PluginError>
    where
        T: Serialize + ?Sized,
    {
        let body = encode(topic, payload)?;
        let buffer = plugin_alloc(&metis_encode_payload(&body));
        // SAFETY: the host table outlives the instance, and callbacks may come
        // from any plugin thread (D12).
        let api = unsafe { &*self.api.get() };
        let emit_event = api.emit_event;
        unsafe { emit_event(api.user_data, topic.as_ptr(), topic.len(), buffer) };
        Ok(())
    }

    /// Logs one line through the host's `log` callback.
    ///
    /// The message is borrowed for the duration of the call (D6).
    pub fn log(&self, level: LogLevel, message: &str) {
        // SAFETY: as in `emit_event`.
        let api = unsafe { &*self.api.get() };
        let log = api.log;
        unsafe { log(api.user_data, level.wire(), message.as_ptr(), message.len()) };
    }

    /// Delivers a completion through the host's `complete` callback.
    pub(crate) fn complete(&self, request_id: u64, status: MetisStatus, payload: MetisBuffer) {
        // SAFETY: as in `emit_event`.
        let api = unsafe { &*self.api.get() };
        let complete = api.complete;
        unsafe { complete(api.user_data, request_id, status, payload) };
    }
}

fn encode<T>(what: &str, payload: &T) -> Result<Vec<u8>, PluginError>
where
    T: Serialize + ?Sized,
{
    serde_json::to_vec(payload)
        .map_err(|err| PluginError::Internal(format!("{what} payload: {err}")))
}

struct Cancellation {
    cancelled: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// The cancellation token of one call (D15).
///
/// The spike's poll flag had no waker, so a plugin blocked in I/O could not
/// notice a cancel promptly. This token carries a waker: [`cancelled`] resolves
/// as soon as a cancel is accepted, woken from the thread that accepted it, and
/// [`Call::is_cancelled`] stays as the poll convenience.
///
/// [`cancelled`]: CancellationToken::cancelled
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<Cancellation>,
}

impl CancellationToken {
    pub(crate) fn new() -> Self {
        CancellationToken {
            inner: Arc::new(Cancellation {
                cancelled: AtomicBool::new(false),
                waker: Mutex::new(None),
            }),
        }
    }

    /// Whether a cancel for this call has already been accepted.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// A future that resolves when this call's cancel is accepted.
    pub fn cancelled(&self) -> Cancelled {
        Cancelled {
            inner: Arc::clone(&self.inner),
        }
    }

    pub(crate) fn mark_cancelled(&self) {
        if self.inner.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        let waker = self.inner.waker.lock().unwrap().take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// The future [`CancellationToken::cancelled`] returns.
pub struct Cancelled {
    inner: Arc<Cancellation>,
}

impl Future for Cancelled {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if this.inner.cancelled.load(Ordering::SeqCst) {
            return Poll::Ready(());
        }
        let mut waker = this.inner.waker.lock().unwrap();
        // Re-check under the lock: a cancel that lands between the load above
        // and the store below would otherwise be lost until a later poll.
        if this.inner.cancelled.load(Ordering::SeqCst) {
            return Poll::Ready(());
        }
        *waker = Some(context.waker().clone());
        Poll::Pending
    }
}

/// The per-request context handed to every capability method.
///
/// Cheap to clone, because the adapter keeps one clone in its cancellation
/// registry so that a later `cancel` reaches the same token the method is
/// parked on.
#[derive(Clone)]
pub struct Call {
    id: u64,
    host: Host,
    token: CancellationToken,
}

impl Call {
    pub(crate) fn new(id: u64, host: Host, token: CancellationToken) -> Self {
        Call { id, host, token }
    }

    /// The ABI request id for this call.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Whether a cancel for this call has already been accepted.
    ///
    /// The poll convenience: prefer [`Call::cancellation`] when the arm is
    /// parked on I/O, so it is woken instead of polling.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// The cancellation token: await [`CancellationToken::cancelled`] to be
    /// woken when the cancel is accepted (D15).
    pub fn cancellation(&self) -> CancellationToken {
        self.token.clone()
    }

    /// The instance-level host routes.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// Emits progress for this pending call.
    pub fn progress<T>(&self, progress: &T) -> Result<(), PluginError>
    where
        T: Serialize + ?Sized,
    {
        let body = encode("progress", progress)?;
        let buffer = plugin_alloc(&metis_encode_payload(&body));
        // SAFETY: the host table outlives the instance.
        let api = unsafe { &*self.host.api.get() };
        let progress = api.progress;
        unsafe { progress(api.user_data, self.id, buffer) };
        Ok(())
    }

    /// Sends an event from inside a capability.
    pub fn emit_event<T>(&self, topic: &str, payload: &T) -> Result<(), PluginError>
    where
        T: Serialize + ?Sized,
    {
        self.host.emit_event(topic, payload)
    }

    /// Logs one line from inside a capability.
    pub fn log(&self, level: LogLevel, message: &str) {
        self.host.log(level, message);
    }

    pub(crate) fn mark_cancelled(&self) {
        self.token.mark_cancelled();
    }
}
