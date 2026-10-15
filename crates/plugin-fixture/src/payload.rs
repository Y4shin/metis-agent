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

pub use plugin_contract::authoring::{
    CapabilityDescriptor, CapabilityKind, CapabilityList, HostContext, Initialisation,
};

/// The configuration the fixture declares.
///
/// The fixture is loaded directly rather than from a bundle, so no manifest
/// declares a configuration schema for it: this type is the declaration, and a
/// configuration that does not decode into it, or that was resolved for another
/// instance, is the "does not match what the plugin declared" class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FixtureConfig {
    /// The instance name this configuration was resolved for.
    pub instance: String,
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The unknown-field rule, which is what makes a payload additive: a type
    /// never denies a field it does not know, so a body from a newer build
    /// decodes here and the fields this build does know keep their meaning.
    #[test]
    fn a_request_with_a_field_this_build_does_not_know_decodes() {
        let request: EchoRequest = serde_json::from_str(
            r#"{"echo":"later","repeat":2,"added_after_this_build":{"nested":true}}"#,
        )
        .expect("an unknown field is ignored rather than fatal");
        assert_eq!(
            request,
            EchoRequest {
                echo: "later".to_string(),
                repeat: 2,
            }
        );
    }

    /// The other half of the same rule: a field added later is optional with a
    /// default, so a body from an older build still decodes. `repeat` is this
    /// build's live example of it.
    #[test]
    fn a_request_without_a_field_that_has_a_default_decodes() {
        let request: EchoRequest =
            serde_json::from_str(r#"{"echo":"older"}"#).expect("repeat has a default");
        assert_eq!(
            request,
            EchoRequest {
                echo: "older".to_string(),
                repeat: 1,
            }
        );
    }

    #[test]
    fn a_response_with_a_field_this_build_does_not_know_decodes() {
        let response: EchoResponse =
            serde_json::from_str(r#"{"echo":"hi","bytes":2,"added_after_this_build":null}"#)
                .expect("an unknown field is ignored rather than fatal");
        assert_eq!(
            response,
            EchoResponse {
                echo: "hi".to_string(),
                bytes: 2,
            }
        );
    }
}
