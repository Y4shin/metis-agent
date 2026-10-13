//! PROTOTYPE (throwaway). Not product code.
//!
//! The primitives-only half of the registry crate, and the seam the registry
//! routes through.
//!
//! Everything here compiles with `--no-default-features`, which is the point:
//! no serde, no JSON Schema, no validator, no libloading. The registry proper,
//! which is serde and JSON shaped end to end, is behind the `registry` feature
//! in [`registry`], exactly the way `plugin-contract` keeps `authoring` off by
//! default.
//!
//! What lives here and not in `plugin-contract`:
//!
//! - the object safe `Plugin` seam (`Box<dyn Plugin>`, `BoxFuture` returns, no
//!   associated consts or types), which round one established;
//! - the error taxonomy the host needs and the ABI's frozen status space is too
//!   small to carry: distinct variants for a caller payload that violates a
//!   schema, a capability refused by version gating, a capability that was
//!   never registered, a response that violates the plugin's own published
//!   schema, and a plugin-side failure with its own status;
//! - capability descriptors that reference their schemas *by id*, so the
//!   registry can fetch them separately rather than embedding them in the
//!   listing.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::Duration;

#[cfg(feature = "registry")]
mod registry;
#[cfg(feature = "registry")]
pub use registry::{
    PluginInfo, PluginLoader, PluginRegistry, RefusedCapability, RegistryError, RoutingMode,
};

/// The boxed future every seam method returns.
///
/// This type is the object safety price: an `async fn` or `-> impl Future`
/// method cannot be in a `dyn` compatible trait, and a non-generic trait method
/// cannot be generic over the output either, so every async seam method returns
/// a boxed, `Send`, lifetime carrying future instead.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The status a plugin reports, mirrored from the ABI status space minus `Ok`
/// and `Pending`.
///
/// Deliberately ABI independent: the primitives build carries no `plugin-contract`
/// dependency, so a plugin-side failure is expressed here and the adapter maps
/// the ABI status onto it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    InvalidArgument,
    NotFound,
    PermissionDenied,
    Cancelled,
    Internal,
    Unsupported,
}

impl Status {
    pub const fn as_str(self) -> &'static str {
        match self {
            Status::InvalidArgument => "invalid_argument",
            Status::NotFound => "not_found",
            Status::PermissionDenied => "permission_denied",
            Status::Cancelled => "cancelled",
            Status::Internal => "internal",
            Status::Unsupported => "unsupported",
        }
    }
}

/// The error the seam and the registry report.
///
/// Every failure path the prototype exercises has its own variant, so a caller
/// can branch on the variant rather than on a message string. Round one's
/// listing had only the six ABI statuses plus a string, which is why three of
/// these paths were indistinguishable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    /// The caller's request payload violates the capability's published schema.
    CallerPayloadInvalid { capability: String, detail: String },
    /// No enabled plugin registered this capability.
    CapabilityNotFound { capability: String },
    /// The capability exists but version gating refused it (never registered).
    VersionGated {
        capability: String,
        advertised: u32,
        supported: u32,
    },
    /// The plugin's response violates the plugin's own published schema.
    ResponseViolation {
        plugin: String,
        capability: String,
        detail: String,
    },
    /// The plugin failed the call with its own status.
    PluginFailed {
        status: Status,
        capability: String,
        detail: String,
    },
    /// The plugin advertised a capability it does not implement.
    Unsupported { capability: String, detail: String },
    /// The caller cancelled the call, or the plugin gave up on a cancelled call.
    Cancelled,
    /// Capability discovery failed.
    Discovery { plugin: String, detail: String },
    /// A schema id the plugin advertised could not be resolved.
    SchemaMissing {
        plugin: String,
        schema_id: String,
        detail: String,
    },
    /// A plugin stayed loaded but its capabilities are not routable.
    Disabled { plugin: String, capability: String },
    /// Something failed inside the host registry itself.
    Internal { detail: String },
}

