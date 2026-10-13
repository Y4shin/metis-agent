//! The fixture's payload shapes: JSON bodies that ride inside `MetisBuffer`.
//!
//! These types are deliberately not part of `plugin-contract`. Everything above
//! the primitive ABI is payload (D1, D9), and the fixture is a consumer of the
//! envelope, not a second definition of it. The harness in the same package
//! reuses them so that both sides of the boundary agree on the JSON, without
//! any of it leaking into the committed C header.

use serde::{Deserialize, Serialize};

/// V1 capability kinds (D9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Tool,
    Provider,
}

/// One capability descriptor (D9): a name, a kind, a version, a schema
/// reference. `invoke` addresses an entry by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub name: String,
    pub kind: CapabilityKind,
    pub version: u32,
    pub schema: String,
}

/// The `list_capabilities` payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityList {
    pub capabilities: Vec<CapabilityDescriptor>,
}

/// The fixture's one request shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchoRequest {
    pub echo: String,
    #[serde(default = "one")]
    pub repeat: u32,
}

fn one() -> u32 {
    1
}

/// The fixture's one response shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchoResponse {
    pub echo: String,
    pub bytes: usize,
}
