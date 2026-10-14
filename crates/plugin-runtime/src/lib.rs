//! The host side of the plugin boundary: loading, the registry, and the driver.
//!
//! `plugin-contract` owns the wire: the tables, the status space, the buffer
//! ownership rules, and the payload envelope. This crate owns what a daemon does
//! with them. It loads a cdylib, checks the version, creates an instance, reads
//! the capability declaration, routes a call by capability name, validates the
//! payload in both directions, polls the plugin's future, and decides when an
//! instance is quarantined and when a library may be unloaded.
//!
//! Two properties shape the design.
//!
//! **The daemon is the only executor.** A plugin future is a plain
//! [`Future`]: the driver hands one back and the caller polls it on whatever
//! runtime it already has. This crate starts no runtime and no thread, so the
//! daemon decides when plugin code runs, which is what makes unloading safe.
//!
//! **Reclaim is the caller's, on the poll return.** A poll that is abandoned
//! quarantines its instance, so no new future is accepted and the library stays
//! resident. Nothing is unloaded while a future is alive, and the ordering is
//! recorded in the host's ledger rather than inferred from a timer.

use std::collections::HashSet;
use std::ffi::c_void;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::pin::Pin;
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use plugin_contract::abi::*;
use plugin_contract::payload::{metis_encode_payload, metis_payload_body};

pub mod registry;

pub use plugin_contract::capability::{CapabilityDescriptor, CapabilityKind, CapabilityList};
pub use registry::{Entry, Registry, RegistryError, RouteError};

/// What can go wrong on the host side of one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// The library could not be loaded, or its entry symbol was missing.
    Load(String),
    /// The table was refused: a different major, a minor older than this
    /// build's, or a table too short for an entry this build needs. A newer
    /// minor inside the major is accepted, because its extra entries are
    /// appended after the last entry this build reads.
    Version(MetisAbiError),
    /// The instance could not be created.
    Create(MetisStatus),
    /// The instance has been quarantined: no new future is accepted from it.
    Quarantined,
    /// The plugin reported this status for the call.
    Status(MetisStatus),
    /// The capability name is not declared by the instance.
    UnknownCapability(String),
    /// A payload was not a payload, or a lifecycle step was attempted at the
    /// wrong time.
    Payload(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Load(detail) => {
                write!(f, "the plugin library could not be loaded: {detail}")
            }
            CallError::Version(error) => write!(f, "{error}"),
            CallError::Create(status) => {
                write!(f, "create_instance returned {}", status.as_str())
            }
            CallError::Quarantined => write!(f, "the instance is quarantined"),
            CallError::Status(status) => write!(f, "the call returned {}", status.as_str()),
            CallError::UnknownCapability(name) => write!(f, "no capability named {name}"),
            CallError::Payload(detail) => write!(f, "{detail}"),
        }
    }
}

impl std::error::Error for CallError {}

impl From<MetisAbiError> for CallError {
    fn from(error: MetisAbiError) -> Self {
        CallError::Version(error)
    }
}

// --- loading --------------------------------------------------------------

/// One loaded plugin library and the table it exports.
///
/// The library stays resident for as long as this value lives, so unloading is
/// the act of dropping the last reference to the instances that came from it.
pub struct Loaded {
    table: *const MetisPluginTableV1,
    /// The library handle, which is the only thing keeping the plugin mapped.
    ///
    /// It is deliberately never read: dropping it is what unloads, so it is
    /// named with a leading underscore to say that it exists for its lifetime
    /// and not for its value.
    _keep_mapped: libloading::Library,
    path: std::path::PathBuf,
    abi: (u32, u32),
}

// SAFETY: the table is the plugin's own, it is immutable for the library's
// lifetime, and the ABI's contract is that every entry may be called from any
// thread. A library handle is itself safe to move between threads. This is what
// lets one instance be shared by a daemon that polls futures on a
// multi-threaded runtime.
unsafe impl Send for Loaded {}
unsafe impl Sync for Loaded {}

