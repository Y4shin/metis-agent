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

/// The manifest's content, which extraction does not parse.
const MANIFEST_BYTES: &[u8] = b"name = \"dev.metis.test-plugin\"\n";

/// The library's content, which extraction only copies.
const LIBRARY_BYTES: &[u8] = b"the library's bytes, which are the point";

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
///
/// A test that stages takes [`lock`] for its whole body, so no other test of
/// this binary creates or removes one while this counts. `refusal` takes the
/// lock itself; a test that stages directly takes it first.
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

    /// A file entry that declares `mode` as its Unix mode.
    fn file_declaring_mode(&mut self, name: &str, bytes: &[u8], mode: u32) {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(mode);
        self.entry(name, bytes, options);
    }

    /// A file entry that declares the raw external attributes `attributes`.
    fn file_declaring_attributes(&mut self, name: &str, bytes: &[u8], attributes: u32) {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .external_attributes(attributes);
        self.entry(name, bytes, options);
    }

    /// A symbolic link entry, which the archive's rules do not allow.
    fn symlink(&mut self, name: &str, target: &str) {
        self.writer
            .add_symlink(name, target, SimpleFileOptions::default())
            .expect("the symlink entry is added");
    }

    /// A directory entry, which the archive's rules do not allow.
    fn directory(&mut self, name: &str) {
        self.writer
            .add_directory(name, SimpleFileOptions::default())
            .expect("the directory entry is added");
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

/// A well-formed archive for `target`: the root manifest and that target's one
/// library file.
fn well_formed_archive(target: &str) -> Vec<u8> {
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.deflated_file(&library_entry(target, LIBRARY_FILE), LIBRARY_BYTES);
    archive.finish()
}

/// A well-formed archive for `target` plus one more entry, added last, so a
/// refusal has to remove what the extraction already wrote.
fn archive_then(target: &str, add: impl FnOnce(&mut TestArchive)) -> Vec<u8> {
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.deflated_file(&library_entry(target, LIBRARY_FILE), LIBRARY_BYTES);
    add(&mut archive);
    archive.finish()
}

/// The refusal an archive earns, with the assertion that it left no staging
/// directory behind: a refusal removes what it staged before it answers.
fn refusal(archive: &[u8], target: &str) -> StageError {
    let _guard = lock();
    let before = staging_directories();
    let error = extract_archive(archive, target).expect_err("the archive must be refused");
    assert_eq!(
        staging_directories(),
        before,
        "a refusal left a staging directory behind"
    );
    error
}

/// The offset of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Flips one byte of the stored bytes `needle`, so the entry's checksum no
/// longer matches the data it holds.
fn corrupt_stored_data(archive: &mut [u8], needle: &[u8]) {
    let at = find(archive, needle).expect("the stored bytes are in the archive");
    archive[at] ^= 0xff;
}

/// The well-formed case: every later test's baseline, and the one archive a
/// refusal test modifies rather than replaces.
#[test]
fn a_well_formed_bundle_stages_the_manifest_and_this_hosts_library() {
    let _guard = lock();
    let manifest_bytes = MANIFEST_BYTES;
    let library_bytes = LIBRARY_BYTES;
    let archive = well_formed_archive(TARGET);

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

#[test]
fn an_archive_that_is_not_a_zip_is_refused() {
    assert!(matches!(
        refusal(b"this is not a ZIP archive at all", TARGET),
        StageError::ArchiveUnreadable { .. }
    ));
}

#[test]
fn a_bundle_without_this_hosts_library_is_refused() {
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.file(&library_entry(OTHER_TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    assert_eq!(
        refusal(&archive.finish(), TARGET),
        StageError::MissingTargetLibrary {
            target: TARGET.to_string()
        }
    );
}

#[test]
fn a_target_directory_holding_two_files_is_refused() {
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.file(&library_entry(TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    archive.file(&library_entry(TARGET, "second.so"), LIBRARY_BYTES);
    assert_eq!(
        refusal(&archive.finish(), TARGET),
        StageError::AmbiguousTargetLibrary {
            target: TARGET.to_string(),
            files: 2
        }
    );
}

#[test]
fn an_absolute_entry_name_is_refused() {
    let archive = archive_then(TARGET, |archive| archive.file("/etc/passwd", b"escaped"));
    assert_eq!(
        refusal(&archive, TARGET),
        StageError::AbsoluteEntryName {
            name: "/etc/passwd".to_string()
        }
    );
}

#[test]
fn an_entry_name_that_carries_a_parent_directory_is_refused() {
    let archive = archive_then(TARGET, |archive| archive.file("../../escape", b"escaped"));
    assert_eq!(
        refusal(&archive, TARGET),
        StageError::ParentEntryName {
            name: "../../escape".to_string()
        }
    );
}

#[test]
fn a_symlink_entry_is_refused() {
    let archive = archive_then(TARGET, |archive| {
        archive.symlink(&library_entry(TARGET, "escape.so"), "/etc/passwd");
    });
    assert_eq!(
        refusal(&archive, TARGET),
        StageError::EntryIsSymlink {
            name: library_entry(TARGET, "escape.so")
        }
    );
}

#[test]
fn a_directory_entry_is_refused() {
    let archive = archive_then(TARGET, |archive| {
        archive.directory(&format!("lib/{TARGET}/extra"));
    });
    assert_eq!(
        refusal(&archive, TARGET),
        StageError::EntryIsDirectory {
            name: format!("lib/{TARGET}/extra/")
        }
    );
}

#[test]
fn an_entry_declaring_another_file_type_is_refused() {
    // A FIFO: not a regular file, and not a directory or a symlink either.
    let mode = 0o010000 | 0o600;
    let archive = archive_then(TARGET, |archive| {
        archive.file_declaring_attributes(&library_entry(TARGET, "fifo.so"), b"x", mode << 16);
    });
    assert_eq!(
        refusal(&archive, TARGET),
        StageError::EntryIsNotRegularFile {
            name: library_entry(TARGET, "fifo.so"),
            mode
        }
    );
}

#[test]
fn an_entry_declaring_another_mode_is_refused() {
    // A regular file with the usual permissions: the host writes 0o600, so a
    // declaration of anything else is refused rather than applied.
    let archive = archive_then(TARGET, |archive| {
        archive.file_declaring_mode(&library_entry(TARGET, "world.so"), b"x", 0o644);
    });
    assert_eq!(
        refusal(&archive, TARGET),
        StageError::EntryModeRefused {
            name: library_entry(TARGET, "world.so"),
            mode: 0o100644
        }
    );
}

#[test]
fn an_entry_whose_bytes_do_not_read_is_refused() {
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, b"stored bytes whose checksum will not match");
    archive.file(&library_entry(TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    let mut archive = archive.finish();
    corrupt_stored_data(&mut archive, b"stored bytes whose checksum");
    assert!(matches!(
        refusal(&archive, TARGET),
        StageError::EntryUnreadable { name, .. } if name == MANIFEST
    ));
}

#[test]
fn a_bundle_without_a_root_manifest_still_stages() {
    // Whether a manifest is there is the manifest step's question, not
    // extraction's: extraction answers where the file would be.
    let _guard = lock();
    let mut archive = TestArchive::new();
    archive.file(&library_entry(TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    let staged = extract_archive(&archive.finish(), TARGET).expect("the library alone stages");
    assert!(!staged.manifest_path().exists());
    assert_eq!(
        std::fs::read(staged.library_path()).expect("the library is staged"),
        LIBRARY_BYTES
    );
}
