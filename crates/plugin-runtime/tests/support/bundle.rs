//! A signed bundle on disk, for the tests that drive the public load entry.
//!
//! A load takes a path, so a test here turns the fixture cdylib's own bytes and
//! a manifest the test writes by hand into one signed container, writes that
//! file, and removes it when the guard is dropped. The archive and the container
//! are built the way a producer builds them, so the load entry reads a real
//! bundle rather than a mock of one.
//!
//! The target is derived here rather than imported, because the contract fixes
//! it as the operating system and the architecture joined by a hyphen and the
//! runtime does not export its own derivation. The load entry derives the same
//! string for its own call to the extractor, so a drift between the two would
//! refuse a bundle that carries the library this host needs.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::time::{SystemTime, UNIX_EPOCH};

use plugin_runtime::bundle::TrustStore;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::fixture_path;
use super::signer::{self, TestChain};

/// The name the test chains' leaf-authorising certificate carries, which is the
/// name every manifest here declares and the name the instance is registered
/// under.
///
/// It carries no dot because the registry resolves a qualified capability name
/// as `<instance>.<tool>` by the first dot, so an instance name that carried one
/// would not route. That limitation is not this entry's to change.
pub const INSTANCE_NAME: &str = "fixture";

/// The manifest's name at the archive root, as the contract fixes it.
pub const MANIFEST: &str = "manifest.toml";

/// How many bundles this process has written, so two of them cannot share a
/// name.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// This host's target, spelled the way contract 17.8 fixes it.
pub fn host_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The file name the fixture's own library carries, which is the name a
/// bundle's archive carries it under.
pub fn library_file_name() -> String {
    fixture_path()
        .file_name()
        .expect("the fixture library has a file name")
        .to_string_lossy()
        .into_owned()
}

/// The bytes a bundle carries as this host's library: the fixture cdylib.
pub fn library_bytes() -> Vec<u8> {
    std::fs::read(fixture_path()).expect("the fixture library this test built reads")
}

/// The archive-relative path a library for `target` is carried at.
pub fn library_path(target: &str, file_name: &str) -> String {
    format!("lib/{target}/{file_name}")
}

/// Writes the entries into a ZIP archive, entry by entry.
///
/// Every entry declares the mode this host writes, because the extractor
/// refuses any other declared mode, and stored rather than deflated, because
/// compression is not what these tests are about.
pub fn archive(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
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

/// One archive's entries: the root manifest when the test gives one, this
/// host's library, and anything else the test adds.
pub fn entries(manifest: Option<&str>, extra: &[(String, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    let target = host_target();
    let mut entries = Vec::new();
    if let Some(manifest) = manifest {
        entries.push((MANIFEST.to_string(), manifest.as_bytes().to_vec()));
    }
    entries.push((library_path(&target, &library_file_name()), library_bytes()));
    entries.extend(extra.iter().cloned());
    entries
}

/// The test chain: three links, with a leaf common name a capability can be
/// addressed under.
pub fn chain() -> TestChain {
    let mut specs = signer::standard_specs(3);
    specs[0].common_name = INSTANCE_NAME.to_string();
    TestChain::build(&specs, &[1, 2, 3])
}

/// The trust store that accepts one chain's anchor and nothing else.
///
/// The anchor is installed as a root explicitly, because the runtime's
/// compiled-in root set trusts nothing. A test that wants the bundle refused
/// passes its own store instead.
pub fn trust_for(chain: &TestChain) -> TrustStore {
    TrustStore::empty().with_root(chain.anchor().public_key())
}

/// The capability names the fixture declares, in the order it declares them.
///
/// They are literals rather than a list read out of the library, because the
/// manifest an author writes is hand-written: a list read from the library
/// would make the entry's cross-check vacuous.
pub const CAPABILITIES: [&str; 10] = [
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

/// The manifest an author writes by hand, declaring exactly the capability
/// names it is given and this host's library.
///
/// The text is a literal, not a serialisation of any type in this tree: the
/// format belongs to the author, and a hand-written manifest is what proves it.
pub fn manifest<S: AsRef<str>>(names: &[S]) -> String {
    let target = host_target();
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
name = "{INSTANCE_NAME}"
version = "1.0.0"
abi_major = 1
min_abi_minor = 2

[libraries]
{target} = "{library}"
{capabilities}
[config_schema]
type = "object"
"#,
        library = library_path(&target, &library_file_name()),
    )
}

/// A signed bundle on disk and the trust store that accepts it.
pub fn bundle_file(manifest: Option<&str>) -> (BundleFile, TrustStore) {
    let chain = chain();
    let trust = trust_for(&chain);
    let bytes = signer::signed_bundle(&chain, &archive(&entries(manifest, &[])));
    (BundleFile::write(&bytes), trust)
}

/// A signed bundle written to a file the load entry reads, removed on drop.
#[derive(Debug)]
pub struct BundleFile {
    path: PathBuf,
}

impl BundleFile {
    /// Writes the container's bytes under a name no other test shares.
    pub fn write(bytes: &[u8]) -> BundleFile {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .subsec_nanos();
        let name = format!(
            "metis-load-test-{}-{nanos}-{}.metisbnd",
            std::process::id(),
            COUNTER.fetch_add(1, SeqCst)
        );
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, bytes)
            .unwrap_or_else(|error| panic!("the bundle file writes: {error}"));
        BundleFile { path }
    }

    /// Where the file is, which is what the load entry takes.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for BundleFile {
    fn drop(&mut self) {
        // The file is this value's own, and a removal that fails is not worth
        // panicking in a drop.
        let _ = std::fs::remove_file(&self.path);
    }
}