impl Loaded {
    /// Loads a cdylib and reads its table through the version gate.
    ///
    /// # Safety
    ///
    /// The library is arbitrary native code that will run in this process. A
    /// caller must have established that the path is one it trusts.
    pub unsafe fn load(path: impl AsRef<Path>) -> Result<Loaded, CallError> {
        let path = path.as_ref().to_path_buf();
        // SAFETY: the caller vouches for the library.
        let library = unsafe { libloading::Library::new(&path) }
            .map_err(|err| CallError::Load(format!("{}: {err}", path.display())))?;
        // SAFETY: the symbol is looked up by the frozen name, and the signature
        // is the frozen one.
        let entry = unsafe {
            library
                .get::<MetisPluginEntryV1Fn>(METIS_PLUGIN_ENTRY_V1)
                .map_err(|err| CallError::Load(format!("{}: {err}", path.display())))?
        };
        // SAFETY: the entry point returns the plugin's table, which the plugin
        // keeps alive for at least as long as the library is loaded.
        let table = unsafe { entry() };
        // SAFETY: the table is the plugin's own, and the gate reads only what
        // its reported size covers.
        let read = unsafe { metis_read_table(table) }?;
        Ok(Loaded {
            table,
            _keep_mapped: library,
            path,
            abi: (read.abi_major, read.abi_minor),
        })
    }

    /// The ABI version the table reports.
    pub fn abi(&self) -> (u32, u32) {
        self.abi
    }

    /// The path this library was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The table the library exports.
    pub fn table(&self) -> &MetisPluginTableV1 {
        // SAFETY: the table is live for as long as the library is.
        unsafe { &*self.table }
    }

    /// Whether the plugin's table covers the service-completion entry, which
    /// presence is decided by `size`.
    pub fn has_service_complete(&self) -> bool {
        metis_plugin_table_has_service_complete(self.table().size)
    }

    /// Creates one instance and reads its capability declaration.
    ///
    /// The declaration is read once, here, so routing and schema validation
    /// never need a second call into the plugin.
    pub fn create_instance(self: &Arc<Self>, host: &Host) -> Result<Arc<Instance>, CallError> {
        host.bind_release(self.table().release_buffer);
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: the function pointer is the table's own, and the host table
        // lives for as long as the host does.
        let status = unsafe { (self.table().create_instance)(host.api(), &mut handle) };
        if status != MetisStatus::Ok || handle.is_null() {
            return Err(CallError::Create(status));
        }
        let capabilities = read_capabilities(self.table(), handle)?;
        Ok(Arc::new(Instance {
            loaded: Arc::clone(self),
            handle,
            host: host.clone(),
            capabilities,
            quarantined: AtomicBool::new(false),
            in_flight: Mutex::new(HashSet::new()),
        }))
    }
}

/// Reads and parses the instance's capability declaration.
fn read_capabilities(
    table: &MetisPluginTableV1,
    instance: *mut c_void,
) -> Result<Vec<CapabilityDescriptor>, CallError> {
    let mut payload = MetisBuffer::empty();
    // SAFETY: the instance is live and the out parameter is a local.
    let status = unsafe { (table.list_capabilities)(instance, &mut payload) };
    if status != MetisStatus::Ok {
        return Err(CallError::Status(status));
    }
    if payload.ptr.is_null() {
        return Err(CallError::Payload(
            "the capability list has no bytes".to_string(),
        ));
    }
    // SAFETY: the buffer is a plugin allocation, valid until it is released.
    let bytes = unsafe { slice::from_raw_parts(payload.ptr, payload.len) }.to_vec();
    // SAFETY: the table is the one that allocated the buffer.
    let released = unsafe { (table.release_buffer)(payload) };
    if released != MetisStatus::Ok {
        return Err(CallError::Status(released));
    }
    let body = metis_payload_body(&bytes).map_err(CallError::Status)?;
    registry::parse_capabilities(body)
}

