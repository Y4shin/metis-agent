//! PROTOTYPE (throwaway). Not product code.
//!
//! ROUND ONE FILE, RETAINED BUT NO LONGER COMPILED. Round two moved the seam
//! into the `proto-registry` crate (see `FINDINGS2.md`); this file is kept on
//! disk as the record of what round one built. It is not a module of
//! `proto-host` any more.
//!
//! The one object safe seam the registry holds.
//!
//! Compared with the landed `authoring::Plugin` trait, four things are gone and
//! they are gone for one reason: `Box<dyn Plugin>` has to exist.
//!
//! - the four capability arms with their associated types and associated consts
//!   (`type Answer`, `const ANSWER`), which are what cap a plugin at four
//!   capabilities and are both dyn incompatible;
//! - the RPITIT return types (`impl Future`), which are dyn incompatible;
//! - any method generic over the request type, which is dyn incompatible;
//! - the `Default` bound, which the registry does not need because it receives
//!   constructed trait objects.
//!
//! What replaces the four arms is a runtime capability list plus a string
//! keyed `invoke`, which is exactly what the landed C ABI already does.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The boxed future every seam method returns.
///
/// This type is the object safety price: an `async fn` or `-> impl Future`
/// method cannot be in a `dyn` compatible trait, and a non-generic trait method
/// cannot be generic over the output either, so every async seam method returns
/// a boxed, `Send`, lifetime carrying future instead.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// V1 capability kinds, the same two the landed descriptor carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Tool,
    Provider,
}

/// One capability descriptor.
///
/// The landed descriptor's `schema: String` reference becomes two real JSON
/// Schema documents. That is the prototype's central assumption: the registry
/// needs the schemas themselves, not a pointer to them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub name: String,
    pub kind: CapabilityKind,
    pub version: u32,
    pub request_schema: serde_json::Value,
    pub response_schema: serde_json::Value,
}

/// What a plugin calls itself.
#[derive(Debug, Clone)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
}

/// The error a plugin or the registry reports.
///
/// The variants mirror the ABI's frozen status space minus `Ok` and `Pending`,
/// as the landed `PluginError` does. There is deliberately no variant that
/// carries "which plugin": the registry attributes a response violation by
/// writing the plugin's name into the message, because the error type has no
/// field for it and adding one would move the ABI status space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    InvalidArgument(String),
    NotFound(String),
    PermissionDenied(String),
    Unsupported(String),
    Internal(String),
    Cancelled,
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginError::InvalidArgument(detail) => write!(f, "invalid argument: {detail}"),
            PluginError::NotFound(detail) => write!(f, "not found: {detail}"),
            PluginError::PermissionDenied(detail) => write!(f, "permission denied: {detail}"),
            PluginError::Unsupported(detail) => write!(f, "unsupported: {detail}"),
            PluginError::Internal(detail) => write!(f, "internal: {detail}"),
            PluginError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for PluginError {}

/// A capability payload, as bytes.
///
/// The seam is bytes shaped on purpose: one `invoke` serves every capability,
/// so the payload type cannot depend on the capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload(Vec<u8>);

impl Payload {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Payload(bytes)
    }

    pub fn from_json<T: Serialize>(value: &T) -> Result<Self, PluginError> {
        serde_json::to_vec(value)
            .map(Payload)
            .map_err(|err| PluginError::InvalidArgument(format!("payload encode: {err}")))
    }

    pub fn to_json<T: DeserializeOwned>(&self) -> Result<T, PluginError> {
        serde_json::from_slice(&self.0).map_err(|err| {
            PluginError::InvalidArgument(format!("payload does not decode as the requested type: {err}"))
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// The per request context: id and cancellation flag.
///
/// Kept because the landed ABI's `cancel` arrives out of band and has to reach
/// the call, and because the registry needs something to key a cancel by.
pub struct Call {
    id: u64,
    cancelled: AtomicBool,
}

impl Call {
    pub fn new(id: u64) -> Self {
        Call {
            id,
            cancelled: AtomicBool::new(false),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn mark_cancelled(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// What the registry holds.
///
/// Every method takes `&self` and returns a concrete type, no associated items,
/// no generics, no `impl Trait`. Those are the object safety requirements this
/// prototype actually hit; see `FINDINGS.md`.
pub trait Plugin: Send + Sync + 'static {
    /// The plugin's own name and version.
    fn manifest(&self) -> PluginManifest;

    /// The capabilities this plugin publishes.
    ///
    /// This is the registry, and it is why the capability count is no longer a
    /// property of the trait.
    fn capabilities(&self) -> Vec<CapabilityDescriptor>;

    /// One generic call, addressed by capability name.
    fn invoke<'a>(
        &'a self,
        capability: &'a str,
        payload: Payload,
        call: &'a Call,
    ) -> BoxFuture<'a, Result<Payload, PluginError>>;

    /// Requests cancellation of a pending request.
    fn cancel(&self, request_id: u64) -> Result<(), PluginError>;

    /// Instance teardown.
    fn shutdown(&self) -> Result<(), PluginError>;
}

// --- the smallest executor that can drive a boxed future --------------------

struct ThreadWaker(thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Drives a future to completion on the calling thread.
///
/// The prototype carries no async runtime. Every future here resolves on its
/// first poll, because both adapters use the synchronous side of their call
/// path, so this is a one poll loop and not an executor.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}
