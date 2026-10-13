//! The capability declaration (D9) and the payload shapes a capability arm is
//! parameterised by.
//!
//! A capability is a name, a kind, a version, and a schema reference, and
//! `invoke` addresses it by name. The descriptor type lives here because it is
//! the declaration the export adapter lists and dispatches on. It is the one
//! payload shape this crate owns: the request and response bodies themselves
//! belong to the plugin, which is what [`Shape`] makes possible.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// V1 capability kinds (D9). A new kind never moves the ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    /// A tool: a bounded request, answered with a result.
    Tool,
    /// A provider: a capability with a longer-running contract.
    Provider,
}

/// One capability descriptor (D9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    /// The name `invoke` addresses, and the name an arm dispatches on.
    pub name: String,
    /// Tool or provider.
    pub kind: CapabilityKind,
    /// The capability's semantic version, which the host gates per capability
    /// (D10) rather than failing the whole plugin.
    pub version: u32,
    /// A reference to the capability's payload schema, for hosts and tooling.
    pub schema: String,
}

/// The `list_capabilities` payload: the descriptors a plugin advertises.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityList {
    /// The descriptors, in the order the plugin declared them.
    pub capabilities: Vec<CapabilityDescriptor>,
}

/// The payload shapes of one capability arm.
///
/// The authoring trait cannot name a plugin's types, so each arm is
/// parameterised by a shape the plugin defines: the request the adapter
/// deserialises from the payload body, and the response it serialises into one.
pub trait Shape {
    /// What a call for this arm deserialises into.
    type Request: DeserializeOwned + Send + 'static;
    /// What this arm's response serialises from.
    type Response: Serialize + Send + 'static;
}
