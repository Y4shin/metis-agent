//! PROTOTYPE (throwaway). Not product code.
//!
//! Schema helpers shared by the native plugins and the host's payload types.
//!
//! The seam is bytes shaped, so a host that wants typed calls needs its own copy
//! of the payload types; nothing checks at compile time that they match the
//! plugin's. `MismatchedEchoRequest` shows what that costs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A JSON Schema 2020-12 document generated from a Rust type, with the dialect
/// pinned because `jsonschema` reads it from `$schema`.
pub fn schema_of<T: JsonSchema>() -> Value {
    let mut value = schemars::schema_for!(T).to_value();
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "$schema".to_owned(),
            serde_json::json!("https://json-schema.org/draft/2020-12/schema"),
        );
    }
    value
}

/// The schema id a plugin publishes for one capability side.
pub fn schema_id(plugin: &str, capability: &str, side: &str) -> String {
    format!("{plugin}/{capability}.{side}")
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EchoRequest {
    pub message: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EchoResponse {
    pub echo: String,
    pub length: u64,
}

/// Deliberately incompatible with the cdylib's `tool.echo` request schema: the
/// field is `text`, not `message`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MismatchedEchoRequest {
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AddRequest {
    pub a: i64,
    pub b: i64,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AddResponse {
    pub sum: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct UppercaseRequest {
    pub text: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct UppercaseResponse {
    pub upper: String,
    pub bytes: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BadResponseRequest {
    pub x: i64,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BadResponse {
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CompleteRequest {
    pub prompt: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CompleteResponse {
    pub text: String,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WordCountRequest {
    pub text: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WordCountResponse {
    pub words: u64,
    pub chars: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SummarizeRequest {
    pub text: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SummarizeResponse {
    pub summary: String,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ThrowawayRequest {
    pub value: i64,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ThrowawayResponse {
    pub doubled: i64,
}
