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

use ed25519_dalek::SigningKey;
use plugin_runtime::bundle::{BundleError, TrustStore, key_id, verify_bundle};
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

/// A bundle whose chain region is `chain_bytes`, signed by `leaf`.
///
/// The header is a correct one for these regions, so a test that refuses the
/// bundle refuses it for the reason the test names rather than for a span.
fn bundle_for(chain_bytes: &[u8], archive: &[u8], leaf: &SigningKey) -> Vec<u8> {
    let header = Header::standard(chain_bytes.len(), 64, archive.len());
    signer::bundle(&header, chain_bytes, archive, leaf)
}

/// Writes one header field at the offset the contract fixes, big-endian.
fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
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
    let error =
        verify_bundle(VECTOR, &TrustStore::empty()).expect_err("the test key is not trusted");
    assert_eq!(
        error,
        BundleError::UnknownRoot {
            key_id: key_id(&vector_chain().anchor().public_key()),
        }
    );
}

#[test]
fn an_altered_copy_is_refused() {
    let error =
        verify_bundle(ALTERED, &test_trust()).expect_err("one changed byte breaks the signature");
    assert_eq!(error, BundleError::SignatureInvalid);
    assert!(error.is_trust_failure());
}

// --- the container -------------------------------------------------------
//
// Each test changes one field at the offset the contract fixes. The container's
// checks run before the signature is read, so a changed field is still refused
// for its own reason rather than as a broken signature.

#[test]
fn a_file_shorter_than_the_header_is_refused() {
    let error = verify_bundle(&VECTOR[..20], &test_trust()).expect_err("the header is incomplete");
    assert_eq!(error, BundleError::Truncated { len: 20 });
    assert!(!error.is_trust_failure());
}

#[test]
fn a_file_that_does_not_begin_with_the_magic_is_refused() {
    let mut bytes = VECTOR.to_vec();
    bytes[0] = b'X';
    let mut found = *b"METISBND";
    found[0] = b'X';
    assert_eq!(
        verify_bundle(&bytes, &test_trust()).unwrap_err(),
        BundleError::BadMagic { found }
    );
}

#[test]
fn an_unknown_container_version_is_refused() {
    let mut bytes = VECTOR.to_vec();
    put_u16(&mut bytes, 8, 7);
    assert_eq!(
        verify_bundle(&bytes, &test_trust()).unwrap_err(),
        BundleError::UnknownContainerVersion { found: 7 }
    );
}

#[test]
fn an_unknown_signing_suite_is_refused() {
    let mut bytes = VECTOR.to_vec();
    put_u16(&mut bytes, 10, 2);
    assert_eq!(
        verify_bundle(&bytes, &test_trust()).unwrap_err(),
        BundleError::UnknownSigningSuite { found: 2 }
    );
}

#[test]
fn a_header_length_that_is_not_the_fixed_one_is_refused() {
    let mut bytes = VECTOR.to_vec();
    put_u16(&mut bytes, 12, 41);
    assert_eq!(
        verify_bundle(&bytes, &test_trust()).unwrap_err(),
        BundleError::BadHeaderLength { found: 41 }
    );
}

#[test]
fn a_non_zero_reserved_field_is_refused() {
    let mut bytes = VECTOR.to_vec();
    put_u16(&mut bytes, 14, 1);
    assert_eq!(
        verify_bundle(&bytes, &test_trust()).unwrap_err(),
        BundleError::ReservedNotZero { found: 1 }
    );
}

#[test]
fn a_chain_length_that_disagrees_with_the_signature_offset_is_refused() {
    let mut bytes = VECTOR.to_vec();
    put_u32(&mut bytes, 16, 800);
    assert_eq!(
        verify_bundle(&bytes, &test_trust()).unwrap_err(),
        BundleError::ChainSpanMismatch {
            chain_length: 800,
            signature_offset: 808,
        }
    );
}

#[test]
fn a_span_that_does_not_account_for_the_whole_file_is_refused() {
    let mut bytes = VECTOR.to_vec();
    put_u64(&mut bytes, 32, signer::EMPTY_ZIP.len() as u64 + 1);
    assert_eq!(
        verify_bundle(&bytes, &test_trust()).unwrap_err(),
        BundleError::SpanMismatch {
            file_length: VECTOR.len(),
            signature_offset: 808,
            signature_length: 64,
            zip_length: signer::EMPTY_ZIP.len() as u64 + 1,
        }
    );
}

