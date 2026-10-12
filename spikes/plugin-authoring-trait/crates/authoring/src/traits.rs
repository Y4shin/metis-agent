//! The throwaway authoring trait, roughly what `plugin-contract` will ship.
//!
//! This module deliberately does not mention the ABI. Everything the ABI forced
//! onto the authoring surface lives in `call.rs` (the per-request context) and
//! in the `Send`/`'static` bounds below.

use std::future::Future;

use crate::call::Call;
use crate::payload::{
    CapabilityDescriptor, CompletionRequest, CompletionResponse, EchoRequest, EchoResponse,
};

/// The error an authoring plugin reports.
///
/// The variants mirror the ABI's frozen status space (D7) minus `ok` and
/// `pending`. A trait designed without the ABI in view would use an ordinary
/// Rust error type. See the findings, bend 5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    InvalidArgument(String),
    NotFound(String),
    PermissionDenied(String),
    Unsupported(String),
    Internal(String),
    Cancelled,
}

/// What a plugin author implements.
///
/// Three capability methods plus a cancellation method, dispatched through the
/// ABI's one generic `invoke` by capability name (D1, D9).
///
/// Two things in this signature exist for the ABI and not for the author:
///
/// * `Send + Sync + 'static` on the plugin, and `Future + Send` on every
///   capability method. `invoke` must return pending (D11), so the adapter moves
///   the future onto a worker thread, and only a `Send + 'static` future can
///   travel. The trait therefore dictates the executor's threading model.
/// * `Call` as a per-call argument, which carries the request id, the
///   cancellation flag, and the progress route.
pub trait Plugin: Send + Sync + 'static {
    /// Capability descriptors this plugin claims (D9). The names must match the
    /// names the adapter dispatches on; nothing checks that they agree.
    fn capabilities(&self) -> Vec<CapabilityDescriptor>;

    /// Tool capability `tool.echo`: answer straight away.
    fn echo(
        &self,
        call: Call,
        request: EchoRequest,
    ) -> impl Future<Output = Result<EchoResponse, PluginError>> + Send;

    /// Provider capability `provider.complete`: emit progress, do some work,
    /// and give up early when the request was cancelled.
    fn complete(
        &self,
        call: Call,
        request: CompletionRequest,
    ) -> impl Future<Output = Result<CompletionResponse, PluginError>> + Send;

    /// Tool capability `tool.uncancellable`: finish even after a successful
    /// cancel, which forces the host to drop the late completion (D12).
    fn uncancellable(
        &self,
        call: Call,
        request: EchoRequest,
    ) -> impl Future<Output = Result<EchoResponse, PluginError>> + Send;

    /// Cancellation. The ABI's `cancel` entry point carries a request id only,
    /// so the adapter rebuilds a `Call` from its registry before calling this.
    fn cancel(&self, call: Call) -> Result<(), PluginError>;
}
