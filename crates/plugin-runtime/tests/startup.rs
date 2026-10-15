//! Start-up policy, tested at its one public seam: a directory of bundles in,
//! a report of what started and what start-up disabled out, or the outcome that
//! stopped start-up.
//!
//! Every bundle here is built inside the test the way a producer builds one:
//! the fixture cdylib's own bytes beside a manifest this test writes by hand,
//! signed by a chain whose leaf carries the plugin's name, written as a file in
//! a directory this test owns. A bundle's file name is the only name the policy
//! knows before a manifest is read, so each test chooses the file name and the
//! manifest the bundle declares, and the two agree unless the test is about the
//! case where no manifest is readable.

mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::time::{SystemTime, UNIX_EPOCH};

use plugin_contract::abi::{METIS_ABI_MAJOR, MetisAbiError};
use plugin_runtime::CallError;
use plugin_runtime::bundle::{StageError, TrustStore};
use plugin_runtime::load::LoadError;
use plugin_runtime::manifest::ManifestError;
use plugin_runtime::startup::{DisabledPlugin, StartupReport, StartupStop, start_plugins};
use serde_json::{Map, Value, json};
use support::block_on;
use support::bundle;
use support::signer::{self, TestChain};

/// The manifest's name at the archive root, as the contract fixes it.
const MANIFEST: &str = "manifest.toml";

/// The file name suffix a plugin directory's bundles carry.
const SUFFIX: &str = ".metisbnd";

/// The seeds of the chain every bundle is signed by, unless a test wants one
/// this pass does not trust.
const SEEDS: [u8; 3] = [1, 2, 3];

/// The seeds of a chain no trust store here accepts, for the one outcome that
/// stops start-up because of the signature.
const UNTRUSTED_SEEDS: [u8; 3] = [11, 12, 13];

/// How many directories this process has made, so two tests cannot share one.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// One test's plugin directory, removed when it is dropped.
struct Directory {
    path: PathBuf,
}

impl Directory {
    /// Creates an empty plugin directory no other test shares.
    fn new() -> Directory {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "metis-startup-test-{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, SeqCst)
        ));
        std::fs::create_dir(&path)
            .unwrap_or_else(|error| panic!("{} is creatable: {error}", path.display()));
        Directory { path }
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        // The directory is this value's own, and a removal that fails is not
        // worth panicking in a drop.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// One test's plugin directory and everything the policy needs to read it: the
/// bundles written into it, the configurations resolved for them, and the trust
/// store that accepts their anchors.
struct Plugins {
    dir: Directory,
    trust: TrustStore,
    config: Map<String, Value>,
}

impl Plugins {
    /// A directory whose trust store accepts the chain every bundle here is
    /// signed by, and no other.
    fn new() -> Plugins {
        Plugins {
            dir: Directory::new(),
            trust: TrustStore::empty().with_root(anchor(SEEDS)),
            config: Map::new(),
        }
    }

    /// The directory the policy reads.
    fn path(&self) -> &Path {
        &self.dir.path
    }

    /// The configurations resolved for this directory's plugins.
    fn configs(&self) -> &Map<String, Value> {
        &self.config
    }

    /// Resolves the fixture's own configuration for a bundle file's stem: the
    /// configuration names the instance the host context names, which is what
    /// the fixture's constructor checks.
    fn configure(&mut self, file: &str) {
        self.config
            .insert(file.to_string(), json!({ "instance": file }));
    }

    /// Writes one bundle as `<file>.metisbnd`, signed by a chain whose leaf, and
    /// whose manifest, carry `name`.
    ///
    /// The two names are given separately so a test can tell the name a bundle
    /// is known by before its manifest is read from the name the manifest
    /// declares.
    fn write(&mut self, file: &str, name: &str, bundle: &Bundle) -> PathBuf {
        self.write_signed(file, name, bundle, SEEDS)
    }

    /// Writes one bundle signed by a chain this pass does not trust.
    fn write_untrusted(&mut self, file: &str, name: &str, bundle: &Bundle) -> PathBuf {
        self.write_signed(file, name, bundle, UNTRUSTED_SEEDS)
    }

