//! The fixture's payload shapes: JSON bodies that ride inside `MetisBuffer`.
//!
//! These types are deliberately not part of `plugin-contract`. Everything above
//! the primitive ABI is payload, and the fixture is a consumer of the envelope,
//! not a second definition of it. The harness in the same package reuses them so
//! that both sides of the boundary agree on the JSON, without any of it leaking
//! into the committed C header.
//!
//! The capability descriptor is the one shape the authoring module owns, because
//! it is the declaration the export adapter lists; this module re-exports it so
//! the harness parses exactly the type the adapter writes.

use serde::{Deserialize, Serialize};

pub use plugin_contract::authoring::{CapabilityDescriptor, CapabilityKind, CapabilityList};

/// The fixture's one request shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EchoRequest {
    pub echo: String,
    #[serde(default = "one")]
    pub repeat: u32,
}

fn one() -> u32 {
    1
}

/// The fixture's one response shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EchoResponse {
    pub echo: String,
    pub bytes: usize,
}

/// What a capability reports through the `progress` callback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Progress {
    pub stage: String,
}

/// What a capability asks the host's `timer` service for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TimerRequest {
    pub delay_ms: u64,
}

/// What the host answers a timer request with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TimerAnswer {
    pub delay_ms: u64,
}