// --- the host -------------------------------------------------------------

/// What the host recorded while a plugin was running.
#[derive(Default)]
struct HostInner {
    logs: Mutex<Vec<String>>,
    events: Mutex<Vec<String>>,
    services: Mutex<Vec<PendingService>>,
    /// The ordering ledger: every step unload safety depends on, in order.
    order: Mutex<Vec<String>>,
    /// The plugin's `release_buffer`, which is how a host frees a plugin-owned
    /// buffer. It is bound when an instance is created from a library.
    release_buffer: Mutex<Option<MetisReleaseBufferFn>>,
    /// One owning waker reference per poll the host ran and one per clone the
    /// plugin took, against one for every reference that was consumed by
    /// `wake` or `drop`. The two are equal whenever the host holds no reference
    /// and the plugin holds no clone, which is what makes a leaked reference
    /// visible instead of silent.
    wakers_handed_out: AtomicUsize,
    wakers_released: AtomicUsize,
    /// How many wakes arrived after a future was dropped, which is a counted
    /// violation rather than an ordering-ledger entry.
    wakes_after_drop: AtomicUsize,
}

/// One service the plugin asked for and the host has not answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingService {
    /// The service name, such as `timer`.
    pub service: String,
    /// The request id the answer has to carry back.
    pub request_id: u64,
    /// The request payload, envelope stripped.
    pub body: Vec<u8>,
}

/// The host side of the callback table.
///
/// It is cheap to clone and shared: every instance holds one, and every callback
/// it installs catches its own panic, because a callback is an `extern "C"`
/// frame the plugin calls into and only that frame can catch a panic raised
/// inside it.
#[derive(Clone)]
pub struct Host {
    inner: Arc<HostInner>,
    api: Arc<HostTable>,
}

/// The host table as a value with a stable address.
///
/// The plugin holds this pointer for the instance's lifetime and reads it from
/// whatever thread a poll runs on, which is why the value is boxed once and
/// never mutated.
struct HostTable(MetisHostApiV1);

// SAFETY: the table is built once, never mutated, and the callbacks behind it
// are `extern "C"` entries that only touch `HostInner`, whose state is behind
// mutexes and atomics. The boundary's contract is that a host table is valid
// and immutable for the instance's lifetime and reachable from any thread.
unsafe impl Send for HostTable {}
unsafe impl Sync for HostTable {}

impl Host {
    /// Builds a host with the full callback set installed.
    pub fn new() -> Host {
        let inner = Arc::new(HostInner::default());
        let mut api = Box::new(MetisHostApiV1 {
            size: std::mem::size_of::<MetisHostApiV1>(),
            user_data: std::ptr::null_mut(),
            progress: host_progress,
            emit_event: host_emit_event,
            log: host_log,
            service_request: host_service_request,
        });
        // The `Arc` is what the callbacks dereference. It is turned into a raw
        // pointer here and rebuilt in every callback, which is the standard way
        // to hand a Rust value to C as `user_data`.
        api.user_data = Arc::into_raw(Arc::clone(&inner)) as *mut c_void;
        Host {
            inner,
            api: Arc::new(HostTable(*api)),
        }
    }

    /// The host table to hand to `create_instance` and `invoke_future`.
    pub fn api(&self) -> *const MetisHostApiV1 {
        &self.api.0
    }

    /// Binds the plugin's buffer release entry, so a callback can free a
    /// plugin-owned buffer it received.
    ///
    /// One host serves one library: a second, different binding is refused
    /// loudly rather than silently overwriting the first, because a host that
    /// freed a buffer through another plugin's allocator would corrupt both.
    pub fn bind_release(&self, release: MetisReleaseBufferFn) {
        let mut bound = self.inner.release_buffer.lock().unwrap();
        match *bound {
            Some(existing) if !std::ptr::fn_addr_eq(existing, release) => {
                eprintln!("[runtime] refusing to rebind release_buffer to a second library");
            }
            _ => *bound = Some(release),
        }
    }

