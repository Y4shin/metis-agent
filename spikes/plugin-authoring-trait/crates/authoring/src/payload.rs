//! Payload shapes for this spike.
//!
//! The shared shapes are the mechanics spike's `spike-payload` crate, reused by
//! path rather than re-declared. The provider shapes are added here, which is
//! the point of D1 ("everything else is payload"): a new capability needs new
//! payload types and no ABI change at all.

pub use spike_payload::{
    CapabilityDescriptor, CapabilityKind, CapabilityList, EchoRequest, EchoResponse, ErrorDetail,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub prompt: String,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub stage: String,
}
