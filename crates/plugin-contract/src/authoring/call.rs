//! The per-request context and the instance-level host routes.
//!
//! Both exist only because the ABI is asynchronous and out of band. A plain
//! Rust capability method would return its result; here a call has a request id
//! the host invents, a cancel arrives on another thread and has to reach a
//! future parked on I/O, progress is a separate callback, and a capability that
//! must wait asks the host for a service and is answered later through
//! `service_complete`. Everything the ABI forced onto the authoring surface is
//! collected here, so the trait itself stays free of it.

use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, Waker};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::abi::{
    METIS_LOG_DEBUG, METIS_LOG_ERROR, METIS_LOG_INFO, METIS_LOG_WARN, MetisHostApiV1, MetisStatus,
    metis_host_api_has_service_request,
};
use crate::authoring::alloc::plugin_alloc;
use crate::authoring::traits::PluginError;
use crate::payload::metis_encode_payload;

/// A raw pointer the adapter vouches for.
///
/// A future may be polled from any host thread, so the host table travels into
/// the futures the adapter hands out; the adapter asserts that the host keeps
/// it alive for the instance's lifetime. The pointer is only reachable through
/// `get`, so a closure or an async block that uses the host table captures the
/// whole `SendPtr` (which is `Send`) and not the bare pointer (which is not).
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
// valid and immutable for the instance's lifetime, and every callback that
// receives it is required to be thread-safe.
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
    /// The frozen `METIS_LOG_*` level this build sends.
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
/// A capability receives a [`Call`], which adds the request id, the
/// cancellation token, the progress route, and the service route. The lifecycle
/// hooks receive a `Host`, because there is no request to attach them to.
///
/// Every payload handed to [`Host::emit_event`] is serialised here, wrapped in
/// the payload envelope, and given a buffer the *host* releases: the author
/// writes a topic and a value.
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

    /// The host table this instance was created with.
    pub(crate) fn api(self) -> *const MetisHostApiV1 {
        self.api.get()
    }

    /// Sends a plugin-originated event: a topic, and a payload serialised from
    /// `payload`.
    ///
    /// The payload buffer is plugin-allocated and released by the receiver.
    pub fn emit_event<T>(&self, topic: &str, payload: &T) -> Result<(), PluginError>
    where
        T: Serialize + ?Sized,
    {
        let body = encode("event", payload)?;
        let buffer = plugin_alloc(&metis_encode_payload(&body));
        // SAFETY: the host table outlives the instance, and a future may be
        // polled from any host thread.
        let api = unsafe { &*self.api.get() };
        let emit_event = api.emit_event;
        unsafe { emit_event(api.user_data, topic.as_ptr(), topic.len(), buffer) };
        Ok(())
    }

    /// Logs one line through the host's `log` callback.
    ///
    /// The message is borrowed for the duration of the call.
    pub fn log(&self, level: LogLevel, message: &str) {
        // SAFETY: as in `emit_event`.
        let api = unsafe { &*self.api.get() };
        let log = api.log;
        unsafe { log(api.user_data, level.wire(), message.as_ptr(), message.len()) };
    }

    /// Whether the host table this plugin holds covers `service_request`.
    ///
    /// Presence is decided by the host table's `size`, never by a pointer, so a
    /// host built without the route is detectable instead of a call into
    /// whatever sits past the end of its table.
    pub fn has_service_request(&self) -> bool {
        // SAFETY: as in `emit_event`.
        let api = unsafe { &*self.api.get() };
        metis_host_api_has_service_request(api.size)
    }
}

fn encode<T>(what: &str, payload: &T) -> Result<Vec<u8>, PluginError>
where
    T: Serialize + ?Sized,
{
    serde_json::to_vec(payload)
        .map_err(|err| PluginError::Internal(format!("{what} payload: {err}")))
}

/// The status a host answer carries, as an authoring error.
pub(crate) fn plugin_error(status: MetisStatus) -> PluginError {
    match status {
        MetisStatus::InvalidArgument => {
            PluginError::InvalidArgument("the host refused the service request".to_string())
        }
        MetisStatus::NotFound => {
            PluginError::NotFound("the host does not know that service".to_string())
        }
        MetisStatus::PermissionDenied => {
            PluginError::PermissionDenied("the host refused the service".to_string())
        }
        MetisStatus::Cancelled => PluginError::Cancelled,
        MetisStatus::Unsupported => {
            PluginError::Unsupported("the host does not implement that service".to_string())
        }
        MetisStatus::Internal | MetisStatus::Ok | MetisStatus::Pending => PluginError::Internal(
            format!("the host answered a service with {}", status.as_str()),
        ),
    }
}

