//! The sample bundle: the C fixture's own library, a hand-written manifest, and
//! the test key's signature over both.
//!
//! This is the packaging step the end-to-end gate runs. It builds the C fixture
//! with the same script the ABI harness runs, so a stale library cannot be
//! packaged, then writes the two entries the contract fixes, the root manifest
//! and this host's target library, signs them the way the verifier reads them,
//! and hands back a file the public load entry can read.
//!
//! Three rules from the landed tickets shape what the archive carries. Every
//! entry declares mode `0600`, because the extractor refuses any other declared
//! mode and the `zip` writer writes `0o644` when a producer chooses none; the
//! archive holds exactly one file per target directory and no directory entry,
//! because the extractor refuses a directory entry; and the target directory is
//! spelled `<os>-<arch>` exactly as the extracting host derives it, because
//! extraction matches that string literally.
//!
//! The manifest is a literal in this file rather than a serialisation of any
//! type in this tree. That is the point of the gate: the C fixture is the proof
//! that the format is writable without a generator, so the manifest an author
//! writes by hand is what gets packaged.
//!
//! The signing key is the shared test signer's, from a fixed seed, and it is
//! installed as a root only where a test says so. No shipped build carries it,
//! and the daemon's compiled-in root set stays empty.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use plugin_runtime::bundle::TrustStore;

use super::bundle::{self, BundleFile};
use super::signer::{self, TestChain};
use super::{block_on, repository_root};

/// The name the sample bundle declares, which is also the name the bundle's
/// leaf-authorising certificate carries and the name the instance is registered
/// under. It is the C fixture's own name, which is what its initialisation
/// declaration checks the configuration against.
pub const NAME: &str = "cplugin";

/// The file the C fixture is compiled to, as `scripts/build-c-plugin.sh` writes
/// it, which is the same path the ABI harness loads.
pub const LIBRARY_FILE: &str = "libcplugin.so";

/// The capability names the C fixture declares, in the order it declares them.
///
/// They are literals rather than a list read out of the library, because the
/// manifest an author writes is hand-written: a list read from the library
/// would make the load's cross-check vacuous.
pub const CAPABILITIES: [&str; 4] = ["c.ready", "c.timer", "c.cancel", "c.drop_in_flight"];

/// The capability the gate dispatches: ready on its first poll, and it emits one
/// event, so the gate reads both a response and an event through the sink.
pub const READY_CAPABILITY: &str = "c.ready";

/// The response body the ready capability answers with.
pub const READY_RESPONSE: &str = r#"{"echo":"c.ready","bytes":5}"#;

/// The topic of the event the ready capability emits.
pub const READY_TOPIC: &str = "cplugin.ready";

/// The body of the event the ready capability emits.
pub const READY_EVENT_BODY: &str = r#"{"ready":true}"#;

/// The build lock and the built library, so several tests in one binary compile
/// the C fixture once and never race on the same output files.
static BUILDING: Mutex<()> = Mutex::new(());
static LIBRARY: OnceLock<PathBuf> = OnceLock::new();

/// The C fixture's library, compiled from this tree.
///
/// It is rebuilt once per process by the script the ABI harness runs, so a
/// bundle is never packaged from a library belonging to an older source tree.
pub fn library() -> PathBuf {
    let _building = BUILDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    LIBRARY
        .get_or_init(|| {
            let script = repository_root().join("scripts/build-c-plugin.sh");
            let built = Command::new("bash")
                .arg(&script)
                .output()
                .unwrap_or_else(|error| panic!("{} runs: {error}", script.display()));
            assert!(
                built.status.success(),
                "the C fixture must build before it is packaged: {}",
                String::from_utf8_lossy(&built.stderr)
            );
            let library = repository_root().join("target/c-plugin").join(LIBRARY_FILE);
            assert!(
                library.is_file(),
                "the build script wrote {}",
                library.display()
            );
            library
        })
        .clone()
}

/// The bytes this host's library is carried as, which is the C fixture's own.
pub fn library_bytes() -> Vec<u8> {
    std::fs::read(library()).expect("the C fixture library this test built reads")
}

