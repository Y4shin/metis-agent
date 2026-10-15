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

use std::ffi::c_void;
use std::io::{Cursor, Write};
use std::mem::size_of;
use std::path::PathBuf;
use std::process::Command;

use plugin_contract::abi::{
    METIS_ABI_MAJOR, METIS_ABI_MINOR, MetisAbiError, MetisTablePrefix,
    metis_plugin_table_has_init_plugin, metis_read_table,
};
use plugin_runtime::bundle::{StagedBundle, TrustStore, extract, verify_bundle};
use plugin_runtime::manifest::{Manifest, ManifestError};
use plugin_runtime::{CallError, Loaded};
use serde_json::json;
use support::signer::{self, TestChain};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// The name the test chain's leaf-authorising certificate carries, which is the
/// name every manifest here declares.
const NAME: &str = signer::LEAF_COMMON_NAME;

/// The manifest's name at the archive root, as the contract fixes it.
const MANIFEST: &str = "manifest.toml";

/// The entry symbol this build resolves, which is the frozen name for major 1.
const ENTRY: &str = "metis_plugin_entry_v1";

/// The entry symbol a plugin built for the next major exports, which this build
/// cannot resolve at all.
const OTHER_ENTRY: &str = "metis_plugin_entry_v2";

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
/// carries, declaring the library this host needs.
///
/// The text is a literal, not a serialisation of any type in this tree: the
/// format belongs to the author, and a hand-written manifest is what proves it.
fn hand_written(name: &str) -> String {
    hand_written_with(name, &declaration(&host_target()))
}

/// The same hand-written manifest, with the given `[libraries]` declarations.
fn hand_written_with(name: &str, libraries: &str) -> String {
    format!(
        r#"# A hand-written manifest: the proof that packaging a plugin needs no
# generator. The host trusts this list for discovery.

name = "{name}"
version = "1.0.0"
abi_major = 1
min_abi_minor = 2

# One library path per target, relative to the archive root.
[libraries]
{libraries}

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
"#
    )
}

/// A target that is not this host's, so a bundle can carry more than this host
/// needs.
fn other_target() -> String {
    if host_target() == "linux-x86_64" {
        "macos-aarch64".to_string()
    } else {
        "linux-x86_64".to_string()
    }
}

/// The `[libraries]` declaration for this host's own library.
fn declaration(target: &str) -> String {
    format!(
        "{target} = \"lib/{target}/{library}\"",
        library = library_file_name()
    )
}

/// The hand-written manifest with its two ABI numbers changed, which is how a
/// test asks for a manifest this build does not serve.
fn manifest_needing(name: &str, abi_major: u32, min_abi_minor: u32) -> String {
    hand_written(name)
        .replace("abi_major = 1", &format!("abi_major = {abi_major}"))
        .replace(
            "min_abi_minor = 2",
            &format!("min_abi_minor = {min_abi_minor}"),
        )
}

/// The repository root, which is where the test's own scratch files live.
fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("the repository root is two levels above this crate")
        .to_path_buf()
}

/// The `rustc` the fixture is compiled with.
///
/// Every command of this effort runs inside the devenv shell, which is where the
/// toolchain's `rustc` is on `PATH`; `RUSTC` names it when a caller set that.
/// The `cargo` this test was run by is not used: in the pinned toolchain it is a
/// wrapper with no `rustc` beside it.
fn rustc() -> std::ffi::OsString {
    std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into())
}

/// A library whose table reports `size`, `major` and `minor` under `symbol`.
///
/// The library is compiled once per set of numbers, from the fixture source in
/// `tests/fixtures/`, so the reader is shown a table from a plugin built
/// elsewhere rather than this build's own.
fn patched_table(size: usize, major: u32, minor: u32, symbol: &str) -> PathBuf {
    let directory = repository_root().join("target/manifest-tests");
    std::fs::create_dir_all(&directory).expect("the test's scratch directory is creatable");
    let name = format!("table-{size}-{major}-{minor}-{symbol}");
    let library = directory.join(format!("lib{name}{}", std::env::consts::DLL_SUFFIX));
    if library.exists() {
        return library;
    }
    let source = directory.join(format!("{name}.rs"));
    let text = include_str!("fixtures/patched_table.rs")
        .replace("SYMBOL", symbol)
        .replace("SIZE", &size.to_string())
        .replace("MAJOR", &major.to_string())
        .replace("MINOR", &minor.to_string());
    std::fs::write(&source, text).expect("the fixture's source is written");
    let status = Command::new(rustc())
        .arg("--crate-type=cdylib")
        .arg("--edition=2024")
        .arg("-o")
        .arg(&library)
        .arg(&source)
        .status()
        .expect("rustc is runnable from a test, as the fixture build already assumes");
    assert!(
        status.success(),
        "the library that reports the table this test chose builds: {}",
        library.display()
    );
    library
}

