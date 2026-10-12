//! Spike-only payload types (JSON shapes that ride inside `MetisBuffer`).
//!
//! These are deliberately outside the C ABI crate: everything else is payload
//! (D1), and the C header must not contain JSON shapes.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Tool,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub name: String,
    pub kind: CapabilityKind,
    pub version: u32,
    pub schema: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityList {
    pub capabilities: Vec<CapabilityDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchoRequest {
    pub echo: String,
    #[serde(default = "one")]
    pub repeat: u32,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchoResponse {
    pub echo: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub message: String,
}

impl EchoResponse {
    /// Version of this payload shape, used by the capability descriptors.
    pub const SCHEMA_VERSION: u32 = 1;
}