/// The chain the sample bundle is signed with.
///
/// The leaf-authorising certificate's common name is the name the manifest
/// declares, because the load compares the two; the seeds are the shared test
/// signer's fixed ones, so the chain is reproducible and no key ships.
pub fn chain() -> TestChain {
    let mut specs = signer::standard_specs(3);
    specs[0].common_name = NAME.to_string();
    TestChain::build(&specs, &[1, 2, 3])
}

/// A chain whose anchor is not the configured root, from other fixed seeds.
///
/// It signs the same manifest and the same library, so the only thing a bundle
/// it signs has wrong is the key that signed it.
pub fn untrusted_chain() -> TestChain {
    let mut specs = signer::standard_specs(3);
    specs[0].common_name = NAME.to_string();
    TestChain::build(&specs, &[7, 8, 9])
}

/// The trust store that accepts the sample chain's anchor and nothing else.
pub fn trust() -> TrustStore {
    bundle::trust_for(&chain())
}

/// The hand-written manifest, declaring exactly the capability names it is
/// given and this host's library.
///
/// The text is a literal, not a serialisation of any type in this tree: the
/// format belongs to the author. A test passes a list the library does not
/// declare to make the two disagree on purpose.
pub fn manifest(names: &[&str]) -> String {
    let target = bundle::host_target();
    let library = bundle::library_path(&target, LIBRARY_FILE);
    let capabilities: String = names
        .iter()
        .map(|name| {
            format!(
                "\n[[capabilities]]\nname = \"{name}\"\nkind = \"tool\"\nversion = 1\n\
                 request_schema = {{ type = \"object\" }}\n\
                 response_schema = {{ type = \"object\" }}\n"
            )
        })
        .collect();
    format!(
        r#"# A hand-written manifest: the C fixture is the proof that packaging a
# plugin needs no generator. The host trusts this list for discovery, and the
# library's own declaration is the cross-check.
name = "{NAME}"
version = "1.0.0"
abi_major = 1
min_abi_minor = 2

[libraries]
{target} = "{library}"
{capabilities}
[config_schema]
type = "object"
properties = {{ instance = {{ type = "string" }} }}
"#
    )
}

/// The sample bundle's bytes: the C fixture's library and `manifest`, both
/// inside one archive, signed by `chain` the way the verifier reads them.
///
/// The archive is assembled here rather than through the shared helper because
/// that helper carries the Rust fixture's library; the two entries and the mode
/// every entry declares are the contract's, and the shared `archive` writer is
/// what writes them.
pub fn bytes_signed_by(chain: &TestChain, manifest: &str) -> Vec<u8> {
    let target = bundle::host_target();
    let entries = vec![
        (bundle::MANIFEST.to_string(), manifest.as_bytes().to_vec()),
        (bundle::library_path(&target, LIBRARY_FILE), library_bytes()),
    ];
    signer::signed_bundle(chain, &bundle::archive(&entries))
}

/// The packaging step: the sample bundle's bytes, signed by the test chain.
pub fn bytes(manifest: &str) -> Vec<u8> {
    bytes_signed_by(&chain(), manifest)
}

/// The packaging step: the signed bundle written to a file the load entry reads,
/// beside the trust store that accepts it, and removed when the guard drops.
pub fn pack(manifest: &str) -> (BundleFile, TrustStore) {
    (BundleFile::write(&bytes(manifest)), trust())
}

/// The configuration a host resolved for the sample plugin.
///
/// The C fixture declares the one object it accepts, so a gate that handed it
/// anything else would be testing the refusal class rather than the gate: the
/// bytes are the fixture's own declaration, which the plugin checks without a
/// JSON reader.
pub fn config() -> serde_json::Value {
    serde_json::json!({ "instance": NAME })
}

/// The host context, as contract 18's named part, naming the same instance.
pub fn host_context() -> serde_json::Value {
    serde_json::json!({ "instance_name": NAME, "capability_version": 1 })
}

/// Loads one sample bundle through the public entry, or panics with the refusal.
pub fn load(
    host: &plugin_runtime::Host,
    path: &std::path::Path,
    trust: &TrustStore,
) -> plugin_runtime::load::Serving {
    match block_on(plugin_runtime::load::load_bundle(
        host,
        path,
        &config(),
        &host_context(),
        trust,
    )) {
        Ok(serving) => serving,
        Err(error) => panic!("the sample bundle loads, and this one refused: {error}"),
    }
}
