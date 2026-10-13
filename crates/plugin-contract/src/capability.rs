//! The capability declaration: one type both sides of the wire share.
//!
//! A capability is a name, a kind, a version, and the JSON Schema of its request
//! and response bodies. `invoke_future` addresses it by name, and this payload
//! is the one shape this crate owns: the bodies themselves belong to the plugin.
//!
//! The type is deliberately **not** behind the `authoring` feature, because both
//! sides of the boundary read the same declaration: a plugin publishes it
//! through `list_capabilities`, and a host parses the very same bytes and routes
//! with them. One type beats two identical ones, because two can drift and one
//! cannot. Behaviour is gated instead of data: `schemars`-based derivation lives
//! behind `authoring` (see `authoring::schema_of`), and schema validation lives
//! in the host, because it needs `jsonschema` and this crate does not.
//!
//! The schemas are inline rather than a reference to be fetched. The listing
//! carries both, so `create_instance` never needs a second call to learn what a
//! capability accepts, and a host validates in both directions without a cache
//! or a fetch-failure path.
//!
//! `kind` is a `String` rather than an enum on purpose. A strict enum would fail
//! the whole declaration when a newer plugin declares a kind this build does not
//! know, where the behaviour that keeps one bad capability from taking a plugin
//! down is to refuse that capability alone. [`CapabilityKind`] is the helper
//! that names the kinds this build does know, and a host refuses the capability
//! whose kind is not one of them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The capability kinds this build knows.
///
/// A helper, not the field's type, so an unknown kind round-trips through the
/// declaration instead of failing its parse. A new kind never moves the ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityKind {
    /// A tool: a bounded request, answered with a result.
    Tool,
    /// A provider: a capability with a longer-running contract.
    Provider,
}

impl CapabilityKind {
    /// The wire name of this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            CapabilityKind::Tool => "tool",
            CapabilityKind::Provider => "provider",
        }
    }

    /// The kind a wire name names, or `None` when this build does not know it.
    pub fn of(name: &str) -> Option<CapabilityKind> {
        match name {
            "tool" => Some(CapabilityKind::Tool),
            "provider" => Some(CapabilityKind::Provider),
            _ => None,
        }
    }
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
    /// The wire name of the kind. V1 knows `tool` and `provider`.
    pub kind: String,
    /// The capability's semantic version, which the host gates per capability
    /// rather than failing the whole plugin.
    pub version: u32,
    /// The JSON Schema of this capability's request body.
    pub request_schema: Value,
    /// The JSON Schema of this capability's response body.
    pub response_schema: Value,
}

impl CapabilityDescriptor {
    /// A `tool` capability.
    pub fn tool(
        name: impl Into<String>,
        version: u32,
        request_schema: Value,
        response_schema: Value,
    ) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: name.into(),
            kind: CapabilityKind::Tool.as_str().to_string(),
            version,
            request_schema,
            response_schema,
        }
    }

    /// A `provider` capability.
    pub fn provider(
        name: impl Into<String>,
        version: u32,
        request_schema: Value,
        response_schema: Value,
    ) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: name.into(),
            kind: CapabilityKind::Provider.as_str().to_string(),
            version,
            request_schema,
            response_schema,
        }
    }

    /// The kind this descriptor names, or `None` when this build does not know
    /// it and must refuse this one capability.
    pub fn known_kind(&self) -> Option<CapabilityKind> {
        CapabilityKind::of(&self.kind)
    }
}

/// The `list_capabilities` payload: the descriptors a plugin advertises.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CapabilityList {
    /// The descriptors, in the order the plugin declared them.
    pub capabilities: Vec<CapabilityDescriptor>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_constructors_name_their_kind_on_the_wire() {
        let tool = CapabilityDescriptor::tool("tool.echo", 2, json!({}), json!({}));
        assert_eq!(tool.kind, "tool");
        assert_eq!(tool.known_kind(), Some(CapabilityKind::Tool));

        let provider = CapabilityDescriptor::provider("provider.llm", 1, json!({}), json!({}));
        assert_eq!(provider.kind, "provider");
        assert_eq!(provider.known_kind(), Some(CapabilityKind::Provider));
    }

    #[test]
    fn a_kind_this_build_does_not_know_parses_instead_of_failing_the_declaration() {
        let body = json!({
            "capabilities": [{
                "name": "tool.future",
                "kind": "widget",
                "version": 3,
                "request_schema": {"type": "object"},
                "response_schema": {"type": "object"},
            }],
        });
        let list: CapabilityList =
            serde_json::from_value(body).expect("an unknown kind still parses");
        assert_eq!(list.capabilities.len(), 1);
        assert_eq!(list.capabilities[0].known_kind(), None);
        assert_eq!(CapabilityKind::of("widget"), None);
        assert_eq!(CapabilityKind::of("tool"), Some(CapabilityKind::Tool));
    }
}
