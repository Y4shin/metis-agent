//! The manifest step, tested at its one public seam: the manifest a staged
//! bundle carries, the library this host selects from it, and the library this
//! host maps from that selection.
//!
//! Every bundle here is built inside the test, the way a producer builds one:
//! the fixture cdylib's own bytes are written into an archive beside a manifest
//! this file writes by hand, then signed, verified and extracted. So the format
//! under test is the format an author writes, not a format only a generator
//! could produce.
//!
//! The target is derived here rather than imported, because the contract fixes
//! it as the operating system and the architecture joined by a hyphen, and
//! nothing in the runtime exports that derivation.

mod support;

use std::io::{Cursor, Write};

use plugin_runtime::bundle::{StagedBundle, TrustStore, extract, verify_bundle};
use plugin_runtime::manifest::{Manifest, ManifestError};
use serde_json::json;
use support::signer::{self, TestChain};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// The name the test chain's leaf-authorising certificate carries, which is the
/// name every manifest here declares.
const NAME: &str = signer::LEAF_COMMON_NAME;

/// The manifest's name at the archive root, as the contract fixes it.
const MANIFEST: &str = "manifest.toml";

/// This host's target, spelled the way the contract fixes it: the operating
/// system and the architecture joined by a hyphen.
fn host_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The file name the fixture's own library carries.
fn library_file_name() -> String {
    support::fixture_path()
        .file_name()
        .expect("the fixture library has a file name")
        .to_string_lossy()
        .into_owned()
}

/// The bytes a bundle carries as this host's library: the fixture cdylib.
fn library_bytes() -> Vec<u8> {
    std::fs::read(support::fixture_path()).expect("the fixture library this test built reads")
}

/// The archive-relative path a library for `target` is carried at.
fn library_path(target: &str, file_name: &str) -> String {
    format!("lib/{target}/{file_name}")
}

/// One archive's entries: the root manifest when there is one, this host's
/// library, and anything else the test adds.
fn entries(
    target: &str,
    manifest: Option<&str>,
    extra: &[(String, Vec<u8>)],
) -> Vec<(String, Vec<u8>)> {
    let mut entries = Vec::new();
    if let Some(manifest) = manifest {
        entries.push((MANIFEST.to_string(), manifest.as_bytes().to_vec()));
    }
    entries.push((library_path(target, &library_file_name()), library_bytes()));
    entries.extend(extra.iter().cloned());
    entries
}

/// Writes the entries into a ZIP archive, entry by entry.
///
/// Every entry declares the mode this host writes, because the extractor
/// refuses any other declared mode, and stored rather than deflated, because
/// compression is not what this step tests.
fn archive(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(0o600);
        writer
            .start_file(name.as_str(), options)
            .unwrap_or_else(|error| panic!("the entry {name} starts: {error}"));
        writer
            .write_all(bytes)
            .unwrap_or_else(|error| panic!("the entry {name} writes: {error}"));
    }
    writer.finish().expect("the archive finishes").into_inner()
}

/// A verified bundle, extracted for `target`.
///
/// The test chain's anchor is installed as a root explicitly: the runtime's
/// compiled-in root set trusts nothing.
fn staged(entries: &[(String, Vec<u8>)], target: &str) -> StagedBundle {
    let chain = TestChain::standard(3);
    let bytes = signer::signed_bundle(&chain, &archive(entries));
    let trust = TrustStore::empty().with_root(chain.anchor().public_key());
    let verified = verify_bundle(&bytes, &trust).expect("the bundle this test signed verifies");
    extract(&verified, target).expect("this archive carries the target it was built for")
}

/// A staged bundle for this host: `manifest` at the archive root when it is
/// given, beside the fixture's own library.
fn staged_with(manifest: Option<&str>) -> StagedBundle {
    let target = host_target();
    staged(&entries(&target, manifest, &[]), &target)
}

/// The manifest an author writes by hand, which is what every bundle here
/// carries.
///
/// The text is a literal with the target and the library's file name filled in,
/// not a serialisation of any type in this tree: the format belongs to the
/// author, and a hand-written manifest is what proves it.
fn hand_written(name: &str) -> String {
    let target = host_target();
    format!(
        r#"# A hand-written manifest: the proof that packaging a plugin needs no
# generator. The host trusts this list for discovery.

name = "{name}"
version = "1.0.0"
abi_major = 1
min_abi_minor = 2

# One library path per target, relative to the archive root.
[libraries]
{target} = "lib/{target}/{library}"

# The capability list, with each capability's inline schemas.
[[capabilities]]
name = "tool.echo"
kind = "tool"
version = 1

[capabilities.request_schema]
type = "object"

[capabilities.response_schema]
type = "object"

# The configuration schema the host resolves the plugin's configuration with.
[config_schema]
type = "object"
"#,
        library = library_file_name(),
    )
}

#[test]
fn an_absent_manifest_and_an_invalid_one_are_different_typed_errors() {
    let absent = staged_with(None);
    let error = Manifest::read(&absent, NAME).expect_err("a bundle with no root manifest has none");
    assert_eq!(
        error,
        ManifestError::Absent {
            path: absent.manifest_path().to_path_buf()
        },
        "the refusal names the path the manifest would have been staged at"
    );

    let invalid = staged_with(Some("version = \"1.0.0\"\n"));
    let error = Manifest::read(&invalid, NAME).expect_err("a manifest without a name is refused");
    match error {
        ManifestError::Invalid { detail } => assert!(
            detail.contains("name"),
            "the refusal names what is wrong with the manifest: {detail}"
        ),
        other => {
            panic!("an invalid manifest is not an absent one, and this refusal is {other:?}")
        }
    }
}

#[test]
fn the_manifests_name_must_be_the_name_the_bundle_chain_carries() {
    let staged = staged_with(Some(&hand_written("dev.metis.other-plugin")));
    let error =
        Manifest::read(&staged, NAME).expect_err("a manifest that names another plugin is refused");
    assert_eq!(
        error,
        ManifestError::NameMismatch {
            declared: "dev.metis.other-plugin".to_string(),
            chain: NAME.to_string(),
        },
        "the chain's name is what the manifest is compared against"
    );
}

#[test]
fn a_hand_written_manifest_loads_with_every_declared_field() {
    let target = host_target();
    let library = format!("lib/{target}/{}", library_file_name());
    let text = hand_written(NAME);

    let manifest = Manifest::parse(&text).expect("a hand-written manifest is the format");
    assert_eq!(manifest.name(), NAME);
    assert_eq!(manifest.version(), "1.0.0");
    assert_eq!(manifest.abi_major(), 1);
    assert_eq!(manifest.min_abi_minor(), 2);
    assert_eq!(
        manifest.libraries().get(&target).map(String::as_str),
        Some(library.as_str()),
        "the manifest declares one library path per target"
    );
    assert_eq!(manifest.capabilities().len(), 1);
    let capability = &manifest.capabilities()[0];
    assert_eq!(capability.name, "tool.echo");
    assert_eq!(capability.kind, "tool");
    assert_eq!(capability.version, 1);
    assert_eq!(capability.request_schema, json!({"type": "object"}));
    assert_eq!(capability.response_schema, json!({"type": "object"}));
    assert_eq!(manifest.config_schema(), &json!({"type": "object"}));

    let staged = staged_with(Some(&text));
    let loaded = Manifest::read(&staged, NAME).expect("the bundle's own manifest loads");
    assert_eq!(
        loaded, manifest,
        "the bundle's manifest is the text the author wrote"
    );
}