#[test]
fn a_signature_offset_before_the_header_is_refused() {
    let mut bytes = VECTOR.to_vec();
    put_u32(&mut bytes, 16, 1000);
    put_u64(&mut bytes, 20, 20);
    assert_eq!(
        verify_bundle(&bytes, &test_trust()).unwrap_err(),
        BundleError::BytesBeforeHeader {
            signature_offset: 20,
            header_length: 40,
        }
    );
}

#[test]
fn an_empty_chain_is_refused() {
    let chain = vector_chain();
    let bytes = bundle_for(&[], signer::EMPTY_ZIP, chain.leaf().key());
    let error = verify_bundle(&bytes, &test_trust()).expect_err("a chain needs one link");
    assert_eq!(error, BundleError::EmptyChain);
    assert!(error.is_trust_failure());
}

#[test]
fn a_chain_with_more_than_eight_links_is_refused() {
    let chain = TestChain::standard(9);
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    let error =
        verify_bundle(&bytes, &test_trust()).expect_err("a chain holds at most eight links");
    assert_eq!(error, BundleError::ChainTooLong { links: 9 });
    assert!(!error.is_trust_failure());
}

#[test]
fn trailing_bytes_in_the_chain_region_are_refused() {
    let chain = vector_chain();
    let mut chain_bytes = chain.der();
    chain_bytes.push(0x00);
    let bytes = bundle_for(&chain_bytes, signer::EMPTY_ZIP, chain.leaf().key());
    let error = verify_bundle(&bytes, &test_trust()).expect_err("the region must be consumed");
    assert_eq!(error, BundleError::TrailingChainBytes { bytes: 1 });
}

#[test]
fn a_malformed_certificate_is_refused() {
    let chain = vector_chain();
    let bytes = bundle_for(
        &chain.leaf().der()[..10],
        signer::EMPTY_ZIP,
        chain.leaf().key(),
    );
    let error = verify_bundle(&bytes, &test_trust()).expect_err("a truncated certificate");
    assert!(
        matches!(error, BundleError::MalformedCertificate { link: 0, .. }),
        "{error:?}"
    );
    assert!(!error.is_trust_failure());
}

#[test]
fn a_non_canonical_encoding_is_refused() {
    // The `basicConstraints` extension of a link, with its `critical` flag. The
    // only BOOLEAN the chain carries is that flag, and DER requires a true
    // BOOLEAN to be `FF` rather than the BER spelling `01`.
    const CRITICAL_BASIC_CONSTRAINTS: [u8; 8] = [0x06, 0x03, 0x55, 0x1D, 0x13, 0x01, 0x01, 0xFF];
    let chain = vector_chain();
    let mut leaf = chain.leaf().der().to_vec();
    let found = leaf
        .windows(CRITICAL_BASIC_CONSTRAINTS.len())
        .position(|window| window == CRITICAL_BASIC_CONSTRAINTS)
        .expect("the leaf carries a critical basicConstraints");
    let flag = found + CRITICAL_BASIC_CONSTRAINTS.len() - 1;
    assert_eq!(leaf[flag], 0xFF);
    leaf[flag] = 0x01;
    let bytes = bundle_for(&leaf, signer::EMPTY_ZIP, chain.leaf().key());
    let error = verify_bundle(&bytes, &test_trust()).expect_err("a strict decoder refuses this");
    assert!(
        matches!(error, BundleError::NonCanonicalDer { link: 0, .. }),
        "{error:?}"
    );
    assert!(!error.is_trust_failure());
}

#[test]
fn a_non_minimal_length_is_refused() {
    let chain = vector_chain();
    let leaf = chain.leaf().der();
    // The certificate's outer SEQUENCE, with its length written in three octets
    // where two suffice. The value is the same; its encoding is not DER's.
    assert_eq!(&leaf[..2], &[0x30, 0x82]);
    let mut non_canonical = vec![0x30, 0x83, 0x00, leaf[2], leaf[3]];
    non_canonical.extend_from_slice(&leaf[4..]);
    let bytes = bundle_for(&non_canonical, signer::EMPTY_ZIP, chain.leaf().key());
    let error = verify_bundle(&bytes, &test_trust()).expect_err("a strict decoder refuses this");
    assert!(
        matches!(error, BundleError::NonCanonicalDer { link: 0, .. }),
        "{error:?}"
    );
    assert!(!error.is_trust_failure());
}

// --- the chain profile ---------------------------------------------------

/// The standard three-link chain with one or more specifications changed.
///
/// The keys are still one per link, so the anchor is the same key as the
/// vector's and `test_trust` still trusts it.
fn chain_with(mutate: impl FnOnce(&mut Vec<signer::LinkSpec>)) -> TestChain {
    let mut specs = signer::standard_specs(3);
    mutate(&mut specs);
    let seeds: Vec<u8> = (1..=specs.len() as u8).collect();
    TestChain::build(&specs, &seeds)
}

