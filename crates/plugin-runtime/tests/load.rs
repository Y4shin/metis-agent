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
fn hand_written(names: &[&str]) -> String {
    let target = bundle::host_target();
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