    /// Writes one bundle signed by the chain built from `seeds`, carrying
    /// `bundle`'s entries.
    fn write_signed(&mut self, file: &str, name: &str, bundle: &Bundle, seeds: [u8; 3]) -> PathBuf {
        let chain = signed_by(name, seeds);
        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        if let Some(manifest) = &bundle.manifest {
            entries.push((MANIFEST.to_string(), manifest.as_bytes().to_vec()));
        }
        if let Some((file, bytes)) = &bundle.library {
            let target = support::bundle::host_target();
            entries.push((support::bundle::library_path(&target, file), bytes.clone()));
        }
        entries.extend(bundle.extra.iter().cloned());
        let bytes = signer::signed_bundle(&chain, &support::bundle::archive(&entries));
        let path = self.dir.path.join(format!("{file}{SUFFIX}"));
        std::fs::write(&path, &bytes)
            .unwrap_or_else(|error| panic!("{} writes: {error}", path.display()));
        path
    }

    /// Starts every bundle this directory holds, at the seam under test.
    fn start(&self) -> Result<StartupReport, StartupStop> {
        block_on(start_plugins(self.path(), self.configs(), &self.trust))
    }
}

/// What one bundle carries.
struct Bundle {
    /// The manifest at the archive root, or none at all.
    manifest: Option<String>,
    /// This host's library: its file name inside the target directory, and its
    /// bytes.
    library: Option<(String, Vec<u8>)>,
    /// Entries beyond the manifest and this host's library.
    extra: Vec<(String, Vec<u8>)>,
}

impl Bundle {
    /// A bundle carrying `manifest` when the test writes one, and the fixture's
    /// own library for this host.
    fn new(manifest: Option<String>) -> Bundle {
        Bundle {
            manifest,
            library: Some((
                support::bundle::library_file_name(),
                support::bundle::library_bytes(),
            )),
            extra: Vec::new(),
        }
    }

    /// The same bundle with no library entry at all.
    fn without_library(mut self) -> Bundle {
        self.library = None;
        self
    }

    /// The same bundle with one more archive entry.
    fn with_entry(mut self, name: &str, bytes: &[u8]) -> Bundle {
        self.extra.push((name.to_string(), bytes.to_vec()));
        self
    }
}

/// The chain whose leaf carries `name`, built from `seeds`.
fn signed_by(name: &str, seeds: [u8; 3]) -> TestChain {
    let mut specs = signer::standard_specs(3);
    specs[0].common_name = name.to_string();
    TestChain::build(&specs, &seeds)
}

/// The public key of a chain's anchor, which is its last link's key.
fn anchor(seeds: [u8; 3]) -> [u8; 32] {
    signer::public_key(&signer::key(seeds[2]))
}

/// The manifest an author writes by hand: the fixture's library for this host,
/// the capability names it is given, and the ABI this build speaks.
fn manifest(name: &str, capabilities: &[&str]) -> String {
    manifest_declaring(
        name,
        capabilities,
        &support::bundle::host_target(),
        &support::bundle::library_file_name(),
        1,
        2,
    )
}

/// The same manifest, declaring another target, another library file and
/// another ABI demand.
fn manifest_declaring(
    name: &str,
    capabilities: &[&str],
    target: &str,
    library_file: &str,
    abi_major: u32,
    min_abi_minor: u32,
) -> String {
    // The format carries a capability list as a field, so a manifest that
    // declares none still has to carry the empty one. It has to sit before the
    // first table, because a key written after one belongs to that table.
    let declared = if capabilities.is_empty() {
        "\ncapabilities = []".to_string()
    } else {
        capabilities
            .iter()
            .map(|capability| {
                format!(
                    "\n[[capabilities]]\nname = \"{capability}\"\nkind = \"tool\"\nversion = 1\n\
                     request_schema = {{ type = \"object\" }}\n\
                     response_schema = {{ type = \"object\" }}\n"
                )
            })
            .collect()
    };
    format!(
        r#"name = "{name}"
version = "1.0.0"
abi_major = {abi_major}
min_abi_minor = {min_abi_minor}{declared}

[libraries]
{target} = "lib/{target}/{library_file}"

[config_schema]
type = "object"
"#
    )
}

/// The outcome that stopped start-up.
///
/// A started report is not `Debug`, so the tests read the outcome out by hand
/// rather than through `expect_err`.
fn refusal(result: Result<StartupReport, StartupStop>, what: &str) -> StartupStop {
    match result {
        Err(stop) => stop,
        Ok(report) => panic!(
            "{what}: {} serving and {} disabled started instead",
            report.serving().len(),
            report.disabled().len()
        ),
    }
}