#[test]
fn an_intermediate_without_the_ca_constraint_is_refused() {
    let chain = chain_with(|specs| specs[1].ca = None);
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    let error = verify_bundle(&bytes, &test_trust()).expect_err("link 1 authorises a key");
    assert_eq!(error, BundleError::CaConstraintMissing { link: 1 });
    assert!(error.is_trust_failure());
}

#[test]
fn an_anchor_without_the_ca_constraint_is_refused() {
    let chain = chain_with(|specs| specs[2].ca = None);
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    let error = verify_bundle(&bytes, &test_trust()).expect_err("link 2 authorises a key");
    assert_eq!(error, BundleError::CaConstraintMissing { link: 2 });
}

#[test]
fn a_leaf_that_authorises_another_key_is_refused() {
    let chain = chain_with(|specs| specs[0].ca = Some(true));
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    let error = verify_bundle(&bytes, &test_trust()).expect_err("the leaf must be an end entity");
    assert_eq!(error, BundleError::LeafIsCa);
    assert!(error.is_trust_failure());
}

#[test]
fn an_unknown_critical_extension_is_refused() {
    let chain = chain_with(|specs| {
        specs[1]
            .extensions
            .push(signer::extension("1.2.3.4.5", true, &[0x05, 0x00]));
    });
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    let error = verify_bundle(&bytes, &test_trust()).expect_err("the extension is critical");
    assert_eq!(
        error,
        BundleError::UnknownCriticalExtension {
            link: 1,
            oid: "1.2.3.4.5".to_string(),
        }
    );
    assert!(error.is_trust_failure());
}

#[test]
fn an_unknown_non_critical_extension_is_ignored() {
    let chain = chain_with(|specs| {
        specs[1]
            .extensions
            .push(signer::extension("1.2.3.4.5", false, &[0x05, 0x00]));
    });
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    verify_bundle(&bytes, &test_trust()).expect("a non-critical extension may be ignored");
}

#[test]
fn a_leaf_without_a_common_name_is_refused() {
    let chain = chain_with(|specs| specs[0].common_name = String::new());
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    let error = verify_bundle(&bytes, &test_trust()).expect_err("the name is checked");
    assert_eq!(error, BundleError::MissingCommonName);
    assert!(error.is_trust_failure());
}

// --- the links -----------------------------------------------------------

#[test]
fn a_broken_link_is_refused() {
    let chain = chain_with(|specs| specs[0].issuer_common_name = "Someone else".to_string());
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    let error = verify_bundle(&bytes, &test_trust()).expect_err("the names must chain");
    assert_eq!(error, BundleError::ChainBroken { child: 0 });
    assert!(error.is_trust_failure());
}

#[test]
fn a_link_signature_that_does_not_verify_is_refused() {
    let chain = vector_chain();
    // One byte of the leaf certificate's own signature, which the intermediate
    // is supposed to have made.
    let mut leaf = chain.leaf().der().to_vec();
    let last = leaf.len() - 1;
    leaf[last] ^= 0x01;
    let mut chain_bytes = leaf;
    chain_bytes.extend_from_slice(chain.link(1).der());
    chain_bytes.extend_from_slice(chain.anchor().der());
    let bytes = bundle_for(&chain_bytes, signer::EMPTY_ZIP, chain.leaf().key());
    let error = verify_bundle(&bytes, &test_trust()).expect_err("the link does not verify");
    assert_eq!(error, BundleError::ChainSignatureInvalid { child: 0 });
    assert!(error.is_trust_failure());
}

#[test]
fn a_payload_signed_by_another_key_is_refused() {
    let chain = vector_chain();
    let bytes = bundle_for(&chain.der(), signer::EMPTY_ZIP, chain.link(1).key());
    let error = verify_bundle(&bytes, &test_trust()).expect_err("the leaf key signs the payload");
    assert_eq!(error, BundleError::SignatureInvalid);
}

#[test]
fn a_signature_of_the_wrong_length_is_refused() {
    let chain = vector_chain();
    let chain_bytes = chain.der();
    let header = Header::standard(chain_bytes.len(), 99, signer::EMPTY_ZIP.len());
    let mut bytes = header.to_bytes().to_vec();
    bytes.extend_from_slice(&chain_bytes);
    bytes.extend_from_slice(&[0u8; 99]);
    bytes.extend_from_slice(signer::EMPTY_ZIP);
    let error = verify_bundle(&bytes, &test_trust()).expect_err("a signature is 64 bytes");
    assert_eq!(error, BundleError::MalformedSignature { length: 99 });
    assert!(error.is_trust_failure());
}

