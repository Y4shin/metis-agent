//! The credential check: pairing a surface, and authorizing every other call.
//!
//! Two calls are exempt from the check, and the exemption is deliberate: `Pair`
//! is how a surface gets a credential in the first place. Which listener
//! requires a credential at all is the [`AuthPolicy`]: the local socket is
//! already authenticated by the operating system's access control, and the
//! remote listener is not, so each listener mounts the policy that matches how
//! it was reached.
//!
//! The check is an interceptor rather than something a handler remembers to
//! call, so it runs before the handler for unary and streaming calls alike and
//! a new RPC is covered the moment it is declared.
//!
//! The unary half is proven over a real socket in `crates/daemon/tests/auth.rs`,
//! where a recording service shows a refused call never reached its handler. The
//! streaming half has no wire test yet because the contract declares no
//! streaming RPC: `intercept_streaming` calls the same `admit` the unary half
//! does, and `event-stream` exercises it when it appends `StreamEvents`.

use std::fmt;

use api_contracts::auth::{self, CredentialRefusal, PairRefusal};
use api_contracts::connect::metis::v1::DAEMON_SERVICE_PAIR_SPEC;
use api_contracts::identity::{self, ClientIdentity};
use connectrpc::interceptor::{
    Next, NextStream, StreamRequest, StreamResponse, UnaryRequest, UnaryResponse,
};
use connectrpc::{ConnectError, ErrorCode, Interceptor, PayloadStream, RequestContext};
use http::{HeaderMap, HeaderValue};
use subtle::ConstantTimeEq as _;

use crate::secret::{PRESHARED_KEY_ENV, Secret};
use crate::state::{StateError, Store, Verify};
use crate::token::{Token, mint};

/// The response header a credential refusal carries. The spelling lives in
/// `api_contracts::auth`, because the client reads the same header; it is
/// re-exported here so the daemon's own refusals and the client's mapping
/// cannot name different headers.
pub use api_contracts::auth::AUTH_ERROR_HEADER;

/// The request header the credential travels in, as `Bearer <token>`.
pub use api_contracts::auth::AUTHORIZATION_HEADER;

/// The response header a pairing refusal carries. The spelling lives in
/// `api_contracts::auth` for the same reason [`AUTH_ERROR_HEADER`] does: the
/// client reads the header the daemon writes.
pub use api_contracts::auth::PAIR_ERROR_HEADER;

/// How a call must be authenticated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPolicy {
    /// The transport already authenticated the caller: the local socket's
    /// directory and file carry the operating system's access control, and a
    /// local client therefore needs no credential.
    LocalAcl,
    /// The caller must present the surface's token. The remote listener uses
    /// this, because nothing about a TCP connection says who is calling.
    TokenRequired,
}

/// The daemon refused a call because of its credential.
///
/// The two variants are the distinction a caller acts on: no credential means
/// pair, and a refused credential means the access is gone.
///
/// The enum is non-exhaustive: a later credential form adds its own refusals
/// without breaking a caller that matches on it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthError {
    /// The call carried no credential.
    MissingCredential,
    /// The call carried a credential the daemon will not accept.
    RejectedCredential {
        /// Why it was refused.
        cause: RejectionCause,
    },
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCredential => {
                write!(f, "this call needs a credential; pair the surface first")
            }
            Self::RejectedCredential { cause } => write!(f, "the credential was refused: {cause}"),
        }
    }
}

impl std::error::Error for AuthError {}

/// Why a presented credential was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RejectionCause {
    /// The surface has a row, and the credential is not the one it holds.
    Invalid,
    /// The surface has no row: it was never paired, or its token was revoked.
    Revoked,
}

impl fmt::Display for RejectionCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid => write!(f, "it is not this surface's token"),
            Self::Revoked => {
                write!(f, "this surface has no token: pair it, or it was revoked")
            }
        }
    }
}

