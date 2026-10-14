//! Reverse-proxy mode: the trusted-proxy rule and the plaintext warning.
//!
//! ADR 0004 leaves TLS to a reverse proxy, so this daemon stays plaintext and
//! must not claim an encryption it cannot see. Forwarded information is read
//! only from the proxies the operator names in [`TRUSTED_PROXIES_ENV`]: a
//! forwarded header from any other peer is ignored, because anything that can
//! open a connection could otherwise name its own address and scheme.
//!
//! Everything here is a decision over a peer, the trusted set, and the request
//! headers, so a listener and a handler reach the same answer instead of each
//! restating the rule. The module sits beside `transport` because both the
//! listener front-ends and the daemon's handlers reach `api-contracts`.

/// The environment variable that names the addresses trusted to send forwarded
/// headers.
///
/// Entries are exact addresses, comma separated, and whitespace is tolerated. An
/// absent or empty value names no proxy, which is the default: a proxy is
/// trusted because an operator said so, never because it is loopback, is
/// private, or happened to connect first.
pub const TRUSTED_PROXIES_ENV: &str = "METIS_TRUSTED_PROXIES";

use std::fmt;
use std::net::IpAddr;

use http::HeaderMap;

use crate::transport::ConfigError;

/// The header a reverse proxy appends the client's address to.
const FORWARDED_FOR: &str = "x-forwarded-for";

/// The header a reverse proxy names the scheme it was reached with in.
const FORWARDED_PROTO: &str = "x-forwarded-proto";

/// The forwarded scheme that says the leg before the proxy is encrypted.
const HTTPS: &str = "https";

/// The addresses trusted to send forwarded headers.
///
/// The set is exact: an address is trusted because an operator named it, not
/// because it is loopback, is private, or is the address this daemon is bound
/// to. Membership is a comparison of addresses, so a peer that connects from
/// another source address is not the proxy that was named.
#[derive(Debug, Clone)]
pub struct TrustedProxies(Vec<IpAddr>);

impl TrustedProxies {
    /// Read the trusted-proxy set from the environment.
    ///
    /// An absent or empty [`TRUSTED_PROXIES_ENV`] names no proxy. A value that
    /// holds something other than addresses is a typed error rather than a
    /// silent empty set, so a typo is loud instead of looking like a
    /// deliberately closed configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidTrustedProxy`] when an entry is not an IP
    /// literal.
    pub fn resolve() -> Result<Self, ConfigError> {
        match std::env::var_os(TRUSTED_PROXIES_ENV) {
            Some(value) if !value.is_empty() => Self::parse(&value.to_string_lossy()),
            _ => Ok(Self(Vec::new())),
        }
    }

    /// Parse a comma-separated list of addresses, whitespace tolerated.
    ///
    /// An empty or whitespace-only value names no proxy, and an empty entry
    /// between commas is skipped rather than refused.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidTrustedProxy`] when an entry is not an IP
    /// literal.
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        let mut addresses = Vec::new();
        for entry in value.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let address =
                entry
                    .parse::<IpAddr>()
                    .map_err(|_| ConfigError::InvalidTrustedProxy {
                        variable: TRUSTED_PROXIES_ENV,
                        value: value.to_owned(),
                    })?;
            addresses.push(address);
        }
        Ok(Self(addresses))
    }

    /// Whether `peer` is one of the named proxies.
    #[must_use]
    pub fn contains(&self, peer: IpAddr) -> bool {
        self.0.contains(&peer)
    }

    /// Whether no proxy is named, which is the default and the case the
    /// plaintext warning exists for.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for TrustedProxies {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for address in &self.0 {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{address}")?;
            first = false;
        }
        Ok(())
    }
}