// --- revocation and the key id rule -------------------------------------

#[test]
fn a_revoked_leaf_key_is_refused() {
    let mut trust = test_trust();
    trust.revoke(key_id(&vector_chain().leaf().public_key()));
    let error = verify_bundle(VECTOR, &trust).expect_err("the leaf key is revoked");
    assert_eq!(
        error,
        BundleError::RevokedKey {
            key_id: key_id(&vector_chain().leaf().public_key()),
        }
    );
    assert!(error.is_trust_failure());
}

#[test]
fn a_revoked_anchor_key_is_refused() {
    let mut trust = test_trust();
    trust.revoke(key_id(&vector_chain().anchor().public_key()));
    let error = verify_bundle(VECTOR, &trust).expect_err("a revoked root anchors nothing");
    assert_eq!(
        error,
        BundleError::RevokedKey {
            key_id: key_id(&vector_chain().anchor().public_key()),
        }
    );
}

#[test]
fn a_key_id_carried_by_a_certificate_is_not_trusted() {
    // The leaf carries a key identifier, and the revocation list names exactly
    // that identifier. The verifier computes the key id from the key, so the
    // carried value decides nothing and the bundle still verifies.
    let carried = [0xAB; 16];
    let chain = chain_with(|specs| {
        specs[0]
            .extensions
            .push(signer::key_identifier_extension(&carried));
    });
    let mut trust = test_trust();
    trust.revoke(carried);
    // The revocation list names a key by the id computed from it.
    trust.revoke(key_id(&chain.anchor().public_key()));
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    let error = verify_bundle(&bytes, &trust).expect_err("the anchor really is revoked");
    assert_eq!(
        error,
        BundleError::RevokedKey {
            key_id: key_id(&chain.anchor().public_key()),
        },
        "the recomputed anchor id is what the list matches, not the carried one"
    );
    let trust_without_the_anchor = {
        let mut trust = test_trust();
        trust.revoke(carried);
        trust
    };
    verify_bundle(&bytes, &trust_without_the_anchor)
        .expect("the carried identifier decides nothing");
}

// --- validity -----------------------------------------------------------

#[test]
fn an_expired_certificate_is_refused() {
    let now = signer::now_seconds();
    let chain = chain_with(|specs| specs[1].validity = (now - 7200, now - 3600));
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    let error = verify_bundle(&bytes, &test_trust()).expect_err("the window has passed");
    assert_eq!(
        error,
        BundleError::Expired {
            link: 1,
            not_after: now - 3600,
        }
    );
    assert!(error.is_trust_failure());
}

#[test]
fn a_certificate_that_is_not_yet_valid_is_refused() {
    let now = signer::now_seconds();
    let chain = chain_with(|specs| specs[0].validity = (now + 3600, now + 7200));
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    let error = verify_bundle(&bytes, &test_trust()).expect_err("the window has not started");
    assert_eq!(
        error,
        BundleError::NotYetValid {
            link: 0,
            not_before: now + 3600,
        }
    );
    assert!(error.is_trust_failure());
}

#[test]
fn a_certificate_inside_the_clock_skew_is_accepted() {
    let now = signer::now_seconds();
    let chain = chain_with(|specs| specs[0].validity = (signer::NOT_BEFORE, now - 30));
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    verify_bundle(&bytes, &test_trust()).expect("30 seconds of skew is tolerated");
}

// --- the classification `startup-policy` reads ---------------------------

/// The class of a refusal, stated here rather than read from the build under
/// test.
///
/// This match has no wildcard arm on purpose: a new variant stops this file from
/// compiling until its class is decided, because the daemon's start-up policy
/// turns on it.
fn classified_as_a_trust_failure(error: &BundleError) -> bool {
    match error {
        // A file this build cannot read. There is no evidence of tampering, only
        // a bundle that does not work here, so the bundle degrades.
        BundleError::Truncated { .. }
        | BundleError::BadMagic { .. }
        | BundleError::UnknownContainerVersion { .. }
        | BundleError::UnknownSigningSuite { .. }
        | BundleError::BadHeaderLength { .. }
        | BundleError::ReservedNotZero { .. }
        | BundleError::BytesBeforeHeader { .. }
        | BundleError::ChainSpanMismatch { .. }
        | BundleError::SpanMismatch { .. }
        | BundleError::ChainTooLong { .. }
        | BundleError::TrailingChainBytes { .. }
        | BundleError::NonCanonicalDer { .. }
        | BundleError::MalformedCertificate { .. } => false,
        // A file that claims a signature this daemon does not trust. Start-up
        // stops, because a bundle claiming an untrusted signature is evidence
        // that something is wrong.
        BundleError::EmptyChain
        | BundleError::UnknownRoot { .. }
        | BundleError::RevokedKey { .. }
        | BundleError::Expired { .. }
        | BundleError::NotYetValid { .. }
        | BundleError::CaConstraintMissing { .. }
        | BundleError::LeafIsCa
        | BundleError::UnknownCriticalExtension { .. }
        | BundleError::ChainBroken { .. }
        | BundleError::ChainSignatureInvalid { .. }
        | BundleError::MalformedSignature { .. }
        | BundleError::SignatureInvalid
        | BundleError::MissingCommonName => true,
    }
}