    /// Every log line the plugin has sent, in order.
    pub fn logs(&self) -> Vec<String> {
        self.inner.logs.lock().unwrap().clone()
    }

    /// Every event topic the plugin has emitted, in order.
    pub fn events(&self) -> Vec<String> {
        self.inner.events.lock().unwrap().clone()
    }

    /// The services the plugin has asked for and not been answered.
    pub fn pending_services(&self) -> Vec<PendingService> {
        self.inner.services.lock().unwrap().clone()
    }

    /// Removes and returns one pending service, which is how a host takes a
    /// request it is about to answer.
    pub fn take_service(&self, request_id: u64) -> Option<PendingService> {
        let mut services = self.inner.services.lock().unwrap();
        let index = services
            .iter()
            .position(|service| service.request_id == request_id)?;
        Some(services.remove(index))
    }

    /// The ordering ledger: the steps unload safety depends on.
    pub fn ordering(&self) -> Vec<String> {
        self.inner.order.lock().unwrap().clone()
    }

    /// How many owning waker references the host handed out, how many were
    /// released, and how many wakes arrived after a future was dropped.
    ///
    /// The host hands out one reference for every poll it runs and one for
    /// every clone the plugin takes, and a release is counted whenever one of
    /// those references is consumed by `wake` or `drop`. The first two numbers
    /// are equal whenever nothing is outstanding, so a call that leaks a
    /// reference shows up here as an imbalance rather than as silence.
    pub fn waker_counts(&self) -> (usize, usize, usize) {
        (
            self.inner.wakers_handed_out.load(SeqCst),
            self.inner.wakers_released.load(SeqCst),
            self.inner.wakes_after_drop.load(SeqCst),
        )
    }

    pub(crate) fn order(&self, event: impl Into<String>) {
        self.inner.order.lock().unwrap().push(event.into());
    }
}

impl Default for Host {
    fn default() -> Self {
        Host::new()
    }
}

/// Frees a plugin-owned buffer through the plugin's own entry.
fn release_plugin_buffer(inner: &HostInner, buffer: MetisBuffer) -> Vec<u8> {
    if buffer.ptr.is_null() {
        return Vec::new();
    }
    // SAFETY: the buffer is a plugin allocation, valid until it is released.
    let bytes = unsafe { slice::from_raw_parts(buffer.ptr, buffer.len) }.to_vec();
    let release = *inner.release_buffer.lock().unwrap();
    if let Some(release) = release {
        // SAFETY: the entry is the table's own, and the buffer is the one the
        // plugin just handed over.
        let _ = unsafe { release(buffer) };
    }
    bytes
}

/// The `Arc<HostInner>` behind a `user_data` pointer.
///
/// # Safety
///
/// `user_data` must be a pointer this module built with `Arc::into_raw`, and the
/// reference must not outlive the `Arc` it came from.
unsafe fn host_inner<'a>(user_data: *mut c_void) -> &'a HostInner {
    // SAFETY: the caller guarantees the pointer came from `Arc::into_raw` on an
    // `Arc<HostInner>` that is still alive.
    unsafe { &*(user_data as *const HostInner) }
}

// --- the instance ---------------------------------------------------------

/// One loaded, configured occurrence of a plugin.
pub struct Instance {
    loaded: Arc<Loaded>,
    handle: *mut c_void,
    host: Host,
    capabilities: Vec<CapabilityDescriptor>,
    quarantined: AtomicBool,
    /// The request ids this instance still holds a future for. Nothing may be
    /// unloaded while this is non-empty.
    in_flight: Mutex<HashSet<u64>>,
}

// SAFETY: everything the instance touches is the loaded table, whose entries
// the ABI allows from any thread, and host state that is behind mutexes and
// atomics. The contract's single-poller rule is enforced by the host, not by
// Rust's auto traits, which is why the instance is declared thread-safe rather
// than inferred.
unsafe impl Send for Instance {}
unsafe impl Sync for Instance {}

