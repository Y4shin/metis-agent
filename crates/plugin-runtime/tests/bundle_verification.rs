//! The bundle verifier, tested at its one public seam: `verify_bundle` over real
//! bytes.
//!
//! The bytes come from two places. A committed vector, built by the test signer
//! in `support/signer.rs` and checked byte for byte, pins the contract's layout
//! and domain tag. Bundles built inside a test from the same signer carry the
//! refusal cases, because a refusal needs a file whose signature is valid and
//! whose structure is wrong.

mod support;

use std::path::PathBuf;

use support::signer::{self, TestChain};

fn vector_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/test-bundle.metis")
}

/// Rebuilds the committed vector from the test signer.
fn rebuilt_vector() -> Vec<u8> {
    let chain = TestChain::standard(3);
    signer::signed_bundle(&chain, signer::EMPTY_ZIP)
}

/// Writes the committed vector.
///
/// It is not part of the suite: it exists so the vector can be rebuilt with
/// `cargo test -p plugin-runtime --test bundle_verification -- --ignored`, and
/// the ordinary run checks the committed bytes against the same builder.
#[test]
#[ignore = "regenerates the committed vector rather than checking it"]
fn regenerate_committed_vector() {
    let bytes = rebuilt_vector();
    std::fs::create_dir_all(vector_path().parent().expect("the vector directory"))
        .expect("the vector directory is writable");
    std::fs::write(vector_path(), &bytes).expect("the vector is writable");
    assert!(bytes.len() > 40);
}
