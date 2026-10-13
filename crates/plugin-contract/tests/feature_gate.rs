//! The `authoring` feature seam, in the half that has to run with the feature
//! off.
//!
//! Three things make the gate worth something, and none of them needs the
//! feature on:
//!
//! - the feature is off by default, so `plugin-runtime` depends on the
//!   primitives, the payload envelope and the capability declaration alone (D3);
//! - with it off the ordinary dependency closure is the serde pair the ungated
//!   declaration needs and nothing above the boundary, so no async runtime, no
//!   `schemars` and no `jsonschema` reaches the primitives crate;
//! - the trait module names no ABI type, which is the whole point of the
//!   authoring surface.
//!
//! The other half (the crate still builds with the feature off, and the
//! committed header does not grow) belongs to
//! `scripts/regenerate-abi-header.sh --check`, which parses the crate with
//! default features.

use std::path::PathBuf;
use std::process::Command;

fn crate_file(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} is readable: {err}", path.display()))
}

/// The value of a top-level key in a manifest section, without a TOML parser.
fn manifest_value(manifest: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == format!("[{section}]");
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            return Some(value.trim().to_string());
        }
    }
    None
}

#[test]
fn the_authoring_feature_is_off_by_default_and_only_schemars_is_optional() {
    let manifest = crate_file("Cargo.toml");

    let authoring = manifest_value(&manifest, "features", "authoring")
        .expect("the manifest declares an `authoring` feature");
    assert!(
        authoring.contains("dep:schemars"),
        "the feature turns on schema derivation: {authoring}"
    );
    assert!(
        manifest_value(&manifest, "features", "default").is_none(),
        "no default feature turns `authoring` on"
    );
    // The declaration rides as a JSON payload both sides read, so the serde pair
    // is deliberately ungated. Only derivation is an author's business.
    for dependency in ["serde", "serde_json"] {
        let declared = manifest_value(&manifest, "dependencies", dependency)
            .unwrap_or_else(|| panic!("the manifest declares {dependency}"));
        assert!(
            !declared.contains("optional = true"),
            "{dependency} backs the ungated declaration, so it is not optional: {declared}"
        );
    }
    let schemars = manifest_value(&manifest, "dependencies", "schemars")
        .expect("the manifest declares schemars");
    assert!(
        schemars.contains("optional = true"),
        "schemars is optional, so only the feature pulls it in: {schemars}"
    );
}

#[test]
fn the_feature_off_dependency_closure_carries_nothing_above_the_boundary() {
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "plugin-contract",
            "--edges",
            "normal",
            "--no-default-features",
        ])
        .output()
        .expect("cargo can be run from a test, as the ABI harness already does");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8_lossy(&output.stdout).to_string();
    for absent in [
        "schemars",
        "jsonschema",
        "tokio",
        "async-std",
        "smol",
        "futures",
        "async-ffi",
    ] {
        assert!(
            !tree.contains(absent),
            "{absent} must not reach the boundary with the feature off:\n{tree}"
        );
    }
    // The one cost the ungated declaration is allowed: the serde pair it is
    // serialised with, which the host already depends on.
    for present in ["serde", "serde_json"] {
        assert!(
            tree.contains(present),
            "the declaration is serialised with {present}, which is in the closure:\n{tree}"
        );
    }
}

#[test]
fn the_trait_module_names_no_abi_type() {
    let traits = crate_file("src/authoring/traits.rs");

    assert!(
        !traits.contains("crate::abi"),
        "the trait module reaches for no ABI type"
    );
    assert!(
        !traits.contains("Metis"),
        "no ABI type name appears in the trait module: the ABI types an author \
         surface needs are carried by `call` and the adapters, not by the trait"
    );
}