impl Instance {
    /// The capability declaration this instance published.
    pub fn capabilities(&self) -> &[CapabilityDescriptor] {
        &self.capabilities
    }

    /// The host that serves this instance.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The library this instance came from.
    pub fn loaded(&self) -> &Arc<Loaded> {
        &self.loaded
    }

    /// Whether the instance is quarantined.
    ///
    /// A quarantined instance accepts no new future: a poll it abandoned is
    /// still running, and starting another would put two polls inside one
    /// plugin.
    pub fn is_quarantined(&self) -> bool {
        self.quarantined.load(SeqCst)
    }

    /// Quarantines the instance. Its library stays resident.
    pub fn quarantine(&self, reason: &str) {
        if !self.quarantined.swap(true, SeqCst) {
            self.host.order(format!("instance quarantined: {reason}"));
        }
    }

    /// The request ids this instance still holds a future for.
    pub fn in_flight(&self) -> HashSet<u64> {
        self.in_flight.lock().unwrap().clone()
    }

    /// Starts one call and returns the future the host polls.
    ///
    /// Nothing enters the plugin here beyond `invoke_future` itself: the
    /// returned future is polled by the caller's runtime.
    pub fn invoke(
        self: &Arc<Self>,
        capability: &str,
        request_id: u64,
        body: &[u8],
    ) -> Result<PluginCall, CallError> {
        if self.is_quarantined() {
            return Err(CallError::Quarantined);
        }
        if !self
            .capabilities
            .iter()
            .any(|declared| declared.name == capability)
        {
            return Err(CallError::UnknownCapability(capability.to_string()));
        }
        let encoded = metis_encode_payload(body);
        let mut handle = MetisFutureHandle::null();
        // SAFETY: the instance is live, the request buffer is host-owned and
        // valid for this call, and the out parameter is a local.
        let status = unsafe {
            (self.loaded.table().invoke_future)(
                self.handle,
                capability.as_ptr(),
                capability.len(),
                request_id,
                MetisBuffer {
                    ptr: encoded.as_ptr() as *mut u8,
                    len: encoded.len(),
                    cap: encoded.len(),
                    owner: METIS_BUFFER_OWNER_HOST,
                    reserved: 0,
                },
                self.host.api(),
                &mut handle,
            )
        };
        if status != MetisStatus::Ok {
            return Err(CallError::Status(status));
        }
        self.in_flight.lock().unwrap().insert(request_id);
        self.host
            .order(format!("call {request_id} started on {capability}"));
        Ok(PluginCall {
            instance: Arc::clone(self),
            handle,
            request_id,
            state: Arc::new(CallState {
                waker: Mutex::new(None),
                dropped: AtomicBool::new(false),
                counts: Arc::clone(&self.host.inner),
            }),
        })
    }

    /// Marks a call's cancellation token. The plugin's future is expected to
    /// resolve as `Cancelled` on its next poll.
    pub fn cancel(&self, request_id: u64) -> Result<(), CallError> {
        // SAFETY: the instance is live and the function pointer is its own.
        let status = unsafe { (self.loaded.table().cancel)(self.handle, request_id) };
        if status == MetisStatus::Ok {
            Ok(())
        } else {
            Err(CallError::Status(status))
        }
    }