struct Cancellation {
    cancelled: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// The cancellation token of one call.
///
/// The token carries a waker, so a future parked on it is woken from the thread
/// that accepted the cancel instead of polling; [`CancellationToken::is_cancelled`]
/// stays as the poll convenience.
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

// --- host services --------------------------------------------------------

/// One outstanding host service, keyed by the call's request id.
///
/// The plugin owns this side: `service_request` registers it, the host's
/// `service_complete` resolves it, and the awaiting future reads the answer and
/// removes it. It is the whole of "the plugin parks and the host answers": no
/// thread, no runtime, and no completion callback.
struct ServiceSlot {
    answer: Mutex<Option<Result<Vec<u8>, MetisStatus>>>,
    waker: Mutex<Option<Waker>>,
}

fn services() -> &'static Mutex<HashMap<u64, Arc<ServiceSlot>>> {
    static SERVICES: OnceLock<Mutex<HashMap<u64, Arc<ServiceSlot>>>> = OnceLock::new();
    SERVICES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolves an outstanding host service, waking the future that awaited it.
///
/// Returns whether a service with that request id was outstanding, which is
/// what makes a stray or duplicated `service_complete` visible instead of
/// silent.
pub(crate) fn resolve_service(request_id: u64, status: MetisStatus, body: Vec<u8>) -> bool {
    let slot = services().lock().unwrap().get(&request_id).cloned();
    let Some(slot) = slot else {
        return false;
    };
    let answer = match status {
        MetisStatus::Ok => Ok(body),
        other => Err(other),
    };
    *slot.answer.lock().unwrap() = Some(answer);
    let waker = slot.waker.lock().unwrap().take();
    if let Some(waker) = waker {
        waker.wake();
    }
    true
}

/// The future [`Call::service_request`] returns: the host's answer to one
/// service.
///
/// It is a plain `std::task` future, so the host polls it with everything else
/// that call holds. While it is pending, the host is free: the answer wakes the
/// poll through the waker stored here.
pub struct ServiceAnswer<R> {
    request_id: u64,
    slot: Arc<ServiceSlot>,
    response: PhantomData<fn() -> R>,
}

impl<R> Drop for ServiceAnswer<R> {
    fn drop(&mut self) {
        // A future that is dropped before its answer arrives (a cancel, or a
        // host that gave up) must not leave the slot registered.
        services().lock().unwrap().remove(&self.request_id);
    }
}

impl<R: DeserializeOwned> Future for ServiceAnswer<R> {
    type Output = Result<R, PluginError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(answer) = this.slot.answer.lock().unwrap().take() {
            return Poll::Ready(match answer {
                Ok(body) => serde_json::from_slice(&body)
                    .map_err(|err| PluginError::Internal(format!("service answer body: {err}"))),
                Err(status) => Err(plugin_error(status)),
            });
        }
        *this.slot.waker.lock().unwrap() = Some(context.waker().clone());
        Poll::Pending
    }
}

// --- the per-request context ---------------------------------------------

/// The context handed to every capability.
///
/// Cheap to clone, because the adapter keeps one clone in its cancellation
/// registry so that a later `cancel` reaches the same token the call is parked
/// on.
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
    /// The poll convenience: prefer [`Call::cancellation`] when the call is
    /// parked, so it is woken instead of polling.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// The cancellation token: await [`CancellationToken::cancelled`] to be
    /// woken when the cancel is accepted.
    pub fn cancellation(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Emits progress for this in-flight call.
    pub fn progress<T>(&self, progress: &T) -> Result<(), PluginError>
    where
        T: Serialize + ?Sized,
    {
        let body = encode("progress", progress)?;
        let buffer = plugin_alloc(&metis_encode_payload(&body));
        // SAFETY: the host table outlives the instance.
        let api = unsafe { &*self.host.api() };
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

    /// Asks the host for a service and returns the future that yields its
    /// answer.
    ///
    /// The request payload is plugin-owned: the host releases it through the
    /// table's `release_buffer`, including from inside this call. The answer
    /// arrives later through the plugin's `service_complete`, which wakes the
    /// returned future; a capability awaits it and its poll returns `Pending`
    /// in the meantime, so the host never blocks on a service. A host built
    /// before the route is refused as unsupported rather than called through.
    pub fn service_request<Req, Resp>(
        &self,
        service: &str,
        request: &Req,
    ) -> Result<ServiceAnswer<Resp>, PluginError>
    where
        Req: Serialize + ?Sized,
        Resp: DeserializeOwned,
    {
        // SAFETY: the host table outlives the instance.
        let api = unsafe { &*self.host.api() };
        // Presence is decided by the host table's `size`, never by a pointer:
        // a host built without the route is refused rather than called through
        // into whatever sits past the end of its table.
        if !metis_host_api_has_service_request(api.size) {
            return Err(PluginError::Unsupported(format!(
                "the host table covers {} bytes, which does not reach the {service} route",
                api.size
            )));
        }
        let body = encode("service request", request)?;
        let slot = Arc::new(ServiceSlot {
            answer: Mutex::new(None),
            waker: Mutex::new(None),
        });
        services()
            .lock()
            .unwrap()
            .insert(self.id, Arc::clone(&slot));
        let buffer = plugin_alloc(&metis_encode_payload(&body));
        let service_request = api.service_request;
        // SAFETY: the callback signature is the frozen one, and every argument
        // is valid for the call.
        let status = unsafe {
            service_request(
                api.user_data,
                service.as_ptr(),
                service.len(),
                self.id,
                buffer,
            )
        };
        if status != MetisStatus::Ok {
            services().lock().unwrap().remove(&self.id);
            return Err(plugin_error(status));
        }
        Ok(ServiceAnswer {
            request_id: self.id,
            slot,
            response: PhantomData,
        })
    }

    pub(crate) fn mark_cancelled(&self) {
        self.token.mark_cancelled();
    }
}