/// The address a request is attributed to, and where that address came from.
///
/// The variant is the provenance, so a test, a warning, or an audit record can
/// tell the peer's own address from one a trusted proxy forwarded. That is what
/// makes "forwarded information is accepted only from a named proxy"
/// observable, rather than a property of a value that happens to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientAddress {
    /// The address of the peer that opened the connection.
    Peer(IpAddr),
    /// The left-most entry of a forwarded chain a trusted peer sent.
    Forwarded(IpAddr),
}

impl ClientAddress {
    /// The address itself, whatever its provenance.
    #[must_use]
    pub fn address(self) -> IpAddr {
        match self {
            Self::Peer(address) | Self::Forwarded(address) => address,
        }
    }
}

impl fmt::Display for ClientAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Peer(address) => write!(f, "peer {address}"),
            Self::Forwarded(address) => write!(f, "forwarded {address}"),
        }
    }
}

/// The address a request is attributed to.
///
/// A forwarded chain is read only from a peer the operator named, and only its
/// left-most entry: a multi-hop chain is not walked, because every hop past the
/// first is a claim this daemon cannot check. An entry that is not an IP
/// literal is not an address, so the answer falls back to the peer rather than
/// inventing one.
///
/// `None` when no peer is known, which is the local Unix socket: a local caller
/// has no address, and nothing is forwarded on its behalf.
#[must_use]
pub fn client_address(
    peer: Option<IpAddr>,
    trusted: &TrustedProxies,
    headers: &HeaderMap,
) -> Option<ClientAddress> {
    let peer = peer?;
    if !trusted.contains(peer) {
        return Some(ClientAddress::Peer(peer));
    }
    let forwarded = headers
        .get(FORWARDED_FOR)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .and_then(|left| left.parse::<IpAddr>().ok());
    Some(match forwarded {
        Some(address) => ClientAddress::Forwarded(address),
        None => ClientAddress::Peer(peer),
    })
}

/// How the leg between the caller and the proxy was served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientScheme {
    /// Nothing said the leg was encrypted, so it is treated as plaintext.
    Plaintext,
    /// A trusted proxy said it was reached over TLS.
    Encrypted,
}

