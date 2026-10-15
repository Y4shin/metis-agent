//! The no-executor property at the load-entry seam, in a binary of its own.
//!
//! The entry composes verification, extraction, manifest validation, library
//! selection, the ABI check, the plugin's asynchronous initialisation, the
//! capability cross-check and registration, and the one property that must not
//! change across all of it is that nothing here starts a thread: the entry
//! hands back a future, the caller's runtime polls it, and plugin code only runs
//! inside those polls.
//!
//! This lives in its own integration binary on purpose: the test harness starts
//! a thread per test, so a count taken in a binary that runs other cases in
//! parallel would measure the harness rather than the load.

mod support;

use plugin_runtime::Host;
use plugin_runtime::load::load_bundle;
use serde_json::json;
use support::block_on;
use support::bundle;
use support::thread_count;

#[test]
fn a_bundle_load_starts_no_thread_and_no_runtime_of_its_own() {
    // Everything that is not the entry is built and warmed first, including the
    // fixture cdylib, the archive and the signed container, so the count is
    // taken around the load itself.
    let (file, trust) = bundle::bundle_file(Some(&bundle::manifest(&bundle::CAPABILITIES)));
    let host = Host::new();
    let config = json!({ "instance": "loading" });
    let context = json!({ "instance_name": "loading", "capability_version": 1 });

    let before = thread_count();
    let serving = block_on(load_bundle(&host, file.path(), &config, &context, &trust))
        .expect("the fixture's own bundle loads");
    let after = thread_count();

    assert_eq!(
        before, after,
        "the load entry starts no thread of its own, so the caller stays the only executor"
    );
    drop(serving);
}
