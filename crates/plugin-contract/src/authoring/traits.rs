//! The authoring trait and its error type.
//!
//! This module deliberately names no ABI type. Everything the ABI forced onto
//! the authoring surface is in `call` (the per-request context and the host
//! routes) and in the bounds and signatures below, and the status mapping
//! lives in `plugin_adapter`, on the ABI side of the line.

use std::fmt;
use std::future::Future;

use crate::authoring::call::{Call, Host};
use crate::authoring::capability::{CapabilityDescriptor, Shape};

/// The error a plugin reports.
///
/// The variants mirror the ABI's frozen status space (D7) minus `Ok` and
/// `Pending`: `Pending` is the adapter's business, not an author's, and the
/// remaining six are the failure space a plugin can express. The first five
/// carry a detail string for the host's log; `Cancelled` is control flow, not
/// detail, and it is the one variant with a protocol consequence:
/// [`PluginError::Cancelled`] returned after a successful cancel completes
/// nothing, while the same value returned without a successful cancel still
/// completes with the cancelled status, so a host is never parked on a
/// completion that will not arrive (D12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    /// The request did not make sense.
    InvalidArgument(String),
    /// The named thing does not exist here.
    NotFound(String),
    /// The caller is not allowed to do this.
    PermissionDenied(String),
    /// This build does not implement what was asked for.
    Unsupported(String),
    /// Something failed inside the plugin.
    Internal(String),
    /// The plugin gave up on this call.
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

/// What a plugin author implements.
///
/// Four capability arms, an instance lifecycle pair, and a cancellation method,
/// plus the capability declaration the adapter lists.
///
/// The arms are this trait's vocabulary, and each arm's capability name is the
/// plugin's to choose (`ANSWER` and friends). The declaration and the arm names
/// are two independent declarations on purpose: the adapter compares them at
/// instance creation, so a descriptor whose name no arm answers fails loudly
/// there instead of surfacing as `not_found` at call time. A plugin that does
/// not implement an arm names it with the empty string and leaves its method
/// answering [`PluginError::Unsupported`]; nothing advertises that name, so no
/// host calls it.
///
/// Two bounds are the ABI's, not the author's, and are documented rather than
/// hidden: the plugin is `Send + Sync + 'static` and every capability future is
/// `Send`, because the adapter moves each call onto a worker thread so `invoke`
/// can return `Pending` (D11, D15).
pub trait Plugin: Default + Send + Sync + 'static {
    /// The capability descriptors this plugin advertises (D9).
    ///
    /// The adapter returns exactly these from `list_capabilities`, and checks
    /// every name here against the arm names below at instance creation.
    fn capabilities(&self) -> Vec<CapabilityDescriptor>;

    /// The capability name of the `answer` arm, or `""` when this plugin does
    /// not implement it.
    const ANSWER: &'static str = "";
    /// The capability name of the `deferred` arm, or `""` when this plugin does
    /// not implement it.
    const DEFERRED: &'static str = "";
    /// The capability name of the `cancellable` arm, or `""` when this plugin
    /// does not implement it.
    const CANCELLABLE: &'static str = "";
    /// The capability name of the `fault` arm, or `""` when this plugin does
    /// not implement it.
    const FAULT: &'static str = "";

    /// The payload shape of the `answer` arm.
    type Answer: Shape;
    /// The payload shape of the `deferred` arm.
    type Deferred: Shape;
    /// The payload shape of the `cancellable` arm.
    type Cancellable: Shape;
    /// The payload shape of the `fault` arm.
    type Fault: Shape;

    /// Answers a call, and gets its result to the host through the adapter.
    ///
    /// The result arrives through the host's `complete` callback: the trait
    /// path is always pending (D15), even when this future is ready on its
    /// first poll.
    fn answer(
        &self,
        call: Call,
        request: <Self::Answer as Shape>::Request,
    ) -> impl Future<Output = Result<<Self::Answer as Shape>::Response, PluginError>> + Send {
        let _ = (call, request);
        async {
            Err(PluginError::Unsupported(
                "answer is not implemented".to_string(),
            ))
        }
    }

    /// A capability with a longer-running contract: emit progress, then answer.
    fn deferred(
        &self,
        call: Call,
        request: <Self::Deferred as Shape>::Request,
    ) -> impl Future<Output = Result<<Self::Deferred as Shape>::Response, PluginError>> + Send {
        let _ = (call, request);
        async {
            Err(PluginError::Unsupported(
                "deferred is not implemented".to_string(),
            ))
        }
    }

    /// A capability that parks on the cancellation token and is woken by a
    /// cancel (D15).
    fn cancellable(
        &self,
        call: Call,
        request: <Self::Cancellable as Shape>::Request,
    ) -> impl Future<Output = Result<<Self::Cancellable as Shape>::Response, PluginError>> + Send
    {
        let _ = (call, request);
        async {
            Err(PluginError::Unsupported(
                "cancellable is not implemented".to_string(),
            ))
        }
    }

    /// A capability that fails inside its own future, which the adapter turns
    /// into an internal-error completion instead of losing the call (D7).
    fn fault(
        &self,
        call: Call,
        request: <Self::Fault as Shape>::Request,
    ) -> impl Future<Output = Result<<Self::Fault as Shape>::Response, PluginError>> + Send {
        let _ = (call, request);
        async {
            Err(PluginError::Unsupported(
                "fault is not implemented".to_string(),
            ))
        }
    }

    /// A cancel for one of this plugin's calls was accepted.
    ///
    /// The call's token is already marked when this runs, so an arm parked on
    /// [`Call::cancellation`] has been woken. Returning an error here is
    /// reported as the cancel's status; the default accepts.
    fn cancel(&self, call: Call) -> Result<(), PluginError> {
        let _ = call;
        Ok(())
    }

    /// Instance initialisation, run by the adapter from `create_instance`.
    ///
    /// `Default` construction stays infallible and configuration-free; this is
    /// where initialisation reports failure, and the error's status is what
    /// `create_instance` returns. A plugin whose `started` fails yields no
    /// instance.
    fn started(&self, host: &Host) -> Result<(), PluginError> {
        let _ = host;
        Ok(())
    }

    /// Instance teardown, run by the adapter from `shutdown` after every worker
    /// has been joined, so no callback can arrive after it returns (D12).
    fn stopped(&self, host: &Host) {
        let _ = host;
    }
}
