//! The capability declaration and its inline schemas.
//!
//! A capability is a name, a kind, a version, and the JSON Schema of its
//! request and response bodies. `invoke_future` addresses it by name, and the
//! descriptor is the one payload shape this crate owns: the bodies themselves
//! belong to the plugin.
//!
//! The schemas are inline rather than a reference to be fetched. The listing
//! carries both, so `create_instance` never needs a second call to learn what a
//! capability accepts, and a host validates in both directions without a cache
//! or a fetch-failure path.

use schemars::Schema;
use serde::{Deserialize, Serialize};

/// V1 capability kinds. A new kind never moves the ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    /// A tool: a bounded request, answered with a result.
    Tool,
    /// A provider: a capability with a longer-running contract.
    Provider,
}

/// One capability descriptor.
///
/// `name` is instance-qualified by the host as `<instance-name>.<tool-name>`;
/// the plugin declares the tool-level name, and the host's registry adds the
/// qualifier, so one plugin loaded twice keeps two distinct namespaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    /// The name `invoke_future` addresses, and the name a `call` routes on.
    pub name: String,
    /// Tool or provider.
    pub kind: CapabilityKind,
    /// The capability's semantic version, which the host gates per capability
    /// rather than failing the whole plugin.
    pub version: u32,
    /// The JSON Schema of this capability's request body.
    pub request_schema: Schema,
    /// The JSON Schema of this capability's response body.
    pub response_schema: Schema,
}

/// The `list_capabilities` payload: the descriptors a plugin advertises.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CapabilityList {
    /// The descriptors, in the order the plugin declared them.
    pub capabilities: Vec<CapabilityDescriptor>,
}
