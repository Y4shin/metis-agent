//! The load entry, tested at its one public seam: a bundle path in, a
//! registered, serving plugin out, or a typed refusal that names the first
//! thing that was wrong.
//!
//! Every bundle here is built inside the test, the way a producer builds one:
//! the fixture cdylib's own bytes are written into an archive beside a manifest
//! this file writes by hand, then signed. So the entry reads a real signed
//! bundle, and the manifest under test is the format an author writes rather
//! than a format only a generator could produce.
//!
//! The configuration and the host context are built here as plain JSON, because
//! their value types live behind `plugin-contract`'s `authoring` feature, which
//! the runtime keeps off: contract 18's two names are written by hand and the
//! plugin decodes them into its own types.

mod support;

use plugin_contract::abi::MetisStatus;
use plugin_fixture::DEFECTIVE_INITIALISATION;
use plugin_runtime::Host;
use plugin_runtime::bundle::TrustStore;
use plugin_runtime::load::{LoadError, Serving, load_bundle};
use plugin_runtime::manifest::ManifestError;
use serde_json::{Value, json};
use support::block_on;
use support::bundle;

/// The capability names the fixture declares, in the order it declares them.
///
/// They are written here as literals because the manifest an author writes is
/// hand-written: a list read out of the library would make the cross-check
/// vacuous.
const CAPABILITIES: [&str; 10] = [
    "tool.echo",
    "tool.pending",
    "tool.cancellable",
    "tool.stubborn",
    "tool.panic",
    "tool.panic_drop",
    "tool.blocking",
    "tool.deferred",
    "tool.wake_deferred",
    "tool.newer",
];

/// The name the test chain's leaf-authorising certificate carries, which is the
/// name the manifest declares and the name the instance is registered under.
const NAME: &str = bundle::INSTANCE_NAME;

/// The configuration the host resolved, which the fixture declares as one
/// object naming the instance it was resolved for.
fn config(instance: &str) -> Value {
    json!({ "instance": instance })
}

/// The host context, as contract 18's named part, written by hand because its
/// value type lives behind the contract's authoring gate.
fn host_context(instance: &str) -> Value {
    json!({ "instance_name": instance, "capability_version": 1 })
}

/// The manifest an author writes by hand, declaring exactly the capability
/// names it is given and this host's library.
fn hand_written<S: AsRef<str>>(names: &[S]) -> String {
    let target = bundle::host_target();
    let capabilities: String = names
        .iter()
        .map(|name| {
            let name = name.as_ref();
            format!(
                "\n[[capabilities]]\nname = \"{name}\"\nkind = \"tool\"\nversion = 1\n\
                 request_schema = {{ type = \"object\" }}\n\
                 response_schema = {{ type = \"object\" }}\n"
            )
        })
        .collect();
    format!(
        r#"# A hand-written manifest: the proof that packaging a plugin needs no
# generator. The host trusts this list for discovery, and the library's own
# declaration is the cross-check.
name = "{NAME}"
version = "1.0.0"
abi_major = 1
min_abi_minor = 2

[libraries]
{target} = "{library}"
{capabilities}
[config_schema]
type = "object"
"#,
        library = bundle::library_path(&target, &bundle::library_file_name()),
    )
}

/// The refusal a load earned.
///
/// A loaded plugin is not `Debug`, so the tests read the refusal out by hand
/// rather than through `expect_err`.
fn refusal(result: Result<Serving, LoadError>, what: &str) -> LoadError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{what}"),
    }
}

/// The configuration and the host context of one load, matched: the
/// configuration names the instance the host context names.
fn matched(instance: &str) -> (Value, Value) {
    (config(instance), host_context(instance))
}

#[test]
fn a_bundle_loads_into_a_registered_instance_with_its_declared_capabilities() {
    let host = Host::new();
    let (file, trust) = bundle::bundle_file(Some(&hand_written(&CAPABILITIES)));

    let serving = block_on(load_bundle(
        &host,
        file.path(),
        &config("loaded"),
        &host_context("loaded"),
        &trust,
    ))
    .expect("the fixture's own bundle loads through the one entry");

    assert_eq!(
        serving.name(),
        NAME,
        "the instance is registered under the name the manifest declares"
    );
    let names: Vec<&str> = serving
        .capabilities()
        .iter()
        .map(|capability| capability.name.as_str())
        .collect();
    assert_eq!(
        names, CAPABILITIES,
        "the serving value carries the capabilities it registered, in the library's order"
    );
    assert_eq!(
        serving.instance().capabilities(),
        serving.capabilities(),
        "what was registered is what the instance declared"
    );
    assert!(
        serving
            .registry()
            .route(&format!("{NAME}.tool.echo"))
            .is_ok(),
        "the registered instance routes a capability"
    );
}

