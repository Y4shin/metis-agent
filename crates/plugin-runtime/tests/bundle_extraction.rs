//! The bundle extractor, tested at its one public seam: `extract(verified,
//! target)` over real bytes.
//!
//! Every archive here is built inside the test with the `zip` writer and then
//! signed by the test signer, so a hostile archive is written by the test
//! rather than committed as a fixture. The staging directory is observed
//! through the filesystem: a refusal leaves none behind, and a `StagedBundle`
//! owns its own until it is dropped.

// rule: allow(dead_code): these tests share helpers, and a slice that has not
// reached one of them yet would otherwise warn.
#![allow(dead_code)]

mod support;

use std::collections::BTreeSet;
use std::io::{Cursor, Write};
use std::sync::{Mutex, MutexGuard};

use plugin_runtime::bundle::{
    STAGING_PREFIX, StageError, StagedBundle, TrustStore, extract, verify_bundle,
};
use support::signer::{self, TestChain};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// The target every test stages for, unless it names another.
const TARGET: &str = "linux-x86_64";

/// A second target, so a bundle can hold more than this host's files.
const OTHER_TARGET: &str = "macos-aarch64";

/// The manifest's name at the archive root, as the contract fixes it.
const MANIFEST: &str = "manifest.toml";

/// The library's file name inside a target directory.
const LIBRARY_FILE: &str = "sample_plugin.so";

/// The lock every test that stages takes, so a test that counts staging
/// directories is not racing another test of this binary.
static STAGING: Mutex<()> = Mutex::new(());

/// The lock, recovered from a previous test's panic so one failure does not
/// cascade.
fn lock() -> MutexGuard<'static, ()> {
    STAGING.lock().unwrap_or_else(|error| error.into_inner())
}

/// The library's path in the archive, for a target.
fn library_entry(target: &str, file_name: &str) -> String {
    format!("lib/{target}/{file_name}")
}

/// The trust store a daemon builds for these tests: the test chain's anchor is
/// a root, and the store starts from the empty compiled-in default.
fn test_trust() -> TrustStore {
    TrustStore::empty().with_root(TestChain::standard(3).anchor().public_key())
}

/// Extracts a bundle whose archive is `archive` for `target`.
///
/// The bundle is signed here and verified here, because extraction's input is a
/// verified bundle: what this seam reads is the archive the verifier accepted.
fn extract_archive(archive: &[u8], target: &str) -> Result<StagedBundle, StageError> {
    let bytes = signer::signed_bundle(&TestChain::standard(3), archive);
    let trust = test_trust();
    let verified = verify_bundle(&bytes, &trust).expect("the bundle this test signed verifies");
    extract(&verified, target)
}

/// The staging directory names the process's temporary directory holds.
fn staging_directories() -> BTreeSet<String> {
    std::fs::read_dir(std::env::temp_dir())
        .expect("the temporary directory is readable")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(STAGING_PREFIX)
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

/// A ZIP archive built entry by entry, so the tests own every byte of it.
struct TestArchive {
    writer: ZipWriter<Cursor<Vec<u8>>>,
}

impl TestArchive {
    fn new() -> TestArchive {
        TestArchive {
            writer: ZipWriter::new(Cursor::new(Vec::new())),
        }
    }

    /// A regular file entry that declares the mode the host itself writes.
    fn file(&mut self, name: &str, bytes: &[u8]) {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(0o600);
        self.entry(name, bytes, options);
    }

    /// A deflated file entry, which is how a producer writes a library.
    fn deflated_file(&mut self, name: &str, bytes: &[u8]) {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o600);
        self.entry(name, bytes, options);
    }

    fn entry(&mut self, name: &str, bytes: &[u8], options: SimpleFileOptions) {
        self.writer
            .start_file(name, options)
            .expect("the entry starts");
        self.writer.write_all(bytes).expect("the entry writes");
    }

    /// Finishes the archive and returns its bytes.
    fn finish(self) -> Vec<u8> {
        self.writer
            .finish()
            .expect("the archive finishes")
            .into_inner()
    }
}

#[test]
fn a_well_formed_bundle_stages_the_manifest_and_this_hosts_library() {
    let manifest_bytes: &[u8] = b"name = \"dev.metis.test-plugin\"\n";
    let library_bytes: &[u8] = b"not a library, but these bytes are the point";
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, manifest_bytes);
    archive.deflated_file(&library_entry(TARGET, LIBRARY_FILE), library_bytes);
    let archive = archive.finish();

    let staged = extract_archive(&archive, TARGET).expect("a well-formed bundle stages");

    assert_eq!(
        std::fs::read(staged.manifest_path()).expect("the manifest is staged"),
        manifest_bytes
    );
    assert_eq!(
        std::fs::read(staged.library_path()).expect("the library is staged"),
        library_bytes
    );
    assert_eq!(staged.manifest_path(), staged.root().join(MANIFEST));
    assert_eq!(
        staged.library_path(),
        staged.root().join("lib").join(TARGET).join(LIBRARY_FILE)
    );
}
