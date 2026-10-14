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
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};

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

/// What the daemon tells the operator about plaintext it can see.
///
/// Both variants exist because both are reachable without anyone making a
/// mistake in code: a listener can be bound to a public address, and a pairing
/// call can arrive over a leg that nothing encrypted. Each `Display` names what
/// is exposed and the remedy, because a warning that only says "plaintext"
/// leaves the operator to guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warning {
    /// A listener serves plaintext on an address reachable from outside this
    /// host, and no trusted proxy is named.
    PlaintextListener {
        /// The address the listener is bound to.
        addr: SocketAddr,
    },
    /// The pairing call, the one call that carries a secret in a body, arrived
    /// over a plaintext leg.
    PlaintextSecret {
        /// The client the plaintext leg came from, or `None` when the leg has no
        /// visible peer, which is the local socket.
        client: Option<IpAddr>,
    },
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlaintextListener { addr } => write!(
                f,
                "the listener at {addr} serves plaintext and no proxy is named in {TRUSTED_PROXIES_ENV}; \
                 terminate TLS in a reverse proxy and name it there"
            ),
            Self::PlaintextSecret { client } => {
                let leg = match client {
                    Some(client) => client.to_string(),
                    None => "no visible peer, so the local socket".to_owned(),
                };
                write!(
                    f,
                    "the pairing call arrived in plaintext from {leg} and carried the preshared key \
                     in its body; terminate TLS in a reverse proxy and name it in {TRUSTED_PROXIES_ENV}"
                )
            }
        }
    }
}

/// What to tell the operator about the listener that is about to accept.
///
/// `Some` exactly when the bind address is reachable from outside this host and
/// no trusted proxy is named. A loopback listener is not exposed, and a named
/// proxy says the operator put something encrypted in front of this one, so
/// neither is worth a warning.
#[must_use]
pub fn listener_warning(addr: SocketAddr, trusted: &TrustedProxies) -> Option<Warning> {
    if addr.ip().is_loopback() || !trusted.is_empty() {
        return None;
    }
    Some(Warning::PlaintextListener { addr })
}

/// What to tell the operator about the leg a secret-bearing call arrived on.
///
/// `Some` on a plaintext leg, whatever it is worth: the warning is the only
/// signal this daemon has that the secret may have been readable on the way.
#[must_use]
pub fn secret_warning(scheme: ClientScheme, client: Option<IpAddr>) -> Option<Warning> {
    match scheme {
        ClientScheme::Encrypted => None,
        ClientScheme::Plaintext => Some(Warning::PlaintextSecret { client }),
    }
}

/// A latch so a per-request warning reaches the operator once per process.
///
/// A warning that repeats on every call is noise an operator learns to skip;
/// the first occurrence is the one that says the configuration is wrong.
#[derive(Debug, Default)]
pub struct WarnOnce {
    emitted: AtomicBool,
}

impl WarnOnce {
    /// A latch that has emitted nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            emitted: AtomicBool::new(false),
        }
    }

    /// Emit `warning` unless this latch already has.
    ///
    /// Returns whether this call was the one that emitted, so the rule is
    /// observable without reading stderr.
    pub fn warn(&self, warning: &Warning) -> bool {
        if self.emitted.swap(true, Ordering::Relaxed) {
            return false;
        }
        eprintln!("metis: {warning}");
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ClientAddress, ClientScheme, TRUSTED_PROXIES_ENV, TrustedProxies, WarnOnce, Warning,
        client_address, client_scheme, listener_warning, secret_warning,
    };
    use crate::transport::ConfigError;
    use http::{HeaderMap, HeaderValue};
    use std::ffi::OsString;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
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

    /// A listener that is reachable from outside this host and has no proxy
    /// named is the exposure the operator must be told about. A named proxy
    /// suppresses it, because naming one says something encrypted sits in
    /// front; loopback is not exposed and needs no proxy at all.
    #[test]
    fn the_listener_warning_follows_the_bind_address_and_the_named_proxies() {
        let exposed = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8443);
        let loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8443);
        let loopback_v6 = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 8443);
        let none = TrustedProxies::parse("").expect("nothing is named");
        let named = TrustedProxies::parse("203.0.113.7").expect("the list parses");

        let warning = listener_warning(exposed, &none)
            .expect("a plaintext listener with no proxy named is warned about");
        assert_eq!(warning, Warning::PlaintextListener { addr: exposed });
        let message = warning.to_string();
        assert!(
            message.contains(&exposed.to_string()),
            "the warning says what is exposed: {message}"
        );
        assert!(
            message.contains(TRUSTED_PROXIES_ENV),
            "the warning names the remedy: {message}"
        );

        assert_eq!(
            listener_warning(exposed, &named),
            None,
            "naming a trusted proxy suppresses the warning"
        );
        assert_eq!(
            listener_warning(loopback, &none),
            None,
            "a loopback listener is not exposed"
        );
        assert_eq!(listener_warning(loopback_v6, &none), None);
    }

    /// The pairing call, the one call carrying a secret in a body, is warned
    /// about on a plaintext leg and not on an encrypted one.
    #[test]
    fn the_secret_warning_follows_the_client_leg() {
        let client = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9));

        let warning = secret_warning(ClientScheme::Plaintext, Some(client))
            .expect("a plaintext client leg is warned about");
        assert_eq!(
            warning,
            Warning::PlaintextSecret {
                client: Some(client)
            }
        );
        let message = warning.to_string();
        assert!(
            message.contains(&client.to_string()),
            "the warning names the leg: {message}"
        );
        assert!(
            message.contains(TRUSTED_PROXIES_ENV),
            "the warning names the remedy: {message}"
        );

        assert_eq!(secret_warning(ClientScheme::Encrypted, Some(client)), None);
        let local = secret_warning(ClientScheme::Plaintext, None)
            .expect("a leg with no peer is still plaintext");
        assert_eq!(local, Warning::PlaintextSecret { client: None });
        assert!(local.to_string().contains(TRUSTED_PROXIES_ENV));
    }

    /// The latch emits once and then reports that it did not, so a warning that
    /// would repeat on every call reaches the operator one time.
    #[test]
    fn warn_once_emits_exactly_once() {
        let warn_once = WarnOnce::new();
        let listener = Warning::PlaintextListener {
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8443),
        };
        let secret = Warning::PlaintextSecret {
            client: Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9))),
        };

        assert!(warn_once.warn(&listener), "the first call emits");
        assert!(!warn_once.warn(&listener), "the second call does not");
        assert!(
            !warn_once.warn(&secret),
            "the latch is spent for every warning, not per warning text"
        );
    }

    /// The front-end reads the trusted-proxy set where it binds, so a value it
    /// cannot read refuses the bind rather than being ignored or silently read
    /// as "no proxy".
    #[tokio::test]
    async fn a_malformed_trusted_proxy_value_refuses_the_tcp_front_end() {
        use crate::transport::serve_tcp_listener_service;
        use connectrpc::{ConnectRpcService, Router};
        use std::sync::Arc;

        let env = TrustedProxiesEnv::set("not-an-address");
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind a loopback port");
        let service = ConnectRpcService::from_arc(Arc::new(Router::new()));
        let error = serve_tcp_listener_service(listener, service)
            .await
            .expect_err("a set the daemon cannot read refuses the bind");
        assert!(
            error.to_string().contains(TRUSTED_PROXIES_ENV),
            "the refusal names the variable: {error}"
        );
        drop(env);
    }
}
