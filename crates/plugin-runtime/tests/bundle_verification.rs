//! The bundle verifier, tested at its one public seam: `verify_bundle` over real
//! bytes.
//!
//! The bytes come from two places. A committed vector, built by the test signer
//! in `support/signer.rs`, pins the contract's layout and domain tag at byte
//! level: the vector and its altered copy are rebuilt and compared on every run.
//! Bundles built inside a test from the same signer carry the refusal cases,
//! because a refusal needs a file whose signature is valid and whose structure
//! is wrong.

mod support;

use std::path::PathBuf;

use plugin_runtime::bundle::{BundleError, TrustStore, verify_bundle};
use support::signer::{self, Header, TestChain};

/// The committed vector: a three-link chain, the leaf's signature, an empty ZIP.
const VECTOR: &[u8] = include_bytes!("vectors/test-bundle.metis");

/// The committed altered copy: the same bundle with one signature byte flipped.
const ALTERED: &[u8] = include_bytes!("vectors/test-bundle-altered-signature.metis");

/// The chain the committed vector carries, rebuilt from the same seeds.
fn vector_chain() -> TestChain {
    TestChain::standard(3)
}

/// The two committed files, as the signer builds them.
fn rebuilt_vectors() -> (Vec<u8>, Vec<u8>) {
    let bytes = signer::signed_bundle(&vector_chain(), signer::EMPTY_ZIP);
    let mut altered = bytes.clone();
    let last = altered.len() - signer::EMPTY_ZIP.len() - 1;
    altered[last] ^= 0x01;
    (bytes, altered)
}

fn vector_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/test-bundle.metis")
}

fn altered_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vectors/test-bundle-altered-signature.metis")
}

/// The trust store a daemon builds for these tests: the test chain's anchor is a
/// root, and the store starts from the empty compiled-in default.
fn test_trust() -> TrustStore {
    TrustStore::empty().with_root(vector_chain().anchor().public_key())
}

/// Writes the committed vectors.
///
/// It is not part of the suite: it exists so the vectors can be rebuilt with
/// `cargo test -p plugin-runtime --test bundle_verification -- --ignored`, and
/// the ordinary run checks the committed bytes against the same builder.
#[test]
#[ignore = "regenerates the committed vectors rather than checking them"]
fn regenerate_committed_vectors() {
    let (bytes, altered) = rebuilt_vectors();
    for (path, content) in [(vector_path(), &bytes), (altered_path(), &altered)] {
        std::fs::create_dir_all(path.parent().expect("the vector directory"))
            .expect("the vector directory is writable");
        std::fs::write(path, content).expect("the vector is writable");
    }
}

/// The committed vector is the bytes this test's signer builds, so the layout
/// and the domain tag the contract fixes are checked rather than assumed.
#[test]
fn the_committed_vectors_match_the_test_signer() {
    let (bytes, altered) = rebuilt_vectors();
    assert_eq!(VECTOR, bytes.as_slice(), "the committed vector");
    assert_eq!(ALTERED, altered.as_slice(), "the committed altered copy");
}

#[test]
fn a_valid_bundle_verifies() {
    let verified = verify_bundle(VECTOR, &test_trust()).expect("a valid bundle verifies");
    assert_eq!(verified.leaf_common_name(), signer::LEAF_COMMON_NAME);
    assert_eq!(verified.archive(), signer::EMPTY_ZIP);
    let header = verified.header();
    assert_eq!(header.container_version, signer::CONTAINER_VERSION);
    assert_eq!(header.signing_suite, signer::SIGNING_SUITE);
    assert_eq!(header.header_length, signer::HEADER_LENGTH);
    assert_eq!(header.reserved, 0);
    assert_eq!(header.chain_length, vector_chain().der().len() as u32);
    assert_eq!(header.signature_offset, 808);
    assert_eq!(header.signature_length, 64);
    assert_eq!(header.zip_length, signer::EMPTY_ZIP.len() as u64);
}

#[test]
fn a_bundle_the_compiled_in_root_set_does_not_trust_is_refused() {
    // The daemon's compiled-in root set is empty, so no shipped build trusts
    // the test key: the vector is refused until a test installs the anchor.
    let error = verify_bundle(VECTOR, &TrustStore::empty()).expect_err("the test key is not trusted");
    assert_eq!(
        error,
        BundleError::UnknownRoot {
            key_id: plugin_runtime::bundle::key_id(&vector_chain().anchor().public_key()),
        }
    );
}

#[test]
fn an_altered_copy_is_refused() {
    let error = verify_bundle(ALTERED, &test_trust()).expect_err("one changed byte breaks the signature");
    assert_eq!(error, BundleError::SignatureInvalid);
    assert!(error.is_trust_failure());
}