/// The refusal a client reads as its own typed error.
///
/// The meaning crosses the wire as [`AUTH_ERROR_HEADER`] rather than as message
/// text, exactly as the identity channel's version refusal crosses as
/// `metis-supported-versions`.
#[must_use]
pub fn refusal_headers(error: &AuthError) -> HeaderMap {
    let refusal = match error {
        AuthError::MissingCredential => CredentialRefusal::Missing,
        AuthError::RejectedCredential { cause } => match cause {
            RejectionCause::Invalid => CredentialRefusal::Invalid,
            RejectionCause::Revoked => CredentialRefusal::Revoked,
        },
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTH_ERROR_HEADER,
        HeaderValue::from_static(refusal.as_str()),
    );
    headers
}

/// The refusal a client reads as its own typed pairing error.
///
/// The meaning crosses the wire as [`PAIR_ERROR_HEADER`] rather than as message
/// text, exactly as [`refusal_headers`] does for the credential. `None` means
/// the refusal has no wire value: a store fault is an internal failure, and a
/// client keeps the transport error rather than being told to act on a pairing
/// state that did not change.
#[must_use]
pub fn pair_refusal_headers(error: &PairError) -> Option<HeaderMap> {
    let refusal = match error {
        PairError::PairingDisabled => PairRefusal::Disabled,
        PairError::WrongPresharedKey => PairRefusal::WrongKey,
        PairError::AlreadyPaired => PairRefusal::AlreadyPaired,
        PairError::Store(_) => return None,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        PAIR_ERROR_HEADER,
        HeaderValue::from_static(refusal.as_str()),
    );
    Some(headers)
}

/// What an authorized call is granted.
///
/// A token grants the whole exposed surface: there are no scopes and no
/// per-client permissions, so the grant names the surface and nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Granted {
    identity: ClientIdentity,
}

impl Granted {
    /// The declaration the authorized call carried.
    #[must_use]
    pub fn identity(&self) -> &ClientIdentity {
        &self.identity
    }

    /// The surface the credential belongs to.
    #[must_use]
    pub fn surface(&self) -> &str {
        self.identity.surface()
    }
}

