//! The identity a client declares on every call, and the version gate.
//!
//! The declaration travels as request metadata on every call. It is not a
//! handshake RPC and not state on the connection: the same channel carries a
//! credential later, and a stream needs the declaration per request anyway. It
//! is a declaration, not authentication, and reading it grants nothing.
//!
//! Three request headers carry it: [`PROTOCOL_VERSION_HEADER`],
//! [`PROTOCOL_SURFACE_HEADER`], and [`PROTOCOL_CLIENT_VERSION_HEADER`]. A
//! refusal is an `InvalidArgument` error whose response carries
//! [`SUPPORTED_VERSIONS_HEADER`], so a client learns which versions the daemon
//! supports from a header instead of from message text it would have to parse.

use std::fmt;

use http::{HeaderMap, HeaderValue};

/// The protocol version this build speaks.
pub const PROTOCOL_VERSION: u32 = 1;

/// The protocol versions this build accepts, in the order a refusal lists them.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[u32] = &[PROTOCOL_VERSION];

/// The request header carrying the protocol version the client speaks.
pub const PROTOCOL_VERSION_HEADER: &str = "metis-protocol-version";

/// The request header carrying the surface the client speaks for.
pub const PROTOCOL_SURFACE_HEADER: &str = "metis-surface";

/// The request header carrying the client's own version.
pub const PROTOCOL_CLIENT_VERSION_HEADER: &str = "metis-client-version";

/// The response header on a refusal, carrying the versions the daemon supports.
///
/// The typed refusal crosses the wire as this header, so the client never
/// parses a message to learn what is supported.
pub const SUPPORTED_VERSIONS_HEADER: &str = "metis-supported-versions";

/// The identity a client declared on one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIdentity {
    protocol_version: u32,
    surface: String,
    client_version: String,
}

impl ClientIdentity {
    /// Build an identity from its three parts.
    #[must_use]
    pub fn new(
        protocol_version: u32,
        surface: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Self {
        Self {
            protocol_version,
            surface: surface.into(),
            client_version: client_version.into(),
        }
    }

    /// The protocol version the client declared.
    #[must_use]
    pub fn protocol_version(&self) -> u32 {
        self.protocol_version
    }

    /// The surface the client speaks for.
    #[must_use]
    pub fn surface(&self) -> &str {
        &self.surface
    }

    /// The client's own version.
    #[must_use]
    pub fn client_version(&self) -> &str {
        &self.client_version
    }
}

impl fmt::Display for ClientIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "surface {} version {} (protocol {})",
            self.surface, self.client_version, self.protocol_version
        )
    }
}

/// The declaration could not be read.
///
/// The enum is non-exhaustive: a later ticket adds its own declaration
/// failures without breaking a caller that matches on it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdentityError {
    /// The request carries no such header, or its value is not readable text.
    MissingHeader {
        /// The header that is absent.
        header: &'static str,
    },
    /// The declared protocol version is not one this build supports.
    UnsupportedVersion {
        /// The version the client declared.
        found: u32,
        /// The versions this build supports.
        supported: &'static [u32],
    },
    /// The version header does not carry a number.
    MalformedVersion {
        /// The value that was received.
        value: String,
    },
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHeader { header } => {
                write!(f, "the request carries no `{header}` header")
            }
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "protocol version {found} is not supported; supported versions: {}",
                list_versions(supported)
            ),
            Self::MalformedVersion { value } => write!(
                f,
                "{PROTOCOL_VERSION_HEADER} is not a number: {value:?}; supported versions: {}",
                list_versions(SUPPORTED_PROTOCOL_VERSIONS)
            ),
        }
    }
}

impl std::error::Error for IdentityError {}

