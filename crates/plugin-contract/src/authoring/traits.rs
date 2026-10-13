//! The authoring trait and its error type.
//!
//! This module deliberately names no ABI type. Everything the ABI forced onto
//! the authoring surface is in `call` (the per-request context and the host
//! routes) and in the bounds and signatures below, and the status mapping lives
//! in `plugin_adapter`, on the ABI side of the line.

use std::fmt;
use std::future::Future;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::authoring::call::{Call, Host};
use crate::capability::CapabilityDescriptor;

/// One call's request, addressed by capability name.
///
/// The body is the payload's bytes with the one-byte envelope already stripped:
/// an author never sees a header, an owner tag, or a length. One call carries
/// one capability name and one body, and there is no second request type per
/// capability: the author decodes `body` into whatever the capability's
/// published schema says.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The capability this call was routed to.
    pub capability: &'a str,
    /// The request body, envelope stripped.
    pub body: &'a [u8],
}

impl<'a> Request<'a> {
    /// Builds a request from a capability name and a body.
    pub const fn new(capability: &'a str, body: &'a [u8]) -> Self {
        Request { capability, body }
    }

    /// Decodes the body as JSON into the caller's request type.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, PluginError> {
        serde_json::from_slice(self.body).map_err(|err| {
            PluginError::InvalidArgument(format!("{} request body: {err}", self.capability))
        })
    }
}

/// One call's response: the body bytes the host reads back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The response body, which the adapter wraps in the envelope.
    pub body: Vec<u8>,
}

impl Response {
    /// Builds a response from raw body bytes.
    pub const fn new(body: Vec<u8>) -> Self {
        Response { body }
    }

    /// Builds a response by serialising a value as JSON.
    pub fn json<T: Serialize>(value: &T) -> Result<Self, PluginError> {
        serde_json::to_vec(value)
            .map(|body| Response { body })
            .map_err(|err| PluginError::Internal(format!("response body: {err}")))
    }

    /// The body bytes.
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

/// The error a plugin reports.
///
/// The variants mirror the ABI's frozen status space minus `Ok` and `Pending`:
/// `Pending` is a poll outcome rather than an error, and the remaining six are
/// the failure space a plugin can express. The first five carry a detail string
/// for the host's log, and `Cancelled` is control flow: it is the terminal
/// status a cancelled call resolves as.
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

impl std::error::Error for PluginError {}

/// What a plugin author implements.
///
/// One routed call, a capability declaration, an instance lifecycle pair, and a
/// cancellation hook. There is no arm per capability and no per-capability
/// request type: `call` receives the capability name and routes on it, so the
/// number of capabilities a plugin advertises is limited only by what it
/// declares. Per-capability typing above this trait is the SDK's generated
/// facade, not a trait constraint.
///
/// Two bounds are the ABI's, not the author's, and are documented rather than
/// hidden: the plugin is `Send + Sync + 'static` because the host may poll one
/// of its futures from any thread, and its futures are `Send` for the same
/// reason. Nothing here spawns or blocks: the host owns when plugin code runs.
pub trait Plugin: Default + Send + Sync + 'static {
    /// The capability descriptors this plugin advertises.
    ///
    /// The adapter returns exactly these from `list_capabilities`, and checks
    /// them for duplicate names at instance creation.
    fn capabilities(&self) -> Vec<CapabilityDescriptor>;

    /// One capability, routed by name. One request in, one response out.
    ///
    /// The future is polled by the host on the host's own runtime, and it must
    /// not block: a capability that has to wait asks for a host service through
    /// [`Call::service`] and returns `Pending` from its poll until the answer
    /// arrives.
    fn call(
        &self,
        call: Call,
        request: Request<'_>,
    ) -> impl Future<Output = Result<Response, PluginError>> + Send;

    /// A cancel for one of this plugin's calls was accepted.
    ///
    /// The call's token is already marked when this runs, so a future parked on
    /// [`Call::cancellation`] has been woken and will resolve as
    /// [`PluginError::Cancelled`] on its next poll. Returning an error here is
    /// reported as the cancel's own status; the default accepts.
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

    /// Instance teardown, run by the adapter from `shutdown`.
    ///
    /// The host drops every future before calling `shutdown`, so nothing is in
    /// flight here and no callback can arrive after it returns.
    fn stopped(&self, host: &Host) {
        let _ = host;
    }
}