/// Authorize a call against the store.
///
/// The surface comes from the identity declaration and the credential from
/// `Authorization: Bearer`; the presented token is parsed and hashed exactly the
/// way the store hashes what it keeps, and the store's answer becomes the typed
/// refusal. A declaration the daemon cannot read is a missing credential: the
/// daemon cannot tell which surface is asking, so the caller has to declare
/// itself and pair.
///
/// # Errors
///
/// Returns [`AuthError::MissingCredential`] when the call carries neither a
/// readable declaration nor a bearer credential, and
/// [`AuthError::RejectedCredential`] when it carries one the store refuses.
pub async fn authorize(store: &Store, headers: &HeaderMap) -> Result<Granted, AuthError> {
    let Ok(identity) = identity::from_headers(headers) else {
        return Err(AuthError::MissingCredential);
    };
    let presented = headers
        .get(AUTHORIZATION_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(auth::bearer_token);
    // A header that is present but not a bearer credential is not a missing
    // credential: the caller presented something, and it is wrong.
    let Some(presented) = presented else {
        return Err(if headers.contains_key(AUTHORIZATION_HEADER) {
            AuthError::RejectedCredential {
                cause: RejectionCause::Invalid,
            }
        } else {
            AuthError::MissingCredential
        });
    };
    let Ok(token) = Token::parse(presented) else {
        return Err(AuthError::RejectedCredential {
            cause: RejectionCause::Invalid,
        });
    };
    let verified = store
        .verify_token(identity.surface(), &token.hash())
        .await
        .map_err(|_| AuthError::RejectedCredential {
            cause: RejectionCause::Invalid,
        })?;
    match verified {
        Verify::Verified => Ok(Granted { identity }),
        Verify::Mismatch => Err(AuthError::RejectedCredential {
            cause: RejectionCause::Invalid,
        }),
        Verify::UnknownSurface => Err(AuthError::RejectedCredential {
            cause: RejectionCause::Revoked,
        }),
    }
}

/// The interceptor every listener mounts.
///
/// It exempts the pairing procedure, leaves [`AuthPolicy::LocalAcl`] a
/// pass-through, and under [`AuthPolicy::TokenRequired`] authorizes the call in
/// both shapes, so a handler cannot forget the check and a streaming RPC is
/// covered at establishment.
pub struct AuthInterceptor {
    store: Store,
    policy: AuthPolicy,
}

impl AuthInterceptor {
    /// An interceptor over `store`, applying `policy`.
    #[must_use]
    pub fn new(store: Store, policy: AuthPolicy) -> Self {
        Self { store, policy }
    }

    /// Whether this call must present a credential.
    ///
    /// The pairing procedure is the one exemption: a surface that has no
    /// credential yet is exactly the caller that needs to reach it. The
    /// exemption is by procedure path, so a later pre-auth RPC has to be added
    /// here deliberately.
    fn needs_credential(&self, path: Option<&str>) -> bool {
        self.policy == AuthPolicy::TokenRequired && path != Some(DAEMON_SERVICE_PAIR_SPEC.procedure)
    }

    /// Admit or refuse one call.
    async fn admit(&self, ctx: &RequestContext) -> Result<(), ConnectError> {
        if !self.needs_credential(ctx.path()) {
            return Ok(());
        }
        // The declaration is the identity channel's business, so a declaration
        // the daemon cannot read is refused the way a handler refuses it: with
        // the header that names the versions this daemon speaks.
        if let Err(error) = identity::from_headers(ctx.headers()) {
            return Err(
                ConnectError::new(ErrorCode::InvalidArgument, error.to_string())
                    .with_headers(identity::refusal_headers(&error)),
            );
        }
        authorize(&self.store, ctx.headers())
            .await
            .map_err(|error| {
                ConnectError::new(ErrorCode::Unauthenticated, error.to_string())
                    .with_headers(refusal_headers(&error))
            })?;
        Ok(())
    }
}

#[connectrpc::async_trait]
impl Interceptor for AuthInterceptor {
    async fn intercept_unary(
        &self,
        req: UnaryRequest,
        next: Next<'_>,
    ) -> Result<UnaryResponse, ConnectError> {
        self.admit(&req.ctx).await?;
        next.run(req).await
    }

    async fn intercept_streaming(
        &self,
        req: StreamRequest,
        inbound: PayloadStream,
        next: NextStream<'_>,
    ) -> Result<StreamResponse, ConnectError> {
        self.admit(&req.ctx).await?;
        next.run(req, inbound).await
    }
}

/// The daemon refused to pair a surface.
///
/// The enum is non-exhaustive: a later pairing step adds its own refusals
/// without breaking a caller that matches on it.
#[derive(Debug)]
#[non_exhaustive]
pub enum PairError {
    /// The daemon has no preshared key configured, so pairing is closed. It is
    /// not a misconfiguration: pairing is optional, and a daemon without a key
    /// still serves every paired surface.
    PairingDisabled,
    /// The preshared key in the request body is not the configured one.
    WrongPresharedKey,
    /// The surface already has a token. Its first token stays the surface's
    /// until it is revoked.
    AlreadyPaired,
    /// The store refused the write.
    Store(StateError),
}

impl fmt::Display for PairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PairingDisabled => write!(
                f,
                "pairing is closed: {PRESHARED_KEY_ENV} names no key to pair with"
            ),
            Self::WrongPresharedKey => write!(f, "the preshared key does not match"),
            Self::AlreadyPaired => write!(
                f,
                "the surface already has a token; revoke it before pairing again"
            ),
            Self::Store(source) => write!(f, "the token could not be stored: {source}"),
        }
    }
}

impl std::error::Error for PairError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(source) => Some(source),
            Self::PairingDisabled | Self::WrongPresharedKey | Self::AlreadyPaired => None,
        }
    }
}