#[test]
fn every_refusal_is_classified() {
    let refusals = [
        BundleError::Truncated { len: 0 },
        BundleError::BadMagic { found: [0; 8] },
        BundleError::UnknownContainerVersion { found: 2 },
        BundleError::UnknownSigningSuite { found: 2 },
        BundleError::BadHeaderLength { found: 41 },
        BundleError::ReservedNotZero { found: 1 },
        BundleError::BytesBeforeHeader {
            signature_offset: 20,
            header_length: 40,
        },
        BundleError::ChainSpanMismatch {
            chain_length: 800,
            signature_offset: 808,
        },
        BundleError::SpanMismatch {
            file_length: 894,
            signature_offset: 808,
            signature_length: 64,
            zip_length: 23,
        },
        BundleError::EmptyChain,
        BundleError::ChainTooLong { links: 9 },
        BundleError::TrailingChainBytes { bytes: 1 },
        BundleError::NonCanonicalDer {
            link: 0,
            detail: String::new(),
        },
        BundleError::MalformedCertificate {
            link: 0,
            detail: String::new(),
        },
        BundleError::UnknownRoot { key_id: [0; 16] },
        BundleError::RevokedKey { key_id: [0; 16] },
        BundleError::Expired {
            link: 0,
            not_after: 0,
        },
        BundleError::NotYetValid {
            link: 0,
            not_before: 0,
        },
        BundleError::CaConstraintMissing { link: 1 },
        BundleError::LeafIsCa,
        BundleError::UnknownCriticalExtension {
            link: 1,
            oid: "1.2.3.4.5".to_string(),
        },
        BundleError::ChainBroken { child: 0 },
        BundleError::ChainSignatureInvalid { child: 0 },
        BundleError::MalformedSignature { length: 99 },
        BundleError::SignatureInvalid,
        BundleError::MissingCommonName,
    ];
    for refusal in &refusals {
        assert_eq!(
            refusal.is_trust_failure(),
            classified_as_a_trust_failure(refusal),
            "{refusal:?}"
        );
        assert!(
            !refusal.to_string().is_empty(),
            "{refusal:?} has no message"
        );
    }
}

// --- boundaries ----------------------------------------------------------

#[test]
fn one_link_chain_verifies_when_its_key_is_a_root() {
    // A producer may sign with a root key directly: one link, which is the leaf
    // and the anchor at once.
    let chain = TestChain::standard(1);
    let bytes = signer::signed_bundle(&chain, signer::EMPTY_ZIP);
    let trust = TrustStore::empty().with_root(chain.anchor().public_key());
    let verified = verify_bundle(&bytes, &trust).expect("a root key may sign a payload");
    assert_eq!(verified.leaf_common_name(), signer::LEAF_COMMON_NAME);
}

#[test]
fn the_returned_name_is_the_leafs_and_not_another_links() {
    let verified = verify_bundle(VECTOR, &test_trust()).expect("a valid bundle verifies");
    assert_eq!(verified.leaf_common_name(), signer::LEAF_COMMON_NAME);
    assert_ne!(
        verified.leaf_common_name(),
        signer::standard_common_name(1, 3),
        "the intermediate's name is not returned"
    );
    assert_ne!(
        verified.leaf_common_name(),
        signer::standard_common_name(2, 3),
        "the anchor's name is not returned"
    );
}

#[test]
fn a_key_id_is_the_first_sixteen_bytes_of_the_keys_sha256() {
    // A known answer, computed from the definition rather than from this build:
    // SHA-256 of the bytes 00 through 1F, whose first sixteen bytes these are.
    let key: [u8; 32] = std::array::from_fn(|index| index as u8);
    assert_eq!(
        key_id(&key),
        [
            0x63, 0x0d, 0xcd, 0x29, 0x66, 0xc4, 0x33, 0x66, 0x91, 0x12, 0x54, 0x48, 0xbb, 0xb2,
            0x5b, 0x4f,
        ]
    );
}