/// Read the declared identity off a request's headers.
///
/// The version is checked first, so a client that declares a version this build
/// does not speak is refused for the version and not for a header the version
/// check would have made irrelevant. The surface name and the client version
/// are recorded as declared: they are descriptive, and nothing is decided from
/// them.
///
/// # Errors
///
/// Returns [`IdentityError::MissingHeader`] when a declared field is absent,
/// [`IdentityError::MalformedVersion`] when the version is not a number, and
/// [`IdentityError::UnsupportedVersion`] when the version is a number this
/// build does not support.
pub fn from_headers(headers: &HeaderMap) -> Result<ClientIdentity, IdentityError> {
    let declared =
        header_str(headers, PROTOCOL_VERSION_HEADER).ok_or(IdentityError::MissingHeader {
            header: PROTOCOL_VERSION_HEADER,
        })?;
    let protocol_version =
        declared
            .trim()
            .parse::<u32>()
            .map_err(|_| IdentityError::MalformedVersion {
                value: declared.to_owned(),
            })?;
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&protocol_version) {
        return Err(IdentityError::UnsupportedVersion {
            found: protocol_version,
            supported: SUPPORTED_PROTOCOL_VERSIONS,
        });
    }
    let surface =
        header_str(headers, PROTOCOL_SURFACE_HEADER).ok_or(IdentityError::MissingHeader {
            header: PROTOCOL_SURFACE_HEADER,
        })?;
    let client_version = header_str(headers, PROTOCOL_CLIENT_VERSION_HEADER).ok_or(
        IdentityError::MissingHeader {
            header: PROTOCOL_CLIENT_VERSION_HEADER,
        },
    )?;
    Ok(ClientIdentity::new(
        protocol_version,
        surface,
        client_version,
    ))
}

/// The response headers a refusal carries.
///
/// A version refusal lists the supported versions, so the client can name what
/// the daemon does speak; every other refusal carries no extra header, because
/// a missing surface or client version has nothing to list. The caller stores
/// the map on its protocol error, and it reaches the client as response
/// metadata.
#[must_use]
pub fn refusal_headers(error: &IdentityError) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let lists_versions = matches!(
        error,
        IdentityError::UnsupportedVersion { .. } | IdentityError::MalformedVersion { .. }
    ) || matches!(error, IdentityError::MissingHeader { header } if *header == PROTOCOL_VERSION_HEADER);
    if lists_versions {
        headers.insert(
            SUPPORTED_VERSIONS_HEADER,
            HeaderValue::from_str(&supported_versions_value())
                .expect("the supported list is a valid header value"),
        );
    }
    headers
}

/// The value of [`SUPPORTED_VERSIONS_HEADER`]: the supported versions, comma
/// separated.
#[must_use]
pub fn supported_versions_value() -> String {
    list_versions(SUPPORTED_PROTOCOL_VERSIONS)
}

/// Read a [`SUPPORTED_VERSIONS_HEADER`] value back into versions.
///
/// Unknown parts are skipped, so a daemon that grows a version this client does
/// not know still names the ones it does.
#[must_use]
pub fn parse_supported_versions(value: &str) -> Vec<u32> {
    value
        .split(',')
        .filter_map(|part| part.trim().parse::<u32>().ok())
        .collect()
}

