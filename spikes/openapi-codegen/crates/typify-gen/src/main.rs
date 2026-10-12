//! Alternative C input step: turn the OpenAPI document's `components.schemas`
//! into Rust types with `typify`.
//!
//! typify consumes JSON Schema, not OpenAPI, so this shim extracts the schemas and
//! rewrites `#/components/schemas/X` refs to `#/definitions/X`. That shim is part
//! of the cost of the typify alternative.

use schemars::schema::Schema;
use typify::{TypeSpace, TypeSpaceSettings};

fn main() {
    let mut args = std::env::args().skip(1);
    let spec_path = args
        .next()
        .expect("usage: typify-gen <openapi.yaml> <out.rs>");
    let out_path = args
        .next()
        .expect("usage: typify-gen <openapi.yaml> <out.rs>");

    let yaml: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&spec_path).expect("read spec"))
            .expect("parse yaml");
    let schemas = yaml
        .get("components")
        .and_then(|components| components.get("schemas"))
        .expect("components.schemas");
    let mut defs = serde_json::to_value(schemas).expect("schemas to json");
    rewrite_refs(&mut defs);

    let defs = defs.as_object().expect("schemas object");
    let mut settings = TypeSpaceSettings::default();
    settings.with_struct_builder(false);
    let mut type_space = TypeSpace::new(&settings);

    let named = defs
        .iter()
        .map(|(name, value)| {
            let schema: Schema = serde_json::from_value(value.clone()).expect("schema");
            (name.clone(), schema)
        })
        .collect::<Vec<_>>();
    type_space.add_ref_types(named).expect("add_ref_types");

    let tokens = type_space.to_stream();
    std::fs::write(&out_path, tokens.to_string()).expect("write output");
}

/// Rewrite OpenAPI `$ref` pointers into the JSON Schema `$ref` space.
fn rewrite_refs(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            if let Some(name) = text.strip_prefix("#/components/schemas/") {
                *text = format!("#/definitions/{name}");
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(rewrite_refs),
        serde_json::Value::Object(map) => map.values_mut().for_each(rewrite_refs),
        _ => {}
    }
}