    /// Hands the plugin the host's answer to a service request.
    ///
    /// The answer payload is host-owned and borrowed for the call, so the host
    /// frees it when this returns.
    pub fn complete_service(
        &self,
        request_id: u64,
        status: MetisStatus,
        body: &[u8],
    ) -> Result<(), CallError> {
        let encoded = if status == MetisStatus::Ok {
            metis_encode_payload(body)
        } else {
            Vec::new()
        };
        let buffer = if encoded.is_empty() {
            MetisBuffer::empty()
        } else {
            MetisBuffer {
                ptr: encoded.as_ptr() as *mut u8,
                len: encoded.len(),
                cap: encoded.len(),
                owner: METIS_BUFFER_OWNER_HOST,
                reserved: 0,
            }
        };
        // SAFETY: the instance is live, and the buffer is host-owned and valid
        // for this call.
        let answered = unsafe {
            (self.loaded.table().service_complete)(self.handle, request_id, status, buffer)
        };
        if answered == MetisStatus::Ok {
            Ok(())
        } else {
            Err(CallError::Status(answered))
        }
    }

    /// Quiesces the instance, which is only legal once every future it handed
    /// out has been dropped.
    pub fn shutdown(&self) -> Result<(), CallError> {
        if !self.in_flight.lock().unwrap().is_empty() {
            self.host
                .order("shutdown refused: futures are still in flight");
            return Err(CallError::Payload(
                "futures are still in flight, so the instance cannot quiesce".to_string(),
            ));
        }
        // SAFETY: the instance is live and the function pointer is its own.
        let status = unsafe { (self.loaded.table().shutdown)(self.handle) };
        if status == MetisStatus::Ok {
            self.host
                .order("instance quiesced after every future was dropped");
            Ok(())
        } else {
            Err(CallError::Status(status))
        }
    }

    /// Releases the instance, then the library.
    ///
    /// Unload happens here and only here, and it is refused unless every future
    /// is gone and this `Arc` is the last one, which is what proves nothing is
    /// inside plugin code and no waker clone is outstanding. The refusals are
    /// recorded in the ordering ledger rather than inferred.
    pub fn unload(self: Arc<Self>) -> Result<(), CallError> {
        if !self.in_flight.lock().unwrap().is_empty() {
            self.host
                .order("unload refused: futures are still in flight");
            return Err(CallError::Payload(
                "futures are still in flight, so nothing may be unloaded".to_string(),
            ));
        }
        if Arc::strong_count(&self) != 1 {
            self.host
                .order("unload refused: a call still holds the instance");
            return Err(CallError::Payload(
                "a call still holds the instance".to_string(),
            ));
        }
        // SAFETY: the instance came from `create_instance`, quiescence has
        // happened, and every future and waker clone is gone, so no plugin code
        // is running.
        let status = unsafe { (self.loaded.table().destroy_instance)(self.handle) };
        self.host.order("instance destroyed");
        if status != MetisStatus::Ok {
            return Err(CallError::Status(status));
        }
        Ok(())
    }
}

// --- the call future ------------------------------------------------------

/// What one call's host-side state carries.
///
/// The counters live on the host, not here, because they are what the host
/// reports after a run: a waker clone belongs to the host that made it.
struct CallState {
    /// The waker of the task polling this call, set on every poll.
    waker: Mutex<Option<Waker>>,
    /// Whether the plugin's future has been dropped.
    dropped: AtomicBool,
    counts: Arc<HostInner>,
}

/// One call in flight: the plugin's future, driven by the caller's runtime.
///
/// Dropping this drops the plugin's future exactly once, which is what returns
/// the plugin's context box and every waker clone it holds.
pub struct PluginCall {
    instance: Arc<Instance>,
    handle: MetisFutureHandle,
    request_id: u64,
    state: Arc<CallState>,
}

impl PluginCall {
    /// The request id this call was started with.
    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    /// Copies and releases a plugin-owned ready buffer.
    fn take_buffer(&self, buffer: MetisBuffer) -> Result<Vec<u8>, CallError> {
        if buffer.ptr.is_null() {
            return Ok(Vec::new());
        }
        // SAFETY: the buffer is a plugin allocation, valid until released.
        let bytes = unsafe { slice::from_raw_parts(buffer.ptr, buffer.len) }.to_vec();
        // SAFETY: the table is the one that allocated it.
        let status = unsafe { (self.instance.loaded.table().release_buffer)(buffer) };
        if status != MetisStatus::Ok {
            return Err(CallError::Status(status));
        }
        Ok(metis_payload_body(&bytes)
            .map_err(CallError::Status)?
            .to_vec())
    }