#[test]
fn a_bundle_that_is_not_there_is_refused_by_the_path_that_is_not_there() {
    let host = Host::new();
    let path = std::env::temp_dir()
        .join("metis-load-test-absent")
        .join("plugin.metisbnd");

    let error = refusal(
        block_on(load_bundle(
            &host,
            &path,
            &config("loaded"),
            &host_context("loaded"),
            &TrustStore::empty(),
        )),
        "a bundle file that is not there cannot load",
    );

    match error {
        LoadError::Unreadable { path: refused, .. } => {
            assert_eq!(refused, path, "the refusal names the path that was read")
        }
        other => panic!("a file that cannot be read is its own refusal, and this one is {other}"),
    }
}

#[test]
fn a_bundle_this_host_does_not_trust_is_refused_before_its_manifest_is_read() {
    let host = Host::new();
    // The archive carries no manifest at all, so a manifest-first order would
    // answer an absent manifest. The trust refusal is what proves the order.
    let (file, _trust) = bundle::bundle_file(None);

    let error = refusal(
        block_on(load_bundle(
            &host,
            file.path(),
            &config("loaded"),
            &host_context("loaded"),
            &TrustStore::empty(),
        )),
        "a bundle no configured root signed cannot load",
    );

    match error {
        LoadError::Trust(failure) => assert!(
            failure.is_trust_failure(),
            "an unknown root is a trust failure: {failure}"
        ),
        other => {
            panic!("the signature is decided before the manifest, and this refusal is {other}")
        }
    }
}

#[test]
fn a_trusted_bundle_with_no_manifest_is_refused_by_the_manifest_step() {
    let host = Host::new();
    let (file, trust) = bundle::bundle_file(None);

    let error = refusal(
        block_on(load_bundle(
            &host,
            file.path(),
            &config("loaded"),
            &host_context("loaded"),
            &trust,
        )),
        "a trusted bundle with no root manifest cannot load",
    );

    match error {
        LoadError::Manifest(ManifestError::Absent { path }) => assert!(
            path.ends_with(bundle::MANIFEST),
            "the refusal names where the manifest would have been: {}",
            path.display()
        ),
        other => {
            panic!("an absent manifest is the manifest step's refusal, and this one is {other}")
        }
    }
}

#[test]
fn the_plugin_receives_the_configuration_and_the_host_context_before_it_is_registered() {
    let host = Host::new();
    let (file, trust) = bundle::bundle_file(Some(&hand_written(&CAPABILITIES)));
    let (config, context) = matched("loaded");

    let serving = block_on(load_bundle(&host, file.path(), &config, &context, &trust))
        .expect("the fixture's own bundle loads");

    // The plugin read both parts of the one payload: it refuses an
    // initialisation whose configuration names another instance, which is a
    // comparison against the host context, and it logs the context's name, so
    // the one line proves the other part arrived.
    assert!(
        host.logs()
            .iter()
            .any(|line| line.ends_with("fixture: initialised for loaded")),
        "the plugin saw the host context's instance name: {:?}",
        host.logs()
    );
    // The plugin's own constructor ran to completion before the load returned,
    // and the load returns only after registering the instance, so the plugin
    // was initialised before it was registered.
    let ordering = host.ordering();
    assert!(
        ordering
            .iter()
            .any(|line| line.starts_with("initialisation started from")),
        "initialisation started on the host: {ordering:?}"
    );
    assert!(
        ordering
            .iter()
            .any(|line| line.starts_with("initialisation resolved")),
        "the plugin's own constructor resolved before the load returned: {ordering:?}"
    );
    assert_eq!(
        serving.name(),
        NAME,
        "the registered instance is the one that just initialised"
    );
}