#[test]
fn a_trust_failure_stops_start_up() {
    let mut plugins = Plugins::new();
    plugins.write_untrusted(
        "forged",
        "forged",
        &Bundle::new(Some(manifest("forged", &[]))),
    );

    let stop = refusal(
        plugins.start(),
        "a bundle no configured root signed cannot start",
    );

    match stop.error() {
        LoadError::Trust(failure) => assert!(
            failure.is_trust_failure(),
            "an unknown root is a trust failure: {failure}"
        ),
        other => panic!("a signature this host does not trust stops start-up, and this is {other}"),
    }
    assert!(
        stop.path().ends_with("forged.metisbnd"),
        "the stop names the bundle that caused it: {}",
        stop.path().display()
    );
}

#[test]
fn an_absent_manifest_stops_start_up() {
    let mut plugins = Plugins::new();
    plugins.write("nameless", "nameless", &Bundle::new(None));

    let stop = refusal(
        plugins.start(),
        "a bundle with no manifest at its root cannot start",
    );

    match stop.error() {
        LoadError::Manifest(ManifestError::Absent { path }) => assert!(
            path.ends_with(MANIFEST),
            "the refusal names where the manifest would have been: {}",
            path.display()
        ),
        other => panic!("an absent manifest stops start-up, and this outcome is {other}"),
    }
}

#[test]
fn an_invalid_manifest_stops_start_up() {
    let mut plugins = Plugins::new();
    plugins.write(
        "unreadable",
        "unreadable",
        &Bundle::new(Some("this is not the manifest format".to_string())),
    );

    let stop = refusal(
        plugins.start(),
        "a bundle whose manifest is not the format cannot start",
    );

    match stop.error() {
        LoadError::Manifest(ManifestError::Invalid { detail }) => assert!(
            !detail.is_empty(),
            "the refusal names what is wrong with the text"
        ),
        other => panic!("an invalid manifest stops start-up, and this outcome is {other}"),
    }
}

#[test]
fn a_directory_that_cannot_be_listed_stops_start_up() {
    let plugins = Plugins::new();
    let absent = plugins.path().join("no-such-directory");

    let stop = match block_on(start_plugins(
        &absent,
        plugins.configs(),
        &TrustStore::empty(),
    )) {
        Err(stop) => stop,
        Ok(report) => panic!(
            "a plugin directory that is not there cannot start: {} serving",
            report.serving().len()
        ),
    };

    match stop.error() {
        LoadError::Unreadable { path, .. } => assert_eq!(
            path.as_path(),
            absent.as_path(),
            "the refusal names the directory that was read"
        ),
        other => panic!("a directory that does not read stops start-up, and this is {other}"),
    }
}

/// The one plugin a pass disabled, and the one that stopped it when a test
/// expected a stop.
///
/// A started report is not `Debug`, so the tests read a disabled plugin out by
/// hand rather than through `expect_err`.
fn disabled_plugin<'a>(report: &'a StartupReport, what: &str) -> &'a DisabledPlugin {
    match report.disabled() {
        [plugin] => plugin,
        plugins => panic!("{what}: {} plugins were disabled instead", plugins.len()),
    }
}

/// The plugins that started, as a pass with no refusal at all.
fn report(plugins: &Plugins, what: &str) -> StartupReport {
    match plugins.start() {
        Ok(report) => report,
        Err(stop) => panic!("{what}: {stop}"),
    }
}

#[test]
fn an_extraction_refusal_disables_that_one_plugin() {
    let mut plugins = Plugins::new();
    plugins.write(
        "escaping",
        "escaping",
        &Bundle::new(Some(manifest("escaping", &[]))).with_entry("../escape", b"bytes"),
    );

    let report = report(&plugins, "one hostile archive does not stop start-up");
    let plugin = disabled_plugin(&report, "one bundle was disabled");

    match plugin.reason() {
        LoadError::Stage(StageError::ParentEntryName { name }) => assert_eq!(
            name, "../escape",
            "the refusal names the entry that would escape the staging root"
        ),
        other => panic!("an archive this build refuses disables one plugin, and this is {other}"),
    }
    assert!(
        report.serving().is_empty(),
        "the refused bundle did not start"
    );
}

