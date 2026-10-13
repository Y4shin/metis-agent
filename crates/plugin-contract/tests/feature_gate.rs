//! The `authoring` feature seam, in the half that has to run with the feature
//! off.
//!
//! Three things make the gate worth something, and none of them needs the
//! feature on:
//!
//! - the feature is off by default, so `plugin-runtime` depends on the
//!   primitives and the payload envelope alone (D3);
//! - with it off the crate's ordinary dependency list is empty, which is what
//!   keeps an async runtime and serde out of the boundary;
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
fn the_authoring_feature_is_off_by_default_and_its_dependencies_are_optional() {
    let manifest = crate_file("Cargo.toml");

    let authoring = manifest_value(&manifest, "features", "authoring")
        .expect("the manifest declares an `authoring` feature");
    assert!(
        authoring.contains("dep:serde") && authoring.contains("dep:serde_json"),
        "the feature turns on the serde pair: {authoring}"
    );
    assert!(
        manifest_value(&manifest, "features", "default").is_none(),
        "no default feature turns `authoring` on"
    );
    for dependency in ["serde", "serde_json"] {
        let declared = manifest_value(&manifest, "dependencies", dependency)
            .unwrap_or_else(|| panic!("the manifest declares {dependency}"));
        assert!(
            declared.contains("optional = true"),
            "{dependency} is optional, so the feature is what pulls it in: {declared}"
        );
    }
}

#[test]
fn the_crate_has_no_ordinary_dependencies_with_the_feature_off() {
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
    let tree = String::from_utf8_lossy(&output.stdout);
    let roots: Vec<&str> = tree.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(
        roots.len(),
        1,
        "the crate stands alone with the feature off, but the tree is:\n{tree}"
    );
    assert!(
        roots[0].starts_with("plugin-contract "),
        "the one node is the crate itself: {tree}"
    );
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