/// Maps a library for a manifest, which is the step this seam is about.
fn open(library: impl AsRef<std::path::Path>, manifest: &Manifest) -> Result<Loaded, CallError> {
    // SAFETY: every library these tests map is one this test wrote or built from
    // the tree it is testing, which is the trust the caller owes `Loaded::open`.
    unsafe { Loaded::open(library, manifest) }
}

/// The refusal a mapping earns.
///
/// A mapped library is not `Debug`, so the tests read the refusal out by hand
/// rather than through `expect_err`.
fn refusal(result: Result<Loaded, CallError>, what: &str) -> CallError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{what}"),
    }
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

#[test]
fn selection_chooses_this_hosts_library_and_leaves_the_other_targets_declaration_alone() {
    let target = host_target();
    let other = other_target();
    // The bundle carries both targets' files, and the manifest declares both,
    // but the other target's declaration names a file this bundle does not
    // carry: only this host's declaration is resolved.
    let other_entry = format!("lib/{other}/libother.dylib");
    let manifest_text = hand_written_with(
        NAME,
        &format!(
            "{target} = \"lib/{target}/{library}\"
{other} = \"{other_entry}\"",
            library = library_file_name()
        ),
    );
    let extra = [(other_entry.clone(), b"another target's bytes".to_vec())];
    let staged = staged(&entries(&target, Some(&manifest_text), &extra), &target);
    let manifest = Manifest::read(&staged, NAME).expect("the manifest loads");

    let selected = manifest
        .select_library(&staged, &target)
        .expect("this host's declaration names the library the bundle carries");
    assert_eq!(
        selected,
        staged
            .root()
            .join(library_path(&target, &library_file_name())),
        "selection resolves this host's declaration against the staging root"
    );
    assert_eq!(
        std::fs::read(&selected).expect("the selected library is a file"),
        library_bytes(),
        "the selected file is this host's library and nothing else"
    );
    assert!(
        !staged.root().join(&other_entry).exists(),
        "no other target's file is staged, and selection did not need one"
    );

    let error = manifest
        .select_library(&staged, &other)
        .expect_err("this staged bundle does not carry another target's library");
    assert_eq!(
        error,
        ManifestError::LibraryNotStaged {
            target: other.clone(),
            path: staged.root().join(&other_entry),
        },
        "the refusal names the target and the declaration that is not there"
    );
}

#[test]
fn selection_refuses_a_bundle_that_carries_no_library_for_this_host() {
    let target = host_target();
    let other = other_target();

    // Nothing at all is declared for this host.
    let elsewhere = format!("lib/{other}/libother.dylib");
    let undeclared = staged(
        &entries(
            &target,
            Some(&hand_written_with(
                NAME,
                &format!("{other} = \"{elsewhere}\""),
            )),
            &[],
        ),
        &target,
    );
    let manifest = Manifest::read(&undeclared, NAME).expect("the manifest loads");
    assert_eq!(
        manifest.select_library(&undeclared, &target),
        Err(ManifestError::NoTargetLibrary {
            target: target.clone()
        }),
        "a manifest that declares no library for this host declares none"
    );

    // This host is declared, and the file the declaration names is not the one
    // the bundle carries.
    let declared = format!("lib/{target}/not-the-library.so");
    let wrong = staged(
        &entries(
            &target,
            Some(&hand_written_with(
                NAME,
                &format!("{target} = \"{declared}\""),
            )),
            &[],
        ),
        &target,
    );
    let manifest = Manifest::read(&wrong, NAME).expect("the manifest loads");
    assert_eq!(
        manifest.select_library(&wrong, &target),
        Err(ManifestError::LibraryNotStaged {
            target: target.clone(),
            path: wrong.root().join(&declared),
        }),
        "a declaration the bundle does not carry is refused, naming it"
    );
}