#[test]
fn a_missing_target_library_disables_that_one_plugin() {
    let mut plugins = Plugins::new();
    plugins.write(
        "libraryless",
        "libraryless",
        &Bundle::new(Some(manifest("libraryless", &[]))).without_library(),
    );

    let report = report(
        &plugins,
        "an archive with no library does not stop start-up",
    );
    let plugin = disabled_plugin(&report, "one bundle was disabled");

    match plugin.reason() {
        LoadError::Stage(StageError::MissingTargetLibrary { target }) => assert_eq!(
            target,
            &support::bundle::host_target(),
            "the refusal names the target this host asked for"
        ),
        other => {
            panic!(
                "an archive with no library for this host disables one plugin, and this is {other}"
            )
        }
    }
}

#[test]
fn a_manifest_that_declares_no_library_for_this_host_disables_that_one_plugin() {
    let mut plugins = Plugins::new();
    plugins.write(
        "declared-elsewhere",
        "declared-elsewhere",
        &Bundle::new(Some(manifest_declaring(
            "declared-elsewhere",
            &[],
            "macos-aarch64",
            "libdeclared.so",
            1,
            2,
        ))),
    );

    let report = report(
        &plugins,
        "a manifest for another host does not stop start-up",
    );
    let plugin = disabled_plugin(&report, "one bundle was disabled");

    match plugin.reason() {
        LoadError::Manifest(ManifestError::NoTargetLibrary { target }) => assert_eq!(
            target,
            &support::bundle::host_target(),
            "the refusal names the target this host asked for"
        ),
        other => panic!(
            "a manifest that declares nothing for this host disables one plugin, and this is {other}"
        ),
    }
}

#[test]
fn an_abi_mismatch_disables_that_one_plugin() {
    let mut plugins = Plugins::new();
    plugins.write(
        "other-major",
        "other-major",
        &Bundle::new(Some(manifest_declaring(
            "other-major",
            &[],
            &support::bundle::host_target(),
            &support::bundle::library_file_name(),
            METIS_ABI_MAJOR + 1,
            2,
        ))),
    );

    let report = report(
        &plugins,
        "a plugin built for another major does not stop start-up",
    );
    let plugin = disabled_plugin(&report, "one bundle was disabled");

    match plugin.reason() {
        LoadError::Call(CallError::Version(MetisAbiError::MajorMismatch { expected, found })) => {
            assert_eq!(*expected, METIS_ABI_MAJOR);
            assert_eq!(*found, METIS_ABI_MAJOR + 1);
        }
        other => panic!("an ABI mismatch disables one plugin, and this is {other}"),
    }
    assert!(
        report.serving().is_empty(),
        "the refused bundle did not start"
    );
}

#[test]
fn a_manifest_that_disagrees_with_the_library_disables_that_one_plugin() {
    let mut plugins = Plugins::new();
    plugins.configure("disagreeing");
    plugins.write(
        "disagreeing",
        "disagreeing",
        &Bundle::new(Some(manifest(
            "disagreeing",
            &[bundle::CAPABILITIES[0], "tool.absent"],
        ))),
    );

    let report = report(
        &plugins,
        "a manifest the library disagrees with does not stop start-up",
    );
    let plugin = disabled_plugin(&report, "one bundle was disabled");

    match plugin.reason() {
        LoadError::CapabilityMismatch { capability, .. } => assert_eq!(
            capability, "tool.absent",
            "the refusal names the capability that differed"
        ),
        other => {
            panic!("a manifest the library disagrees with disables one plugin, and this is {other}")
        }
    }
}

#[test]
fn a_disabled_plugin_is_listable_with_its_name_its_reason_and_its_capabilities() {
    let mut plugins = Plugins::new();
    // The file's name and the manifest's name differ, so the name the report
    // lists is the one the manifest declared: the manifest was read.
    plugins.write(
        "declared-elsewhere",
        "other-host-plugin",
        &Bundle::new(Some(manifest_declaring(
            "other-host-plugin",
            &["tool.echo", "tool.pending"],
            "macos-aarch64",
            "libdeclared.so",
            1,
            2,
        ))),
    );

    let report = report(
        &plugins,
        "a manifest for another host does not stop start-up",
    );
    let plugin = disabled_plugin(&report, "one bundle was disabled");

    assert_eq!(
        plugin.name(),
        "other-host-plugin",
        "the placeholder carries the name the manifest declares"
    );
    assert!(
        matches!(plugin.reason(), LoadError::Manifest(_)),
        "the placeholder carries the refusal that disabled it: {}",
        plugin.reason()
    );
    let names: Vec<&str> = plugin
        .capabilities()
        .iter()
        .map(|capability| capability.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["tool.echo", "tool.pending"],
        "the placeholder lists the capabilities the manifest declared, in its order"
    );
}