impl PluginError {
    /// The variant name, so harness output and logs are machine checkable
    /// without parsing a message.
    pub const fn variant(&self) -> &'static str {
        match self {
            PluginError::CallerPayloadInvalid { .. } => "CallerPayloadInvalid",
            PluginError::CapabilityNotFound { .. } => "CapabilityNotFound",
            PluginError::VersionGated { .. } => "VersionGated",
            PluginError::ResponseViolation { .. } => "ResponseViolation",
            PluginError::PluginFailed { .. } => "PluginFailed",
            PluginError::Unsupported { .. } => "Unsupported",
            PluginError::Cancelled => "Cancelled",
            PluginError::Discovery { .. } => "Discovery",
            PluginError::SchemaMissing { .. } => "SchemaMissing",
            PluginError::Disabled { .. } => "Disabled",
            PluginError::Internal { .. } => "Internal",
        }
    }
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginError::CallerPayloadInvalid { capability, detail } => {
                write!(f, "invalid argument: capability {capability}: the request violates its published schema: {detail}")
            }
            PluginError::CapabilityNotFound { capability } => {
                write!(f, "not found: no enabled capability named {capability} is registered")
            }
            PluginError::VersionGated {
                capability,
                advertised,
                supported,
            } => write!(
                f,
                "unsupported: capability {capability} advertises version {advertised}, this host supports {supported}"
            ),
            PluginError::ResponseViolation {
                plugin,
                capability,
                detail,
            } => write!(
                f,
                "internal: plugin {plugin} returned a response for capability {capability} that violates its published schema: {detail}"
            ),
            PluginError::PluginFailed {
                status,
                capability,
                detail,
            } => write!(
                f,
                "the plugin reported {} for capability {capability}: {detail}",
                status.as_str()
            ),
            PluginError::Unsupported { capability, detail } => {
                write!(f, "unsupported: capability {capability}: {detail}")
            }
            PluginError::Cancelled => write!(f, "cancelled"),
            PluginError::Discovery { plugin, detail } => {
                write!(f, "the capability listing for {plugin} failed: {detail}")
            }
            PluginError::SchemaMissing {
                plugin,
                schema_id,
                detail,
            } => write!(f, "plugin {plugin} could not resolve schema {schema_id}: {detail}"),
            PluginError::Disabled { plugin, capability } => {
                write!(f, "plugin {plugin} is disabled, so {capability} is not routable")
            }
            PluginError::Internal { detail } => write!(f, "internal: {detail}"),
        }
    }
}

impl std::error::Error for PluginError {}

/// What a plugin calls itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    /// The plugin id. Globally unique under the registry's chosen name rule.
    pub id: String,
    /// The plugin's own version string, not the ABI version.
    pub version: String,
    /// The ABI major the plugin targets, which must match the host's.
    pub abi_major: u32,
}

/// V1 capability kinds, the same two the landed descriptor carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "registry", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "registry", serde(rename_all = "snake_case"))]
pub enum CapabilityKind {
    Tool,
    Provider,
}

/// One capability descriptor.
///
/// The schemas are referenced by id, not embedded. That is the round two
/// mitigation for the descriptor size finding: the listing carries names and
/// ids, and the schema documents travel in a separate batched fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "registry", derive(serde::Serialize, serde::Deserialize))]
pub struct CapabilityDescriptor {
    pub name: String,
    pub kind: CapabilityKind,
    pub version: u32,
    pub request_schema_id: String,
    pub response_schema_id: String,
}

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

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// The per request context: id and cancellation flag.
pub struct Call {
    id: u64,
    cancelled: Arc<AtomicBool>,
}

static NEXT_CALL_ID: AtomicU64 = AtomicU64::new(1);

impl Default for Call {
    fn default() -> Self {
        Call::new()
    }
}

impl Call {
    /// A call with a freshly allocated, process-wide unique id.
    pub fn new() -> Self {
        Call::with_id(NEXT_CALL_ID.fetch_add(1, Ordering::SeqCst))
    }