/// Pair `surface`: check the preshared key, mint a token, keep only its hash,
/// and answer with the token text.
///
/// The key is compared in constant time and before anything is written, so a
/// wrong key neither pairs nor learns whether the surface is already paired.
///
/// # Errors
///
/// Returns [`PairError::WrongPresharedKey`] when the body key is not the
/// configured one, [`PairError::AlreadyPaired`] when the surface already has a
/// row, and [`PairError::Store`] when the row cannot be written.
pub async fn pair(
    store: &Store,
    preshared: &Secret,
    surface: &str,
    presented: &str,
) -> Result<Token, PairError> {
    let expected = preshared.expose().as_bytes();
    let presented_bytes = presented.as_bytes();
    let matches: bool = expected.ct_eq(presented_bytes).into();
    if !matches {
        return Err(PairError::WrongPresharedKey);
    }
    let (token, hash) = mint();
    match store.insert_token(surface, &hash).await {
        Ok(()) => Ok(token),
        Err(StateError::SurfaceAlreadyPaired) => Err(PairError::AlreadyPaired),
        Err(source) => Err(PairError::Store(source)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::SecretRef;
    use api_contracts::auth;
    use api_contracts::identity::{
        PROTOCOL_CLIENT_VERSION_HEADER, PROTOCOL_SURFACE_HEADER, PROTOCOL_VERSION_HEADER,
    };
    use http::HeaderValue;

    /// A secret the tests pair with, built the way the daemon builds it.
    fn secret(text: &str) -> Secret {
        SecretRef::parse(&format!("literal:{text}"))
            .resolve()
            .expect("a literal resolves")
    }

    /// The headers of a client that declares `surface`.
    fn declared(surface: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            PROTOCOL_VERSION_HEADER,
            HeaderValue::from_str(&identity::PROTOCOL_VERSION.to_string()).expect("a version"),
        );
        headers.insert(
            PROTOCOL_SURFACE_HEADER,
            HeaderValue::from_str(surface).expect("a surface name"),
        );
        headers.insert(
            PROTOCOL_CLIENT_VERSION_HEADER,
            HeaderValue::from_static("0.0.0-test"),
        );
        headers
    }

    /// The digest contract: the token `pair` returns is the one whose hash the
    /// row holds, under the landed rule that hashes the token's text.
    #[tokio::test]
    async fn the_paired_token_verifies_against_the_row_the_store_wrote() {
        let store = Store::open_in_memory().await.expect("the store opens");
        let token = pair(&store, &secret("preshared"), "cli", "preshared")
            .await
            .expect("the key matches");

        assert_eq!(token.as_str().len(), 64, "32 bytes of hex");
        assert_eq!(
            store.surfaces().await.expect("the rows read"),
            vec!["cli".to_owned()]
        );

        // The client's own path: parse the text it was handed and hash it.
        let presented = Token::parse(token.as_str()).expect("the token parses");
        assert_eq!(
            store
                .verify_token("cli", &presented.hash())
                .await
                .expect("the row reads"),
            Verify::Verified
        );
        // A different token for the same surface is a mismatch rather than a
        // success, so the row is really this token's.
        assert_eq!(
            store
                .verify_token(
                    "cli",
                    &Token::parse(&"0".repeat(64)).expect("parses").hash()
                )
                .await
                .expect("the row reads"),
            Verify::Mismatch
        );
    }

    /// A wrong key pairs nothing, and a second pairing leaves the first token
    /// working.
    #[tokio::test]
    async fn pairing_refuses_a_wrong_key_and_a_second_pairing() {
        let store = Store::open_in_memory().await.expect("the store opens");
        let error = pair(&store, &secret("preshared"), "cli", "wrong")
            .await
            .expect_err("the key does not match");
        assert!(matches!(error, PairError::WrongPresharedKey), "got {error}");
        assert!(
            store.surfaces().await.expect("the rows read").is_empty(),
            "nothing was written"
        );

        let first = pair(&store, &secret("preshared"), "cli", "preshared")
            .await
            .expect("the key matches");
        let error = pair(&store, &secret("preshared"), "cli", "preshared")
            .await
            .expect_err("the surface is already paired");
        assert!(matches!(error, PairError::AlreadyPaired), "got {error}");
        assert_eq!(
            store
                .verify_token("cli", &first.hash())
                .await
                .expect("the row reads"),
            Verify::Verified,
            "the first token is the one that stays"
        );
    }

    /// An empty configured key is not a secret at all, so nothing can pair with
    /// an empty one.
    #[tokio::test]
    async fn an_empty_preshared_key_is_never_configured() {
        let store = Store::open_in_memory().await.expect("the store opens");
        let error = SecretRef::parse("literal:")
            .resolve()
            .expect_err("an empty secret is not a secret");
        assert!(matches!(error, crate::secret::SecretError::Unresolved));
        assert!(
            store.surfaces().await.expect("the rows read").is_empty(),
            "and no pairing can happen without one"
        );
    }

    /// The refusal headers carry the three causes, and the client's vocabulary
    /// reads them back.
    #[test]
    fn refusals_cross_the_wire_as_the_auth_header() {
        let cases = [
            (AuthError::MissingCredential, CredentialRefusal::Missing),
            (
                AuthError::RejectedCredential {
                    cause: RejectionCause::Invalid,
                },
                CredentialRefusal::Invalid,
            ),
            (
                AuthError::RejectedCredential {
                    cause: RejectionCause::Revoked,
                },
                CredentialRefusal::Revoked,
            ),
        ];
        for (error, expected) in cases {
            let headers = refusal_headers(&error);
            let value = headers
                .get(AUTH_ERROR_HEADER)
                .expect("the refusal carries the header");
            assert_eq!(
                CredentialRefusal::parse(value.to_str().expect("a text value")),
                Some(expected),
                "the header says {expected} for {error}"
            );
        }
    }

    /// A missing credential and a wrong one are different answers from
    /// `authorize`.
    #[tokio::test]
    async fn authorize_distinguishes_missing_from_rejected() {
        let store = Store::open_in_memory().await.expect("the store opens");
        let token = pair(&store, &secret("preshared"), "cli", "preshared")
            .await
            .expect("the key matches");

        let error = authorize(&store, &declared("cli"))
            .await
            .expect_err("no credential was presented");
        assert!(matches!(error, AuthError::MissingCredential), "got {error}");

        let mut headers = declared("cli");
        headers.insert(AUTHORIZATION_HEADER, HeaderValue::from_static("basic abc"));
        let error = authorize(&store, &headers)
            .await
            .expect_err("that is not a bearer credential");
        assert!(
            matches!(
                error,
                AuthError::RejectedCredential {
                    cause: RejectionCause::Invalid
                }
            ),
            "got {error}"
        );

        let mut headers = declared("ui");
        headers.insert(
            AUTHORIZATION_HEADER,
            HeaderValue::from_str(&auth::bearer(token.as_str())).expect("a bearer value"),
        );
        let error = authorize(&store, &headers)
            .await
            .expect_err("that surface has no row");
        assert!(
            matches!(
                error,
                AuthError::RejectedCredential {
                    cause: RejectionCause::Revoked
                }
            ),
            "got {error}"
        );

        let mut headers = declared("cli");
        headers.insert(
            AUTHORIZATION_HEADER,
            HeaderValue::from_str(&auth::bearer(&"0".repeat(64))).expect("a bearer value"),
        );
        let error = authorize(&store, &headers)
            .await
            .expect_err("that is not this surface's token");
        assert!(
            matches!(
                error,
                AuthError::RejectedCredential {
                    cause: RejectionCause::Invalid
                }
            ),
            "got {error}"
        );

        let mut headers = declared("cli");
        headers.insert(
            AUTHORIZATION_HEADER,
            HeaderValue::from_str(&auth::bearer(token.as_str())).expect("a bearer value"),
        );
        let granted = authorize(&store, &headers)
            .await
            .expect("the token is this surface's");
        assert_eq!(granted.surface(), "cli");
    }

    /// The policy decides, and the pairing procedure is the one exemption.
    #[tokio::test]
    async fn the_pairing_procedure_is_the_only_exempt_path() {
        let store = Store::open_in_memory().await.expect("the store opens");
        let local = AuthInterceptor::new(store.clone(), AuthPolicy::LocalAcl);
        assert!(!local.needs_credential(Some("/metis.v1.DaemonService/Echo")));

        let required = AuthInterceptor::new(store, AuthPolicy::TokenRequired);
        assert!(required.needs_credential(Some("/metis.v1.DaemonService/Echo")));
        assert!(!required.needs_credential(Some(DAEMON_SERVICE_PAIR_SPEC.procedure)));
        assert!(
            required.needs_credential(None),
            "a call whose path is unknown is not exempt"
        );
    }
}