#[test]
fn a_configuration_the_plugin_cannot_read_is_refused_as_the_configuration_class() {
    let host = Host::new();
    let (file, trust) = bundle::bundle_file(Some(&hand_written(&CAPABILITIES)));
    // The configuration was resolved for another instance, which is the
    // fixture's own way of saying that what it was handed does not match what
    // it declared.
    let (config, context) = (self::config("someone-else"), host_context("loaded"));

    let error = refusal(
        block_on(load_bundle(&host, file.path(), &config, &context, &trust)),
        "a configuration the plugin cannot read cannot load",
    );

    match error {
        LoadError::ConfigurationRefused { plugin, status } => {
            assert_eq!(plugin, NAME, "the refusal names the plugin that refused it");
            assert_eq!(
                status,
                MetisStatus::InvalidArgument,
                "invalid_argument is the one status that speaks about the configuration"
            );
        }
        other => panic!(
            "a configuration the plugin cannot read is the configuration's class, and this refusal is {other}"
        ),
    }
}

#[test]
fn the_plugins_own_setup_failure_is_refused_as_the_plugins_defect() {
    let host = Host::new();
    let (file, trust) = bundle::bundle_file(Some(&hand_written(&CAPABILITIES)));
    // The configuration and the host context agree, so the failure that
    // follows is the plugin's own and not the configuration's.
    let (config, context) = matched(DEFECTIVE_INITIALISATION);

    let error = refusal(
        block_on(load_bundle(&host, file.path(), &config, &context, &trust)),
        "a plugin whose own setup fails cannot load",
    );

    match error {
        LoadError::PluginDefect { plugin, status } => {
            assert_eq!(plugin, NAME, "the refusal names the plugin that failed");
            assert_eq!(
                status,
                MetisStatus::Internal,
                "the plugin's own failure keeps its own status"
            );
        }
        other => {
            panic!(
                "a plugin whose own setup fails is the plugin's defect, and this refusal is {other}"
            )
        }
    }

    // The contract's failure ordering, at the seam where the caller drives the
    // hand-off: the plugin's future is dropped, and then the instance pointer
    // the entry handed out is destroyed.
    let ordering = host.ordering();
    assert!(
        ordering
            .iter()
            .any(|line| line.starts_with("instance destroyed after a failed initialisation")),
        "the failed initialisation destroyed the instance after dropping its future: {ordering:?}"
    );
    assert!(
        !ordering
            .iter()
            .any(|line| line.starts_with("initialisation resolved")),
        "a failed initialisation never resolves Ok, so nothing was registered: {ordering:?}"
    );
}

/// The fixture's declared capability names with one of them replaced, so a
/// manifest can declare a list that disagrees with the library.
fn declared_names(replacing: &str, with: &str) -> Vec<String> {
    let mut names: Vec<String> = CAPABILITIES.iter().map(|name| name.to_string()).collect();
    let index = names
        .iter()
        .position(|name| *name == replacing)
        .unwrap_or_else(|| panic!("the fixture declares {replacing}"));
    names[index] = with.to_string();
    names
}

#[test]
fn a_manifest_capability_the_library_does_not_report_is_refused_by_name() {
    let host = Host::new();
    let (file, trust) = bundle::bundle_file(Some(&hand_written(&declared_names(
        "tool.echo",
        "tool.absent",
    ))));

    let error = refusal(
        block_on(load_bundle(
            &host,
            file.path(),
            &config("loaded"),
            &host_context("loaded"),
            &trust,
        )),
        "a manifest capability the library does not report cannot load",
    );

    match error {
        LoadError::CapabilityMismatch { capability, .. } => assert_eq!(
            capability, "tool.absent",
            "the refusal names the capability the manifest declares and the library does not"
        ),
        other => panic!(
            "a capability the manifest declares and the library does not is a disagreement, and this refusal is {other}"
        ),
    }
}

#[test]
fn a_library_capability_the_manifest_does_not_declare_is_refused_by_name() {
    let host = Host::new();
    let names: Vec<String> = CAPABILITIES
        .iter()
        .filter(|name| **name != "tool.panic_drop")
        .map(|name| name.to_string())
        .collect();
    let (file, trust) = bundle::bundle_file(Some(&hand_written(&names)));

    let error = refusal(
        block_on(load_bundle(
            &host,
            file.path(),
            &config("loaded"),
            &host_context("loaded"),
            &trust,
        )),
        "a manifest that declares fewer capabilities than the library reports cannot load",
    );

    match error {
        LoadError::CapabilityMismatch { capability, .. } => assert_eq!(
            capability, "tool.panic_drop",
            "the refusal names the capability the library reports and the manifest does not declare"
        ),
        other => panic!(
            "a capability the manifest does not declare is a disagreement, and this refusal is {other}"
        ),
    }
}
