//! The credential's wire vocabulary: where it travels and how a refusal is
//! spelled.
//!
//! One module owns both ends, the way `identity` owns the version-refusal
//! channel. The daemon refuses a call through [`AUTH_ERROR_HEADER`] and the
//! client reads the same header, so neither piece of the spelling can drift
//! from the other.
//!
//! The credential travels as request metadata on every call and never in a URL:
//! the header's value is the only place the token appears on the wire, and the
//! preshared key is the one secret that travels in a message body, inside
//! `PairRequest`.

use std::fmt;

/// The request header a credential travels in, as `Bearer <token>`.
pub const AUTHORIZATION_HEADER: &str = "authorization";

/// The scheme a credential is presented with.
pub const BEARER_SCHEME: &str = "Bearer";

/// The response header a credential refusal carries.
///
/// A refusal's meaning crosses the wire as this header rather than as message
/// text, so a client maps it to a typed error instead of parsing prose.
pub const AUTH_ERROR_HEADER: &str = "metis-auth-error";

/// How a credential was refused, as it crosses the wire.
///
/// The enum is non-exhaustive: a later credential form adds its own refusals
/// without breaking a caller that matches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialRefusal {
    /// No credential was presented, so the caller has to pair.
    Missing,
    /// A credential was presented, and it is not the one the surface holds.
    Invalid,
    /// The surface the credential names has no token: it was never paired, or
    /// its token was revoked.
    Revoked,
}

impl CredentialRefusal {
    /// The header value this refusal crosses the wire as.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Invalid => "invalid",
            Self::Revoked => "revoked",
        }
    }

    /// Read a refusal from the header value it crossed the wire as.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "missing" => Some(Self::Missing),
            "invalid" => Some(Self::Invalid),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

impl fmt::Display for CredentialRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `Authorization` header value a token travels as.
#[must_use]
pub fn bearer(token: &str) -> String {
    format!("{BEARER_SCHEME} {token}")
}

/// The token text a bearer `Authorization` header value carries.
///
/// The scheme is matched case-insensitively, because HTTP authentication
/// schemes are, and `None` means the header is not a bearer credential at all,
/// which the daemon reports as an invalid credential rather than a missing one.
#[must_use]
pub fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case(BEARER_SCHEME) {
        return None;
    }
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    Some(token)
}