    fn host_waker(&self) -> MetisWaker {
        // One owning reference is handed to the plugin for this poll, and the
        // host releases it as soon as the poll returns.
        let data = Arc::into_raw(Arc::clone(&self.state)) as *mut c_void;
        self.state.counts.wakers_handed_out.fetch_add(1, SeqCst);
        MetisWaker {
            data,
            vtable: &WAKER_VTABLE,
        }
    }
}

impl Future for PluginCall {
    type Output = Result<Vec<u8>, CallError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.state.dropped.load(SeqCst) {
            return Poll::Ready(Err(CallError::Payload(
                "a dropped call was polled again".to_string(),
            )));
        }
        let waker = this.host_waker();
        let (waker_data, waker_vtable) = (waker.data, waker.vtable);
        let mut out = MetisBuffer::empty();
        // SAFETY: the handle came from this table's `invoke_future`, and the
        // waker is this call's own.
        let status = unsafe {
            let vtable = &*this.handle.vtable;
            (vtable.poll)(this.handle.ctx, waker, &mut out)
        };
        // The reference handed to the poll is the host's own, so it is released
        // as soon as the poll returns, whatever it returned: a poll that parks,
        // fails or panics cannot leak it. A clone the plugin took is the
        // plugin's to release.
        release_waker(MetisWaker {
            data: waker_data,
            vtable: waker_vtable,
        });
        match status {
            MetisStatus::Pending => {
                *this.state.waker.lock().unwrap() = Some(context.waker().clone());
                Poll::Pending
            }
            MetisStatus::Ok => Poll::Ready(this.take_buffer(out)),
            terminal => Poll::Ready(Err(CallError::Status(terminal))),
        }
    }
}

impl Drop for PluginCall {
    fn drop(&mut self) {
        if self.state.dropped.swap(true, SeqCst) {
            return;
        }
        // SAFETY: the handle came from this table's `invoke_future` and this is
        // its one drop.
        let status = unsafe {
            let vtable = &*self.handle.vtable;
            (vtable.drop)(self.handle.ctx)
        };
        self.instance
            .in_flight
            .lock()
            .unwrap()
            .remove(&self.request_id);
        self.instance.host.order(format!(
            "call {} dropped, context reclaimed (drop returned {})",
            self.request_id,
            status.as_str()
        ));
    }
}

// --- the host waker -------------------------------------------------------

unsafe extern "C" fn waker_clone(waker: *const MetisWaker) -> MetisWaker {
    // SAFETY: the waker's data is an `Arc<CallState>` this crate built, and the
    // handle is borrowed for the poll.
    let data = unsafe { (*waker).data };
    // SAFETY: taking the borrowed reference into an `Arc` and putting it back
    // leaves the count unchanged.
    let borrowed = unsafe { Arc::from_raw(data as *const CallState) };
    let cloned = Arc::clone(&borrowed);
    let _ = Arc::into_raw(borrowed);
    cloned.counts.wakers_handed_out.fetch_add(1, SeqCst);
    // The clone becomes the caller's owning reference.
    let _ = Arc::into_raw(cloned);
    MetisWaker {
        data,
        vtable: &WAKER_VTABLE,
    }
}

unsafe extern "C" fn waker_wake(waker: MetisWaker) {
    // SAFETY: the data is an `Arc<CallState>` this crate built, and this
    // consumes one owning reference.
    let state = unsafe { Arc::from_raw(waker.data as *const CallState) };
    state.counts.wakers_released.fetch_add(1, SeqCst);
    if state.dropped.load(SeqCst) {
        // A wake may arrive after the future was dropped. It is recorded rather
        // than dereferenced into anything that no longer exists.
        state.counts.wakes_after_drop.fetch_add(1, SeqCst);
        return;
    }
    if let Some(waker) = state.waker.lock().unwrap().take() {
        waker.wake();
    }
}

