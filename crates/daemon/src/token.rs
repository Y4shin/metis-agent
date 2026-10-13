//! The per-surface token: 32 random bytes from the operating system's source,
//! carried as lowercase hex text, and stored only as the SHA-256 of that text.
//!
//! The token is what a client holds and presents. The daemon keeps the
//! [`TokenHash`], compares it in constant time, and never writes the token
//! itself anywhere.

use std::fmt;

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// The number of random bytes a token carries.
const TOKEN_BYTES: usize = 32;

/// The number of hex characters the token's canonical text form carries.
const TOKEN_HEX_LEN: usize = TOKEN_BYTES * 2;

/// A per-surface credential.
///
/// The token is 32 random bytes in their canonical lowercase hex form, which is
/// the form it travels in and the form it is hashed in. Construct one with
/// [`mint`] to issue a new token, or with [`Token::parse`] to read the text a
/// caller presented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// Read a token from the text a caller presented.
    ///
    /// The text is exactly 64 hexadecimal characters, in either case. The token
    /// keeps the canonical lowercase form, so the hash does not depend on how
    /// the caller wrote the text.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError::Malformed`] when the text is not 64 hexadecimal
    /// characters.
    pub fn parse(text: &str) -> Result<Self, TokenError> {
        if text.len() != TOKEN_HEX_LEN || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(TokenError::Malformed);
        }
        Ok(Self(text.to_ascii_lowercase()))
    }

    /// The token's canonical text form, which is what a client presents.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The SHA-256 of the token's canonical text form.
    ///
    /// This is what the store keeps and what a verification compares against.
    /// The hash is taken over the text rather than the bytes it decodes to,
    /// because the text is what a client sends.
    #[must_use]
    pub fn hash(&self) -> TokenHash {
        let digest = Sha256::digest(self.0.as_bytes());
        let mut bytes = [0_u8; TOKEN_BYTES];
        bytes.copy_from_slice(&digest);
        TokenHash(bytes)
    }
}

/// Mint a token from the operating system's random source.
///
/// The token is returned once together with its hash: the caller hands the
/// token to the surface it belongs to and stores only the hash.
///
/// # Panics
///
/// Panics when the operating system's random source fails. A credential that
/// silently repeats is worse than a daemon that refuses to start.
#[must_use]
pub fn mint() -> (Token, TokenHash) {
    let mut bytes = [0_u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).expect("the operating system's random source is available");
    let token = Token(hex(&bytes));
    let hash = token.hash();
    (token, hash)
}

/// The SHA-256 of a token's canonical text form.
///
/// Equality is constant time, so verifying a presented token does not leak how
/// much of a stored hash matched. The bytes are the form the store keeps, so a
/// row and a presented token are compared without either being re-encoded.
#[derive(Debug, Clone, Copy)]
pub struct TokenHash([u8; TOKEN_BYTES]);

impl TokenHash {
    /// The digest bytes, as the store keeps them.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; TOKEN_BYTES] {
        &self.0
    }
}

impl PartialEq for TokenHash {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_slice().ct_eq(other.0.as_slice()).into()
    }
}

impl Eq for TokenHash {}

/// The text a caller presented is not a token.
///
/// The enum is non-exhaustive: a later credential form adds its own refusals
/// without breaking a caller that matches on it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TokenError {
    /// The text is not 64 hexadecimal characters, so it cannot be a token.
    Malformed,
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => write!(f, "a token is {TOKEN_HEX_LEN} hexadecimal characters"),
        }
    }
}

impl std::error::Error for TokenError {}

/// The lowercase hex form of `bytes`.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(text, "{byte:02x}").expect("writing to a string cannot fail");
    }
    text
}