#[test]
fn a_library_mapped_from_a_bundle_yields_a_table_the_raw_abi_checks_accept() {
    let target = host_target();
    let staged = staged_with(Some(&hand_written(NAME)));
    let manifest = Manifest::read(&staged, NAME).expect("the bundle's manifest loads");
    let selected = manifest
        .select_library(&staged, &target)
        .expect("this host's declaration names the library the bundle carries");

    let loaded =
        open(&selected, &manifest).expect("the bundle's library maps through its manifest");
    assert_eq!(
        loaded.abi(),
        (METIS_ABI_MAJOR, METIS_ABI_MINOR),
        "the mapped table reports this build's ABI"
    );

    // The landing's raw checks accept the mapped library's table, which is what
    // makes the table usable by everything that comes after this step.
    let table = loaded.table();
    // SAFETY: the table is the loaded library's own and is live for as long as
    // the library is.
    let read = unsafe { metis_read_table(table) }.expect("the landed gate reads the mapped table");
    assert!(
        metis_plugin_table_has_init_plugin(table.size),
        "the table's size covers the init_plugin entry this build needs"
    );
    assert_eq!(read.abi_major, METIS_ABI_MAJOR);
    assert_eq!(read.abi_minor, METIS_ABI_MINOR);
}

#[test]
fn the_mapping_step_refuses_a_manifest_built_for_another_abi_major() {
    let target = host_target();
    let text = manifest_needing(NAME, METIS_ABI_MAJOR + 1, METIS_ABI_MINOR);
    let staged = staged_with(Some(&text));
    let manifest = Manifest::read(&staged, NAME).expect("the manifest loads");
    let library = manifest
        .select_library(&staged, &target)
        .expect("the library the manifest declares is staged");

    assert_eq!(
        refusal(
            open(&library, &manifest),
            "a plugin built for another major is refused"
        ),
        CallError::Version(MetisAbiError::MajorMismatch {
            expected: METIS_ABI_MAJOR,
            found: METIS_ABI_MAJOR + 1,
        }),
        "a plugin built for another major is refused with the landed error"
    );
}

#[test]
fn the_mapping_step_answers_this_builds_own_minor_demand_before_it_reads_a_library() {
    // A manifest that needs a minor this build does not know is refused, and
    // the path is deliberately not a library at all: the demand is decided
    // before anything is opened, so the refusal is the version one and not the
    // loader's.
    let text = manifest_needing(NAME, METIS_ABI_MAJOR, METIS_ABI_MINOR + 1);
    let staged = staged_with(Some(&text));
    let manifest = Manifest::read(&staged, NAME).expect("the manifest loads");
    assert_eq!(manifest.min_abi_minor(), METIS_ABI_MINOR + 1);
    let not_a_library = staged.root().join("no-library-here");
    assert_eq!(
        refusal(
            open(&not_a_library, &manifest),
            "this build is older than the plugin needs, and it says so"
        ),
        CallError::Version(MetisAbiError::MinorMismatch {
            known: METIS_ABI_MINOR,
            found: METIS_ABI_MINOR + 1,
        }),
        "the demand is decided before the library is opened"
    );

    // The other direction is served: a plugin that needs an older minor is read
    // by this build, which is newer than it asks for.
    let target = host_target();
    let text = manifest_needing(NAME, METIS_ABI_MAJOR, METIS_ABI_MINOR - 1);
    let staged = staged_with(Some(&text));
    let manifest = Manifest::read(&staged, NAME).expect("the manifest loads");
    assert_eq!(
        manifest.min_abi_minor(),
        METIS_ABI_MINOR - 1,
        "this test means the manifest to need the older minor"
    );
    let library = manifest
        .select_library(&staged, &target)
        .expect("the library the manifest declares is staged");
    open(&library, &manifest).expect("a plugin that needs an older minor is served");
}

