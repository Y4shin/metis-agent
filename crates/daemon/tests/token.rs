//! The per-surface token: minted from the operating system's random source,
//! carried as hex text, and hashed with SHA-256 before it is ever stored.
//!
//! The hash's input is pinned by a known-answer digest computed outside this
//! code, because the client hashes the token it holds the same way and the two
//! must agree.

use daemon::token::{Token, mint};

/// A fixed token text and its SHA-256, computed independently with
/// `sha256sum`: the digest of the 64 ASCII characters of the text.
const KNOWN_HEX_TEXT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const KNOWN_DIGEST: &str = "a8ae6e6ee929abea3afcfc5258c8ccd6f85273e0d4626d26c7279f3250f77c8e";

/// A digest, as lowercase hex, written independently of the crate's own
/// encoding so the comparison is not circular.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A minted token is 64 lowercase hex characters, and two mints differ: the
/// bytes come from the operating system's random source.
#[test]
fn a_minted_token_is_hex_text_and_each_mint_differs() {
    let (token, hash) = mint();
    assert_eq!(
        token.as_str().len(),
        64,
        "32 random bytes are 64 hex characters"
    );
    assert!(
        token
            .as_str()
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character)),
        "the canonical form is lowercase hex: {}",
        token.as_str()
    );
    assert_eq!(
        hash.as_bytes().len(),
        32,
        "a mint carries its 32-byte digest"
    );

    let (other, other_hash) = mint();
    assert_ne!(token, other, "two mints are two tokens");
    assert_ne!(hash, other_hash, "two tokens hash differently");
}

/// The hash is the SHA-256 of the token's canonical text, which is what the
/// daemon receives from a client and what it compares against the stored row.
#[test]
fn the_hash_is_the_sha256_of_the_canonical_text() {
    let token = Token::parse(KNOWN_HEX_TEXT).expect("64 hex characters are a token");
    let hash = token.hash();

    assert_eq!(hash.as_bytes().len(), 32, "SHA-256 is 32 bytes");
    assert_eq!(hex(hash.as_bytes()), KNOWN_DIGEST);
    assert_eq!(
        Token::parse(KNOWN_HEX_TEXT)
            .expect("the same text parses")
            .hash(),
        hash,
        "the hash is stable"
    );
}

/// Parsing accepts the hex text a caller presents, in either case, and keeps
/// the canonical lowercase form, so the hash does not depend on how the text
/// was written. Anything that is not 64 hex characters is refused.
#[test]
fn parsing_accepts_hex_text_in_either_case_and_refuses_anything_else() {
    let canonical = Token::parse(KNOWN_HEX_TEXT).expect("lowercase hex parses");
    let uppercase =
        Token::parse(&KNOWN_HEX_TEXT.to_ascii_uppercase()).expect("uppercase hex parses");
    assert_eq!(
        uppercase.as_str(),
        KNOWN_HEX_TEXT,
        "the canonical form is lowercase"
    );
    assert_eq!(
        uppercase.hash(),
        canonical.hash(),
        "case does not change it"
    );

    let shorter = &KNOWN_HEX_TEXT[..63];
    let longer = format!("{KNOWN_HEX_TEXT}0");
    let not_hex = "z".repeat(64);
    for text in ["", "0", shorter, longer.as_str(), not_hex.as_str()] {
        assert!(
            Token::parse(text).is_err(),
            "{} characters of {:?} are not a token",
            text.len(),
            text
        );
    }
}

/// The credential is never printed: a token's `Debug` form redacts its text, so
/// an accidental `{:?}` cannot put it in a log line.
#[test]
fn a_token_does_not_print_itself() {
    let token = Token::parse(KNOWN_HEX_TEXT).expect("the known text parses");
    let printed = format!("{token:?}");
    assert!(
        !printed.contains(KNOWN_HEX_TEXT),
        "the text is redacted: {printed}"
    );
    assert!(
        printed.contains("redacted"),
        "the printed form says it is redacted: {printed}"
    );
}
