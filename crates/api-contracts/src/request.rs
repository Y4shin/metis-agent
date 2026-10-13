//! The request identifier a caller declares on a call it may want to stop.
//!
//! The identifier travels as request metadata and comes back in the response
//! metadata, so a caller correlates an answer with the call it made. It is also
//! the name cancellation uses: the registry keys in-flight work by the declaring
//! surface and this identifier, and the `Cancel` RPC stops the work under it.

use std::fmt;

use http::HeaderMap;

/// The request header carrying the identifier a caller declares.
pub const REQUEST_ID_HEADER: &str = "metis-request-id";

/// How many bytes an identifier carries at most.
const MAX_REQUEST_ID_BYTES: usize = 64;

/// An identifier a caller declared.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestId(String);

/// The declaration could not be read.
///
/// The enum is non-exhaustive: a later rule adds its own refusal without
/// breaking a caller that matches on it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestIdError {
    /// The identifier is not one this build accepts.
    Malformed,
}

impl fmt::Display for RequestIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => f.write_str("the request identifier is malformed"),
        }
    }
}

impl std::error::Error for RequestIdError {}

impl RequestId {
    /// Read an identifier from the text it is written as.
    ///
    /// The rule is at most 64 bytes, every byte in `0x21..=0x7E`: an identifier
    /// is header-safe, bounded, and printable, so it can travel as request
    /// metadata, come back as response metadata, and be named in a log without
    /// escaping.
    ///
    /// # Errors
    ///
    /// Returns [`RequestIdError::Malformed`] when the text is empty, longer
    /// than the bound, or carries a byte outside the printable range.
    pub fn parse(value: &str) -> Result<Self, RequestIdError> {
        let printable = value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b' ');
        if value.is_empty() || value.len() > MAX_REQUEST_ID_BYTES || !printable {
            return Err(RequestIdError::Malformed);
        }
        Ok(Self(value.to_owned()))
    }

    /// The identifier as it crosses the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Read the identifier a call declared, or `None` when it declared none.
    ///
    /// `None` is legal: a call that declares no identifier is not cancellable,
    /// and every call behaves as it did before the identifier existed.
    ///
    /// # Errors
    ///
    /// Returns [`RequestIdError::Malformed`] when a declared identifier is not
    /// readable text or not one this build accepts. A declaration the daemon
    /// cannot act on is refused rather than ignored, because a caller that
    /// believes its work is cancellable and finds out otherwise has lost the
    /// guarantee it asked for.
    pub fn from_headers(headers: &HeaderMap) -> Result<Option<Self>, RequestIdError> {
        let Some(value) = headers.get(REQUEST_ID_HEADER) else {
            return Ok(None);
        };
        let text = value.to_str().map_err(|_| RequestIdError::Malformed)?;
        Self::parse(text).map(Some)
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{REQUEST_ID_HEADER, RequestId, RequestIdError};
    use http::{HeaderMap, HeaderValue};

    /// An identifier is bounded and printable: at most 64 bytes, every byte in
    /// `0x21..=0x7E`. An empty one names nothing, one byte past the bound is
    /// refused, and every byte a header value cannot carry safely is refused.
    #[test]
    fn an_identifier_is_bounded_and_printable() {
        let id = RequestId::parse("cli-7/abc_9").expect("a printable identifier is accepted");
        assert_eq!(id.as_str(), "cli-7/abc_9");
        assert_eq!(id.to_string(), "cli-7/abc_9");
        assert_eq!(
            RequestId::parse(&"a".repeat(64))
                .expect("64 bytes is the bound")
                .as_str()
                .len(),
            64
        );

        assert_eq!(
            RequestId::parse(""),
            Err(RequestIdError::Malformed),
            "an empty identifier names nothing"
        );
        assert_eq!(
            RequestId::parse(&"a".repeat(65)),
            Err(RequestIdError::Malformed),
            "one byte past the bound"
        );
        assert_eq!(
            RequestId::parse("a b"),
            Err(RequestIdError::Malformed),
            "a space is outside the printable range"
        );
        assert_eq!(
            RequestId::parse("a\tb"),
            Err(RequestIdError::Malformed),
            "a control byte is outside the printable range"
        );
        assert_eq!(
            RequestId::parse("a\u{7f}"),
            Err(RequestIdError::Malformed),
            "0x7F is outside the range, one byte above it"
        );
    }

    /// A declared identifier reads off the request metadata, and a call that
    /// declares none is legal rather than refused: it is simply not cancellable.
    #[test]
    fn a_declared_identifier_reads_off_the_request_metadata() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            RequestId::from_headers(&headers),
            Ok(None),
            "a call may declare no identifier"
        );

        headers.insert(REQUEST_ID_HEADER, HeaderValue::from_static("cli-7"));
        assert_eq!(
            RequestId::from_headers(&headers)
                .expect("the declared identifier is readable")
                .map(|id| id.as_str().to_owned()),
            Some("cli-7".to_owned())
        );

        headers.insert(REQUEST_ID_HEADER, HeaderValue::from_static("bad id"));
        assert_eq!(
            RequestId::from_headers(&headers),
            Err(RequestIdError::Malformed),
            "a malformed declaration is refused rather than ignored"
        );
    }
}
