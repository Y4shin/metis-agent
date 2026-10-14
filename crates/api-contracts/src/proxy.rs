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

use crate::transport::ConfigError;

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

#[cfg(test)]
mod tests {
    use super::{TRUSTED_PROXIES_ENV, TrustedProxies};
    use crate::transport::ConfigError;
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
}