#[test]
fn the_mapping_step_answers_the_landed_typed_errors_for_a_table_this_build_did_not_write() {
    let manifest = Manifest::parse(&hand_written(NAME)).expect("the manifest is the format");
    let prefix = size_of::<MetisTablePrefix>();
    let slot = size_of::<*const c_void>();
    let layout = prefix + 8 * slot;
    let one_entry = prefix + slot;

    // A table that is both stale and too short answers the version, so the
    // version was decided before any entry's presence was read.
    let stale = patched_table(one_entry, METIS_ABI_MAJOR, METIS_ABI_MINOR - 1, ENTRY);
    assert_eq!(
        refusal(open(&stale, &manifest), "a stale minor is refused"),
        CallError::Version(MetisAbiError::MinorMismatch {
            known: METIS_ABI_MINOR,
            found: METIS_ABI_MINOR - 1,
        }),
        "an older minor is refused before any entry is read"
    );

    // A table that stops before an entry this build needs is refused by name.
    let short = patched_table(one_entry, METIS_ABI_MAJOR, METIS_ABI_MINOR, ENTRY);
    assert_eq!(
        refusal(open(&short, &manifest), "a table too short is refused"),
        CallError::Version(MetisAbiError::EntryMissing {
            entry: "destroy_instance",
        }),
        "the first entry the reported size does not cover is the one named"
    );

    // Another major.
    let other_major = patched_table(layout, METIS_ABI_MAJOR + 1, METIS_ABI_MINOR, ENTRY);
    assert_eq!(
        refusal(open(&other_major, &manifest), "another major is refused"),
        CallError::Version(MetisAbiError::MajorMismatch {
            expected: METIS_ABI_MAJOR,
            found: METIS_ABI_MAJOR + 1,
        })
    );

    // A newer minor whose table is longer than this build's layout is read: the
    // additive tolerance is the landed reader's, and this step only reuses it.
    let newer = patched_table(layout + slot, METIS_ABI_MAJOR, METIS_ABI_MINOR + 1, ENTRY);
    let loaded = open(&newer, &manifest).expect("a newer minor's table is read");
    assert_eq!(
        loaded.abi(),
        (METIS_ABI_MAJOR, METIS_ABI_MINOR + 1),
        "the table reports the newer minor this build did not write"
    );
    assert_eq!(
        loaded.table().size,
        layout + slot,
        "the reported size is the plugin's own, which is longer than this build's layout"
    );
}

#[test]
fn the_mapping_step_refuses_a_library_with_no_entry_symbol_for_this_major() {
    let manifest = Manifest::parse(&hand_written(NAME)).expect("the manifest is the format");
    let layout = size_of::<MetisTablePrefix>() + 8 * size_of::<*const c_void>();
    let other_major = patched_table(layout, METIS_ABI_MAJOR, METIS_ABI_MINOR, OTHER_ENTRY);

    let error = refusal(
        open(&other_major, &manifest),
        "a library built for another major has no entry this host can resolve",
    );
    match error {
        CallError::Load(detail) => assert!(
            detail.contains(&other_major.display().to_string()),
            "the refusal says which library could not be loaded: {detail}"
        ),
        other => panic!("a missing entry symbol is a load refusal, and this one is {other:?}"),
    }
}

#[test]
fn the_contracts_documented_manifest_parses() {
    let contract = std::fs::read_to_string(repository_root().join("docs/plugin-contract.md"))
        .expect("the contract is in the tree this build is tested from");
    let example = documented_manifest(&contract);

    let manifest = Manifest::parse(&example)
        .expect("the example the contract shows is a manifest this build reads");
    assert_eq!(
        manifest.abi_check(),
        Ok(()),
        "the documented example is one this build serves"
    );
    assert_eq!(manifest.name(), "dev.metis.example-plugin");
    assert_eq!(manifest.version(), "1.0.0");
    assert_eq!(
        manifest.libraries().keys().collect::<Vec<_>>(),
        vec!["linux-x86_64"],
        "the example declares the library path an author writes per target"
    );
    assert_eq!(manifest.capabilities().len(), 1);
    assert_eq!(manifest.capabilities()[0].name, "tool.echo");
    assert_eq!(manifest.capabilities()[0].kind, "tool");
    assert_eq!(manifest.capabilities()[0].version, 1);
    assert_eq!(
        manifest.capabilities()[0].request_schema,
        json!({
            "type": "object",
            "required": ["echo"],
            "properties": {"echo": {"type": "string"}},
        })
    );
    assert_eq!(
        manifest.config_schema(),
        &json!({
            "type": "object",
            "properties": {"instance": {"type": "string"}},
        })
    );
}

/// The manifest example the contract shows, which is the format an author
/// writes by hand.
///
/// It is read out of the contract rather than restated here, so the documented
/// format and the format this build reads cannot drift: a change to one that is
/// not made to the other fails this test.
fn documented_manifest(contract: &str) -> String {
    let section = contract
        .split_once("### 17.9 The manifest")
        .expect("the contract documents the manifest's fields in their own section")
        .1;
    let block = section
        .split_once("```toml\n")
        .expect("the manifest's section shows one TOML example")
        .1;
    block
        .split_once("\n```")
        .expect("the example is a fenced block")
        .0
        .to_string()
}