unsafe extern "C" fn waker_wake_by_ref(waker: *const MetisWaker) {
    // SAFETY: the data is a live `Arc<CallState>` and stays borrowed.
    let data = unsafe { (*waker).data };
    let state = unsafe { &*(data as *const CallState) };
    if state.dropped.load(SeqCst) {
        state.counts.wakes_after_drop.fetch_add(1, SeqCst);
        return;
    }
    if let Some(waker) = state.waker.lock().unwrap().take() {
        waker.wake();
    }
}

/// Releases one owning waker reference without waking it.
///
/// It is the body of the waker's `drop` entry, and it is also what the host
/// does with the one reference it hands to each poll once that poll returns:
/// both consume exactly one owning reference.
fn release_waker(waker: MetisWaker) {
    if waker.vtable.is_null() {
        return;
    }
    // SAFETY: this consumes one owning reference, as the `drop` entry is
    // defined to.
    let state = unsafe { Arc::from_raw(waker.data as *const CallState) };
    state.counts.wakers_released.fetch_add(1, SeqCst);
}

unsafe extern "C" fn waker_drop(waker: MetisWaker) {
    release_waker(waker);
}

static WAKER_VTABLE: MetisWakerVtable = MetisWakerVtable {
    clone: waker_clone,
    wake: waker_wake,
    wake_by_ref: waker_wake_by_ref,
    drop: waker_drop,
};

// --- host callbacks -------------------------------------------------------
//
// Every callback is an `extern "C"` frame reached from plugin code, and each one
// catches its own panic: only the callback's own frame can.

unsafe extern "C" fn host_progress(user_data: *mut c_void, request_id: u64, payload: MetisBuffer) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user_data` is this host's own pointer.
        let inner = unsafe { host_inner(user_data) };
        let body = release_plugin_buffer(inner, payload);
        println!(
            "[runtime] progress for request {request_id}: {} bytes",
            body.len()
        );
    }));
}

unsafe extern "C" fn host_emit_event(
    user_data: *mut c_void,
    topic: *const u8,
    topic_len: usize,
    payload: MetisBuffer,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user_data` is this host's own pointer, and the topic is
        // borrowed for the call.
        let inner = unsafe { host_inner(user_data) };
        let topic = unsafe { slice::from_raw_parts(topic, topic_len) };
        let _ = release_plugin_buffer(inner, payload);
        inner
            .events
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(topic).to_string());
    }));
}

unsafe extern "C" fn host_log(
    user_data: *mut c_void,
    level: i32,
    message: *const u8,
    message_len: usize,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user_data` is this host's own pointer, and the message is
        // borrowed for the call.
        let inner = unsafe { host_inner(user_data) };
        let message = unsafe { slice::from_raw_parts(message, message_len) };
        inner
            .logs
            .lock()
            .unwrap()
            .push(format!("[{level}] {}", String::from_utf8_lossy(message)));
    }));
}

unsafe extern "C" fn host_service_request(
    user_data: *mut c_void,
    service: *const u8,
    service_len: usize,
    request_id: u64,
    payload: MetisBuffer,
) -> MetisStatus {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user_data` is this host's own pointer, and the service name
        // is borrowed for the call.
        let inner = unsafe { host_inner(user_data) };
        let service = unsafe { slice::from_raw_parts(service, service_len) };
        let service = String::from_utf8_lossy(service).to_string();
        // The payload is plugin-owned and released here, from inside the
        // callback, which is the reentrancy the contract allows.
        let body = release_plugin_buffer(inner, payload);
        if service != "timer" {
            return MetisStatus::NotFound;
        }
        inner.services.lock().unwrap().push(PendingService {
            service,
            request_id,
            body,
        });
        MetisStatus::Ok
    }));
    caught.unwrap_or(MetisStatus::Internal)
}
