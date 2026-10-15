//! The authoring trait, its initialisation payload, and its error type.
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

/// The host context: what the host tells a plugin about itself when it
/// initialises it.
///
/// The field names are the contract, and the JSON shape is what a C author
/// decodes by name. The set is deliberately permissive and forward-compatible:
/// a plugin ignores context fields it does not know, so a host may add one
/// without breaking an older plugin. Nothing secret belongs here; the host
/// decides what it discloses.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostContext {
    /// The name of the instance this plugin is being initialised as.
    pub instance_name: String,
    /// The directory the plugin may keep its own state in, when the host set
    /// one.
    #[serde(default)]
    pub data_dir: Option<String>,
    /// The highest capability descriptor version this host serves.
    #[serde(default = "default_capability_version")]
    pub capability_version: u32,
}

/// The capability descriptor version a host serves when it says nothing else.
fn default_capability_version() -> u32 {
    1
}

/// One initialisation payload: the plugin's own configuration and the host
/// context, as the two named parts of one body.
///
/// The two names are the contract and are documented for a C author. The parts
/// stay JSON values here because the configuration's shape belongs to the
/// plugin, which decodes it into its own type through [`Initialisation::config`],
/// and the host context is the shared [`HostContext`].
#[derive(Debug, Clone, PartialEq)]
pub struct Initialisation {
    config: serde_json::Value,
    host_context: serde_json::Value,
}

impl Initialisation {
    /// The body field carrying the plugin's own configuration.
    pub const CONFIG_FIELD: &'static str = "config";
    /// The body field carrying the host context.
    pub const HOST_CONTEXT_FIELD: &'static str = "host_context";

    /// Builds a payload from the configuration the host resolved and the host
    /// context it chose.
    pub fn new<C>(config: C, host_context: HostContext) -> Result<Initialisation, PluginError>
    where
        C: Serialize,
    {
        let config = serde_json::to_value(config)
            .map_err(|err| PluginError::Internal(format!("configuration payload: {err}")))?;
        let host_context = serde_json::to_value(host_context)
            .map_err(|err| PluginError::Internal(format!("host context payload: {err}")))?;
        Ok(Initialisation {
            config,
            host_context,
        })
    }

    /// The JSON body of this payload, which is what rides inside the envelope.
    pub fn body(&self) -> Vec<u8> {
        serde_json::json!({
            Self::CONFIG_FIELD: self.config,
            Self::HOST_CONTEXT_FIELD: self.host_context,
        })
        .to_string()
        .into_bytes()
    }

    /// Decodes the JSON body a host delivered.
    ///
    /// A body that is not one object with both named parts is the
    /// "configuration does not match what this plugin declared" class, because
    /// the plugin cannot read what it was handed.
    pub fn decode(body: &[u8]) -> Result<Initialisation, PluginError> {
        let value: serde_json::Value = serde_json::from_slice(body).map_err(|err| {
            PluginError::InvalidArgument(format!("the initialisation payload is not JSON: {err}"))
        })?;
        let object = value.as_object().ok_or_else(|| {
            PluginError::InvalidArgument("the initialisation payload is not one object".to_string())
        })?;
        let part = |field: &str| {
            object.get(field).cloned().ok_or_else(|| {
                PluginError::InvalidArgument(format!(
                    "the initialisation payload has no {field} part"
                ))
            })
        };
        Ok(Initialisation {
            config: part(Self::CONFIG_FIELD)?,
            host_context: part(Self::HOST_CONTEXT_FIELD)?,
        })
    }

    /// The plugin's own configuration, decoded into the author's type.
    pub fn config<T: DeserializeOwned>(&self) -> Result<T, PluginError> {
        serde_json::from_value(self.config.clone()).map_err(|err| {
            PluginError::InvalidArgument(format!(
                "the configuration does not match what this plugin declared: {err}"
            ))
        })
    }

    /// The host context.
    pub fn host_context(&self) -> Result<HostContext, PluginError> {
        serde_json::from_value(self.host_context.clone())
            .map_err(|err| PluginError::InvalidArgument(format!("the host context: {err}")))
    }
}

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
///
/// There is no `Default` bound. An instance that exists has completed
/// initialisation, so the only way to obtain a plugin value is the fallible,
/// asynchronous [`Plugin::init`]: an uninitialised value is not representable.
pub trait Plugin: Send + Sync + 'static {
    /// Instance construction and initialisation, as one asynchronous call.
    ///
    /// The plugin's own configuration and the host context arrive together in
    /// [`Initialisation`], and the host is available for logs and events. The
    /// future is polled by the host on the host's own runtime, so initialisation
    /// may await a host service and return `Pending` rather than block.
    ///
    /// A failure whose cause is the configuration the host handed over is
    /// [`PluginError::InvalidArgument`], which is the "the configuration does
    /// not match what this plugin declared" class. Every other error is the
    /// plugin's own defect. Both classes are distinguishable by the caller.
    fn init(
        host: Host,
        initialisation: Initialisation,
    ) -> impl Future<Output = Result<Self, PluginError>> + Send
    where
        Self: Sized;

    /// The capability descriptors this plugin advertises.
    ///
    /// The adapter returns exactly these from `list_capabilities`, and checks
    /// them for duplicate names once initialisation has resolved.
    fn capabilities(&self) -> Vec<CapabilityDescriptor>;

    /// One capability, routed by name. One request in, one response out.
    ///
    /// The future is polled by the host on the host's own runtime, and it must
    /// not block: a capability that has to wait asks for a host service through
    /// [`Call::service_request`] and returns `Pending` from its poll until the
    /// answer arrives.
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

    /// Instance teardown, run by the adapter from `shutdown`.
    ///
    /// The host drops every future before calling `shutdown`, so nothing is in
    /// flight here and no callback can arrive after it returns.
    fn stopped(&self, host: &Host) {
        let _ = host;
    }
}
