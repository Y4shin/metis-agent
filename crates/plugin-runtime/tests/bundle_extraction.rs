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
    MAX_ENTRIES, MAX_ENTRY_SIZE, MAX_UNCOMPRESSED_SIZE, STAGING_PREFIX, StageError, StagedBundle,
    TrustStore, extract, verify_bundle,
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

/// Every path the staging directory holds, relative to it and sorted, with a
/// trailing separator on a directory so an empty one is visible.
fn staged_paths(root: &std::path::Path) -> Vec<String> {
    let mut paths = Vec::new();
    let mut queue = vec![root.to_path_buf()];
    while let Some(directory) = queue.pop() {
        for entry in std::fs::read_dir(&directory).expect("the staging directory reads") {
            let path = entry.expect("a staged entry reads").path();
            let relative = path
                .strip_prefix(root)
                .expect("a staged path is under the root");
            let mut name = relative.to_string_lossy().into_owned();
            if path.is_dir() {
                name.push('/');
                queue.push(path);
            }
            paths.push(name);
        }
    }
    paths.sort();
    paths
}

#[test]
fn a_bundle_holding_several_targets_stages_only_this_hosts_target() {
    let _guard = lock();
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.deflated_file(&library_entry(OTHER_TARGET, "other.so"), b"another target");
    archive.deflated_file(&library_entry(TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    let staged = extract_archive(&archive.finish(), TARGET).expect("this host's target stages");

    // The other target's directory is not created at all, not even empty.
    assert_eq!(
        staged_paths(staged.root()),
        vec![
            "lib/".to_string(),
            format!("lib/{TARGET}/"),
            library_entry(TARGET, LIBRARY_FILE),
            MANIFEST.to_string(),
        ]
    );
    assert_eq!(
        std::fs::read(staged.library_path()).expect("the library is staged"),
        LIBRARY_BYTES
    );
}

#[test]
fn another_target_holding_two_files_still_stages() {
    let _guard = lock();
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.file(&library_entry(OTHER_TARGET, "one.so"), b"one");
    archive.file(&library_entry(OTHER_TARGET, "two.so"), b"two");
    archive.file(&library_entry(TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    let staged = extract_archive(&archive.finish(), TARGET).expect("this host's target stages");
    assert_eq!(
        staged_paths(staged.root()),
        vec![
            "lib/".to_string(),
            format!("lib/{TARGET}/"),
            library_entry(TARGET, LIBRARY_FILE),
            MANIFEST.to_string(),
        ]
    );
}

#[test]
fn an_entry_outside_the_archive_layout_is_not_staged() {
    let _guard = lock();
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.file("notes.txt", b"an entry the layout does not name");
    archive.file(&library_entry(TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    archive.file(&format!("lib/{TARGET}/nested/data.bin"), b"a deeper tree");
    let staged = extract_archive(&archive.finish(), TARGET).expect("this host's target stages");
    assert_eq!(
        staged_paths(staged.root()),
        vec![
            "lib/".to_string(),
            format!("lib/{TARGET}/"),
            library_entry(TARGET, LIBRARY_FILE),
            MANIFEST.to_string(),
        ]
    );
}

#[test]
fn a_hostile_entry_belonging_to_another_target_is_still_refused() {
    // Every entry is validated, whether or not this host's load needs it.
    let archive = archive_then(TARGET, |archive| {
        archive.symlink(&library_entry(OTHER_TARGET, "escape.so"), "/etc/passwd");
    });
    assert_eq!(
        refusal(&archive, TARGET),
        StageError::EntryIsSymlink {
            name: library_entry(OTHER_TARGET, "escape.so")
        }
    );
}

#[test]
fn a_corrupt_entry_for_another_target_does_not_stop_this_hosts_load() {
    // An entry this host does not load is validated by its metadata, not by
    // its bytes, so it is never decompressed: its own damage is not this
    // host's problem.
    const OTHER_BYTES: &[u8] = b"a stored entry for another target";
    let _guard = lock();
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.file(&library_entry(OTHER_TARGET, "other.so"), OTHER_BYTES);
    archive.file(&library_entry(TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    let mut archive = archive.finish();
    corrupt_stored_data(&mut archive, OTHER_BYTES);
    let staged = extract_archive(&archive, TARGET).expect("this host's target stages");
    assert_eq!(
        std::fs::read(staged.library_path()).expect("the library is staged"),
        LIBRARY_BYTES
    );
}

#[test]
fn a_successful_extraction_owns_its_staging_directory_until_it_is_dropped() {
    let _guard = lock();
    let before = staging_directories();
    let staged =
        extract_archive(&well_formed_archive(TARGET), TARGET).expect("a well-formed bundle stages");
    let root = staged.root().to_path_buf();

    assert!(
        root.is_dir(),
        "the staging directory exists while the bundle is held"
    );
    // The one directory that appeared is this bundle's, which is what pins the
    // prefix the counts here rely on.
    let during = staging_directories();
    assert_eq!(
        during.difference(&before).count(),
        1,
        "the extraction created one staging directory"
    );

    drop(staged);
    assert!(
        !root.exists(),
        "the staging directory is removed with the bundle"
    );
    assert_eq!(staging_directories(), before);
}

#[test]
fn a_refused_extraction_removes_its_staging_directory() {
    // The hostile entry is the archive's last, so the manifest and the library
    // were written into the staging directory before the walk reached it: the
    // refusal has a directory of its own to remove.
    let archive = archive_then(TARGET, |archive| archive.file("../escape", b"escaped"));
    let _guard = lock();
    let before = staging_directories();
    let error = extract_archive(&archive, TARGET).expect_err("the traversal entry is refused");
    assert_eq!(
        error,
        StageError::ParentEntryName {
            name: "../escape".to_string()
        }
    );
    assert_eq!(
        staging_directories(),
        before,
        "the refused extraction left its staging directory behind"
    );
}

#[cfg(unix)]
#[test]
fn a_staged_file_and_its_directories_carry_the_hosts_own_modes() {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path)
            .expect("the staged path exists")
            .permissions()
            .mode()
            & 0o7777
    }

    let _guard = lock();
    let staged =
        extract_archive(&well_formed_archive(TARGET), TARGET).expect("a well-formed bundle stages");
    // The archive declares its own modes, and none of them reaches the
    // filesystem: the host writes 0o700 for a directory and 0o600 for a file.
    assert_eq!(mode(staged.root()), 0o700);
    assert_eq!(mode(&staged.root().join("lib")), 0o700);
    assert_eq!(mode(&staged.root().join("lib").join(TARGET)), 0o700);
    assert_eq!(mode(staged.manifest_path()), 0o600);
    assert_eq!(mode(staged.library_path()), 0o600);
}

/// Rewrites the uncompressed size one entry's central-directory record
/// declares, so a test can build an archive whose claim its data does not back.
///
/// A producer controls both its data and its declaration, so this is a field a
/// hostile bundle can set, and the rules read the declaration before any data is
/// decoded.
fn declare_size(archive: &mut [u8], name: &str, declared_size: u32) {
    let record = central_directory_record(archive, name);
    archive[record + 24..record + 28].copy_from_slice(&declared_size.to_le_bytes());
}

/// The offset of the central-directory record of the entry named `name`.
fn central_directory_record(archive: &[u8], name: &str) -> usize {
    const RECORD: &[u8] = b"PK\x01\x02";
    let mut searched: usize = 0;
    while let Some(found) = find(&archive[searched..], RECORD) {
        let record = searched + found;
        let length = u16::from_le_bytes([archive[record + 28], archive[record + 29]]) as usize;
        let name_at = record + 46;
        if archive.get(name_at..name_at + length) == Some(name.as_bytes()) {
            return record;
        }
        searched = record + RECORD.len();
    }
    panic!("the archive holds no central-directory record for {name}");
}

#[test]
fn an_archive_holding_too_many_entries_is_refused() {
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.file(&library_entry(TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    for index in 0..MAX_ENTRIES {
        archive.file(
            &library_entry(OTHER_TARGET, &format!("filler{index}.so")),
            b"x",
        );
    }
    assert_eq!(
        refusal(&archive.finish(), TARGET),
        StageError::TooManyEntries {
            count: MAX_ENTRIES + 2,
            limit: MAX_ENTRIES
        }
    );
}

#[test]
fn an_archive_holding_exactly_the_entry_limit_stages() {
    let _guard = lock();
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.file(&library_entry(TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    for index in 0..MAX_ENTRIES - 2 {
        archive.file(
            &library_entry(OTHER_TARGET, &format!("filler{index}.so")),
            b"x",
        );
    }
    extract_archive(&archive.finish(), TARGET).expect("an archive at the entry limit stages");
}

#[test]
fn an_entry_declaring_more_than_the_entry_limit_is_refused() {
    let name = library_entry(OTHER_TARGET, "big.so");
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.file(&library_entry(TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    archive.file(&name, b"one byte, and a claim this host refuses to believe");
    let mut archive = archive.finish();
    declare_size(&mut archive, &name, MAX_ENTRY_SIZE as u32 + 1);
    assert_eq!(
        refusal(&archive, TARGET),
        StageError::EntryTooLarge {
            name,
            size: MAX_ENTRY_SIZE + 1,
            limit: MAX_ENTRY_SIZE
        }
    );
}

#[test]
fn an_entry_declaring_exactly_the_entry_limit_stages() {
    let _guard = lock();
    let name = library_entry(OTHER_TARGET, "big.so");
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.file(&library_entry(TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    archive.file(&name, b"one byte, and a claim this host accepts");
    let mut archive = archive.finish();
    declare_size(&mut archive, &name, MAX_ENTRY_SIZE as u32);
    extract_archive(&archive, TARGET).expect("a declaration at the limit is inside it");
}

#[test]
fn an_archive_declaring_more_than_the_total_limit_is_refused() {
    // Four entries at the per-entry limit already declare the whole archive
    // limit, and the two staged files push the sum past it.
    let mut archive = TestArchive::new();
    archive.file(MANIFEST, MANIFEST_BYTES);
    archive.file(&library_entry(TARGET, LIBRARY_FILE), LIBRARY_BYTES);
    let declared_entries = 5;
    for index in 0..declared_entries {
        archive.file(
            &library_entry(OTHER_TARGET, &format!("big{index}.so")),
            b"one byte each",
        );
    }
    let mut archive = archive.finish();
    for index in 0..declared_entries {
        declare_size(
            &mut archive,
            &library_entry(OTHER_TARGET, &format!("big{index}.so")),
            MAX_ENTRY_SIZE as u32,
        );
    }
    let staged = (MANIFEST_BYTES.len() + LIBRARY_BYTES.len()) as u64;
    assert_eq!(
        refusal(&archive, TARGET),
        StageError::ArchiveTooLarge {
            declared_size: 4 * MAX_ENTRY_SIZE + staged,
            limit: MAX_UNCOMPRESSED_SIZE
        }
    );
}

#[test]
fn a_hostile_archive_is_refused_at_its_first_hostile_entry() {
    // Several hostile entries at once: a hostile entry is refused rather than
    // skipped, so the archive is answered for the first one the walk reaches
    // and the entries after it are never examined.
    let archive = archive_then(TARGET, |archive| {
        archive.file("../../escape", b"a traversal");
        archive.symlink(&library_entry(TARGET, "escape.so"), "/etc/passwd");
        archive.file("/etc/passwd", b"an absolute name");
    });
    assert_eq!(
        refusal(&archive, TARGET),
        StageError::ParentEntryName {
            name: "../../escape".to_string()
        }
    );
}