/// The scheme the client leg was served over.
///
/// `Encrypted` only when a trusted peer sent `x-forwarded-proto: https`.
/// Everything else, an untrusted peer included, is [`ClientScheme::Plaintext`],
/// because this daemon terminates no TLS and must not claim an encryption it
/// cannot see.
#[must_use]
pub fn client_scheme(
    peer: Option<IpAddr>,
    trusted: &TrustedProxies,
    headers: &HeaderMap,
) -> ClientScheme {
    let encrypted = peer.is_some_and(|peer| {
        trusted.contains(peer)
            && headers
                .get(FORWARDED_PROTO)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.trim().eq_ignore_ascii_case(HTTPS))
    });
    if encrypted {
        ClientScheme::Encrypted
    } else {
        ClientScheme::Plaintext
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ClientAddress, ClientScheme, TRUSTED_PROXIES_ENV, TrustedProxies, client_address,
        client_scheme,
    };
    use crate::transport::ConfigError;
    use http::{HeaderMap, HeaderValue};
    use std::ffi::OsString;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// Serialises the environment in this test binary: `std::env` is process
    /// wide and the harness runs the tests on threads.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Holds one value of `METIS_TRUSTED_PROXIES` for the duration of a test and
    /// restores the previous value on drop, so a panic cannot leak the value
    /// into another test. The lock it carries serialises every environment test
    /// in this binary that reads the trusted-proxy set.
    struct TrustedProxiesEnv {
        previous: Option<OsString>,
        _lock: MutexGuard<'static, ()>,
    }

    impl TrustedProxiesEnv {
        fn set(value: &str) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            let previous = std::env::var_os(TRUSTED_PROXIES_ENV);
            // Edition 2024 makes mutating the process environment unsafe
            // because it is not thread-safe. Every test that touches the
            // variable holds `ENV_LOCK`, so nothing races this write.
            unsafe { std::env::set_var(TRUSTED_PROXIES_ENV, value) };
            Self {
                previous,
                _lock: lock,
            }
        }

        fn unset() -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            let previous = std::env::var_os(TRUSTED_PROXIES_ENV);
            // SAFETY: see `set`.
            unsafe { std::env::remove_var(TRUSTED_PROXIES_ENV) };
            Self {
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for TrustedProxiesEnv {
        fn drop(&mut self) {
            // SAFETY: see `set`; the lock is held until this guard is dropped.
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var(TRUSTED_PROXIES_ENV, value),
                    None => std::env::remove_var(TRUSTED_PROXIES_ENV),
                }
            }
        }
    }

    /// An absent variable and an empty one both name no proxy, so a fresh
    /// install trusts nobody and forwarded headers change nothing.
    #[test]
    fn an_absent_or_empty_value_names_no_proxy() {
        let absent = TrustedProxiesEnv::unset();
        let trusted = TrustedProxies::resolve().expect("an absent variable is not an error");
        assert!(trusted.is_empty());
        drop(absent);

        let empty = TrustedProxiesEnv::set("");
        let trusted = TrustedProxies::resolve().expect("an empty value is treated as unset");
        assert!(trusted.is_empty());
        drop(empty);

        assert!(
            TrustedProxies::parse("")
                .expect("an empty value is treated as unset")
                .is_empty()
        );
        assert!(
            TrustedProxies::parse("   ")
                .expect("whitespace names nothing either")
                .is_empty()
        );
    }

    /// The set is exact addresses, comma separated, with whitespace tolerated
    /// around entries, and membership is membership of that list alone: loopback
    /// is not trusted implicitly.
    #[test]
    fn a_named_list_is_the_trusted_set_and_nothing_else_is() {
        let trusted = TrustedProxies::parse(" 127.0.0.1 ,203.0.113.7, 2001:db8::1 ")
            .expect("the list parses");
        assert!(trusted.contains(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(trusted.contains(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))));
        assert!(trusted.contains(IpAddr::V6("2001:db8::1".parse::<Ipv6Addr>().unwrap())));
        assert!(
            !trusted.contains(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 8))),
            "an address that was not named is not trusted"
        );

        let no_loopback = TrustedProxies::parse("203.0.113.7").expect("the list parses");
        assert!(
            !no_loopback.contains(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            "loopback is not trusted implicitly"
        );
    }

    /// Anything that is not an IP literal is a typed error naming the variable
    /// and the value, never a silent empty set that would look like a
    /// deliberately closed configuration.
    #[test]
    fn a_malformed_entry_is_a_typed_error_naming_variable_and_value() {
        for value in [
            "127.0.0.1:8443",
            "proxy.example.com",
            "10.0.0.256",
            "127.0.0.1, nope",
        ] {
            let error = TrustedProxies::parse(value).expect_err("a non-literal entry is refused");
            match &error {
                ConfigError::InvalidTrustedProxy {
                    variable,
                    value: named,
                } => {
                    assert_eq!(*variable, TRUSTED_PROXIES_ENV);
                    assert_eq!(named, value);
                }
                other => panic!("a malformed entry is refused as InvalidTrustedProxy: {other}"),
            }
            let message = error.to_string();
            assert!(
                message.contains(TRUSTED_PROXIES_ENV),
                "the error names the variable: {message}"
            );
            assert!(
                message.contains(value),
                "the error names the value: {message}"
            );
        }
    }

    /// The configured value is read by the same rule as a parsed one, so a
    /// misspelled entry is loud rather than a daemon that trusts nobody.
    #[test]
    fn a_malformed_configured_value_is_refused_by_resolve() {
        let env = TrustedProxiesEnv::set("not-an-address");
        let error = TrustedProxies::resolve().expect_err("a configured non-literal is refused too");
        assert!(matches!(error, ConfigError::InvalidTrustedProxy { .. }));
        drop(env);

        let env = TrustedProxiesEnv::set("127.0.0.1,203.0.113.7");
        let trusted = TrustedProxies::resolve().expect("a configured list resolves");
        assert!(trusted.contains(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        drop(env);
    }

    /// The address a proxy named is accepted, and the entry that counts is the
    /// left-most one: the chain is not walked, because every hop past the first
    /// is a claim this daemon cannot check.
    #[test]
    fn forwarded_information_is_accepted_from_a_named_proxy() {
        let proxy = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));
        let trusted = TrustedProxies::parse("203.0.113.7").expect("the list parses");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.9, 203.0.113.7"),
        );

        let client =
            client_address(Some(proxy), &trusted, &headers).expect("a connection has a peer");
        assert_eq!(
            client,
            ClientAddress::Forwarded(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9))),
            "the left-most entry is the client"
        );
        assert_eq!(client.address(), IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9)));
        assert_eq!(
            client.to_string(),
            "forwarded 198.51.100.9",
            "the answer says where it came from"
        );
        assert_eq!(
            client_address(Some(proxy), &trusted, &HeaderMap::new()),
            Some(ClientAddress::Peer(proxy))
        );
    }

    /// The same header from a peer nobody named changes nothing: the answer is
    /// the address the connection came from, so a hostile client cannot choose
    /// the address it is recorded under.
    #[test]
    fn forwarded_information_is_ignored_from_an_unnamed_peer() {
        let peer = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));
        let trusted = TrustedProxies::parse("").expect("nothing is named");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.9, 203.0.113.7"),
        );
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));

        let client =
            client_address(Some(peer), &trusted, &headers).expect("a connection has a peer");
        assert_eq!(
            client,
            ClientAddress::Peer(peer),
            "an unnamed peer's forwarded header is not read"
        );
        assert_eq!(client.to_string(), "peer 203.0.113.7");
        assert_eq!(
            client_scheme(Some(peer), &trusted, &headers),
            ClientScheme::Plaintext,
            "the daemon does not claim an encryption it cannot see"
        );
    }

    /// A named proxy whose forwarded entry is not an address falls back to the
    /// peer rather than inventing a client, and a missing peer is no address at
    /// all.
    #[test]
    fn the_rule_never_invents_an_address() {
        let proxy = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));
        let trusted = TrustedProxies::parse("203.0.113.7").expect("the list parses");
        let garbage = {
            let mut headers = HeaderMap::new();
            headers.insert("x-forwarded-for", HeaderValue::from_static("unknown"));
            headers
        };
        assert_eq!(
            client_address(Some(proxy), &trusted, &garbage),
            Some(ClientAddress::Peer(proxy)),
            "an entry that is not an address is not a client address"
        );

        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.9"));
        assert_eq!(
            client_address(None, &trusted, &headers),
            None,
            "the local socket has no peer and nothing is forwarded for it"
        );
        assert_eq!(
            client_scheme(None, &trusted, &headers),
            ClientScheme::Plaintext
        );
    }

    /// The leg is encrypted only when a named proxy says the proxy itself was
    /// reached over TLS; anything else is plaintext.
    #[test]
    fn the_scheme_is_encrypted_only_when_a_named_proxy_says_so() {
        let proxy = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));
        let named = TrustedProxies::parse("203.0.113.7").expect("the list parses");
        let unnamed = TrustedProxies::parse("").expect("nothing is named");
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));

        assert_eq!(
            client_scheme(Some(proxy), &named, &headers),
            ClientScheme::Encrypted
        );
        assert_eq!(
            client_scheme(Some(proxy), &unnamed, &headers),
            ClientScheme::Plaintext
        );
        assert_eq!(
            client_scheme(None, &named, &headers),
            ClientScheme::Plaintext
        );

        headers.insert("x-forwarded-proto", HeaderValue::from_static("http"));
        assert_eq!(
            client_scheme(Some(proxy), &named, &headers),
            ClientScheme::Plaintext,
            "only `https` means encrypted"
        );
    }
}
