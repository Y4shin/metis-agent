//! Schema derivation for authors.
//!
//! Deriving a JSON Schema from a payload type needs `schemars`, which only an
//! author needs: a host validates against the bytes a plugin published and never
//! derives anything. So this lives behind the `authoring` feature, while the
//! declaration it fills in ([`crate::capability::CapabilityDescriptor`]) stays
//! available to both sides.

use schemars::JsonSchema;
use serde_json::Value;

/// The inline JSON Schema a payload type implies, ready for a descriptor field.
///
/// ```ignore
/// let descriptor = CapabilityDescriptor::tool(
///     "tool.echo",
///     1,
///     schema_of::<EchoRequest>(),
///     schema_of::<EchoResponse>(),
/// );
/// ```
pub fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T))
        .expect("a derived JSON Schema serialises to JSON")
}