/// A version list as the refusal carries it.
fn list_versions(versions: &[u32]) -> String {
    versions
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// One header as text, or `None` when it is absent or not readable text.
fn header_str<'a>(headers: &'a HeaderMap, name: &'static str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod tests {
    use super::{
        ClientIdentity, IdentityError, PROTOCOL_CLIENT_VERSION_HEADER, PROTOCOL_SURFACE_HEADER,
        PROTOCOL_VERSION, PROTOCOL_VERSION_HEADER, SUPPORTED_PROTOCOL_VERSIONS,
        SUPPORTED_VERSIONS_HEADER, from_headers, parse_supported_versions, refusal_headers,
        supported_versions_value,
    };
    use http::{HeaderMap, HeaderName, HeaderValue};

    /// A declaration with the three headers, overridden per test.
    fn declared(version: Option<&str>, surface: Option<&str>, client: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in [
            (PROTOCOL_VERSION_HEADER, version),
            (PROTOCOL_SURFACE_HEADER, surface),
            (PROTOCOL_CLIENT_VERSION_HEADER, client),
        ] {
            if let Some(value) = value {
                headers.insert(
                    HeaderName::from_static(name),
                    HeaderValue::from_str(value).expect("a test value is a valid header value"),
                );
            }
        }
        headers
    }

    /// The three declared headers are the identity, read as declared and
    /// nothing more.
    #[test]
    fn the_three_headers_are_the_declared_identity() {
        let identity = from_headers(&declared(Some("1"), Some("cli"), Some("0.0.0")))
            .expect("a complete declaration is readable");
        assert_eq!(identity, ClientIdentity::new(1, "cli", "0.0.0"));
        assert_eq!(identity.protocol_version(), PROTOCOL_VERSION);
        assert_eq!(identity.surface(), "cli");
        assert_eq!(identity.client_version(), "0.0.0");
        assert_eq!(
            identity.surface(),
            "cli",
            "the surface is recorded as declared"
        );
    }

    /// Every declared field is required, and the error names the one that is
    /// absent.
    #[test]
    fn a_missing_header_is_named() {
        for (headers, missing) in [
            (
                declared(None, Some("cli"), Some("0.0.0")),
                PROTOCOL_VERSION_HEADER,
            ),
            (
                declared(Some("1"), None, Some("0.0.0")),
                PROTOCOL_SURFACE_HEADER,
            ),
            (
                declared(Some("1"), Some("cli"), None),
                PROTOCOL_CLIENT_VERSION_HEADER,
            ),
        ] {
            let error = from_headers(&headers).expect_err("an incomplete declaration is refused");
            assert_eq!(
                error,
                IdentityError::MissingHeader { header: missing },
                "the error names the missing header"
            );
            assert!(
                error.to_string().contains(missing),
                "the message names it too: {error}"
            );
        }
    }

    /// A version this build does not speak is refused, and the typed error
    /// lists what it does speak.
    #[test]
    fn an_unsupported_version_names_what_is_supported() {
        let error = from_headers(&declared(Some("2"), Some("cli"), Some("0.0.0")))
            .expect_err("version 2 is not supported");
        assert_eq!(
            error,
            IdentityError::UnsupportedVersion {
                found: 2,
                supported: SUPPORTED_PROTOCOL_VERSIONS,
            }
        );
        let message = error.to_string();
        assert!(message.contains('2'), "the message names what was found");
        assert!(
            message.contains('1'),
            "the message names what is supported: {message}"
        );
    }

    /// The version is checked first, so a client that declares a version this
    /// build does not speak is refused for the version even when it left the
    /// other headers off.
    #[test]
    fn the_version_is_checked_before_the_other_headers() {
        let error = from_headers(&declared(Some("2"), None, None))
            .expect_err("the version is refused first");
        assert!(matches!(
            error,
            IdentityError::UnsupportedVersion { found: 2, .. }
        ));
    }

    /// A version header that is not a number is refused as a malformed
    /// declaration rather than silently read as a version.
    #[test]
    fn a_version_that_is_not_a_number_is_refused() {
        for value in ["two", "", "1.0", "  "] {
            let error = from_headers(&declared(Some(value), Some("cli"), Some("0.0.0")))
                .expect_err("a non-numeric version is refused");
            assert_eq!(
                error,
                IdentityError::MalformedVersion {
                    value: value.to_owned(),
                }
            );
        }
    }

    /// Surrounding whitespace is tolerated: a header value is text, and the
    /// number in it is the version.
    #[test]
    fn a_padded_version_is_still_the_version() {
        let identity = from_headers(&declared(Some(" 1 "), Some("ui"), Some("0.0.0")))
            .expect("the padded version is readable");
        assert_eq!(identity.protocol_version(), 1);
    }

    /// The refusal carries the supported versions as a header, which is how the
    /// typed error crosses the wire; a refusal about a non-version header
    /// carries nothing extra.
    #[test]
    fn only_a_version_refusal_lists_the_supported_versions() {
        let version_refusal = refusal_headers(&IdentityError::UnsupportedVersion {
            found: 2,
            supported: SUPPORTED_PROTOCOL_VERSIONS,
        });
        assert_eq!(
            version_refusal.get(SUPPORTED_VERSIONS_HEADER),
            Some(&HeaderValue::from_str(&supported_versions_value()).expect("a valid value"))
        );

        let missing_version = refusal_headers(&IdentityError::MissingHeader {
            header: PROTOCOL_VERSION_HEADER,
        });
        assert!(
            missing_version.contains_key(SUPPORTED_VERSIONS_HEADER),
            "a client that declares no version is told what is supported"
        );

        let missing_surface = refusal_headers(&IdentityError::MissingHeader {
            header: PROTOCOL_SURFACE_HEADER,
        });
        assert!(
            missing_surface.is_empty(),
            "a missing surface header has nothing to list"
        );
    }

    /// The version list round-trips, and a client that meets a newer daemon
    /// still reads the versions it knows.
    #[test]
    fn the_supported_versions_round_trip() {
        assert_eq!(supported_versions_value(), "1");
        assert_eq!(
            parse_supported_versions(&supported_versions_value()),
            vec![1]
        );
        assert_eq!(parse_supported_versions("1, 2"), vec![1, 2]);
        assert_eq!(parse_supported_versions("2,1"), vec![2, 1]);
        assert_eq!(
            parse_supported_versions("bogus, 1"),
            vec![1],
            "unknown parts are skipped"
        );
        assert_eq!(parse_supported_versions(""), Vec::<u32>::new());
    }
}