    /// A call with an explicit id, for a harness that wants to name one.
    pub fn with_id(id: u64) -> Self {
        // Keep the allocator ahead of any explicit id so the two never collide.
        NEXT_CALL_ID.fetch_max(id + 1, Ordering::SeqCst);
        Call {
            id,
            cancelled: Arc::new(AtomicBool::new(false)),
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

/// What the registry holds and routes through.
///
/// Every method takes `&self` and returns a concrete type, no associated items,
/// no generics, no `impl Trait`. Those are the object safety requirements round
/// one established.
pub trait Plugin: Send + Sync + 'static {
    /// The plugin's own id, version, and targeted ABI major.
    fn manifest(&self) -> PluginManifest;

    /// The capabilities this plugin publishes.
    ///
    /// Fallible, because listing capabilities is a real call that can fail
    /// (round one's issue 1: the old shape registered a plugin with zero
    /// capabilities on a failed listing).
    fn capabilities(&self) -> Result<Vec<CapabilityDescriptor>, PluginError>;

    /// Resolves schema ids to their JSON Schema documents.
    ///
    /// The registry batches every id it discovered into one call, so the
    /// descriptor listing stays small and the schema documents travel once.
    fn schemas(&self, ids: &[String]) -> Result<BTreeMap<String, Vec<u8>>, PluginError>;

    /// Delivers this plugin's configuration before any capability is routed.
    ///
    /// Default: the plugin has nothing to configure.
    fn configure(&self, _configuration: &[u8]) -> Result<(), PluginError> {
        Ok(())
    }

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

/// Which capability versions this host supports.
#[derive(Debug, Clone)]
pub struct VersionPolicy {
    pub default_max_version: u32,
    pub max_versions: HashMap<String, u32>,
}

impl VersionPolicy {
    pub fn new(default_max_version: u32) -> Self {
        VersionPolicy {
            default_max_version,
            max_versions: HashMap::new(),
        }
    }

    pub fn max_version(&self, capability: &str) -> u32 {
        self.max_versions
            .get(capability)
            .copied()
            .unwrap_or(self.default_max_version)
    }
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
/// The prototype carries no async runtime; this is a one-task park/unpark loop,
/// which is enough because the ABI side completes on a plugin worker thread and
/// wakes this one through the slot's waker.
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

// --- the executor seam ------------------------------------------------------
//
// The plugin owns the executor that drives its capability futures, and the SDK
// injects the concrete implementation. The primitives crate defines only the
// seam, because it must stay runtime-free: no runtime, no thread, and no pool
// is named here. The plugin holds an `Arc<dyn Executor>`, so a thread-per-call
// stand-in and a bounded pool are interchangeable behind it, and both can be
// measured in one binary. The host-polled future seam does not use this at all:
// its futures are driven by the host.

/// What an executor runs: one capability's work after `invoke` returned
/// `Pending`.
///
/// A boxed future rather than a generic `spawn<F: Future>`, because the seam
/// must be object safe: the plugin holds an `Arc<dyn Executor>` and does not
/// know whether the SDK handed it a pool, a runtime, or a thread.
pub type Task = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Why an executor refused a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnRefused {
    /// `drain` has started, so no new work is accepted.
    Draining,
}

/// What a `drain` achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainOutcome {
    /// Tasks that ran to completion.
    pub completed: usize,
    /// Tasks still queued when the deadline expired; their futures were
    /// dropped, which releases whatever plugin-owned buffers they held.
    pub dropped: usize,
    /// Tasks still running when the deadline expired. A deadline cannot
    /// preempt a thread already inside plugin code.
    pub still_running: usize,
    /// Whether the deadline expired with work left.
    pub timed_out: bool,
}

/// Drives the capability futures a plugin schedules.
///
/// Three methods and no runtime: this is the whole seam the SDK pool and the
/// thread-per-call fallback share.
pub trait Executor: Send + Sync + 'static {
    /// Schedules one task.
    ///
    /// Once [`drain`](Executor::drain) has started this refuses with
    /// [`SpawnRefused::Draining`], so the plugin can refuse a new call instead
    /// of accepting work it will never run.
    fn spawn(&self, task: Task) -> Result<(), SpawnRefused>;

    /// Stops accepting work, runs what is already queued, and waits for the
    /// running tasks. `deadline` bounds the wait; `None` waits indefinitely.
    fn drain(&self, deadline: Option<Duration>) -> DrainOutcome;

    /// Whether this executor has started draining.
    fn is_draining(&self) -> bool {
        false
    }
}

#[cfg(feature = "registry")]
mod json {
    //! The serde helpers `Payload` grows when the registry feature is on.

    use serde::Serialize;
    use serde::de::DeserializeOwned;

    use super::{Payload, PluginError};

    impl Payload {
        pub fn from_json<T: Serialize>(value: &T) -> Result<Self, PluginError> {
            serde_json::to_vec(value)
                .map(Payload)
                .map_err(|err| PluginError::Internal {
                    detail: format!("payload encode: {err}"),
                })
        }

        pub fn to_json<T: DeserializeOwned>(&self) -> Result<T, PluginError> {
            serde_json::from_slice(&self.0).map_err(|err| PluginError::Internal {
                detail: format!("payload does not decode as the requested type: {err}"),
            })
        }
    }
}
