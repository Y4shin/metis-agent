---
type: deviation report
title: Deviation report for pairing-and-tokens
status: stable
---

## Deviation report: pairing-and-tokens

Implemented on `ticket/pairing-and-tokens` (tip `7dc60b1`, nine checkpoints off
`main` at `4f7c999`, which is the arch-spec approval commit carrying the
`local-identity-and-acl` and `state-store` landings). `main` has since gained one
docs-only commit, `97e0a91`, the `serve-tcp-service` follow-up ticket, which is
not this ticket's work. Compared against the `## Pending ticket:
pairing-and-tokens` section of `docs/tasks/metis-foundations/arch-spec.md` and
the ticket doc's acceptance criteria.

Gates re-run here on the branch tip, independently of the tdd and verify steps:
`cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings` exit 0
(the only warning any target prints is the pre-existing `plugin-contract`
`_FORTIFY_SOURCE` warning from the `cc` build script); the workspace `cargo test`
at 140 passed, 0 failed, 7 ignored; `buf lint`, `buf breaking --against
'.git#branch=main' --against-config buf.yaml` and `bash
scripts/regenerate-contracts.sh --check` all exit 0.

### API surface changes

- **Planned:** `crates/daemon/src/service.rs` exposes `router(store, preshared,
  AuthPolicy) -> Router`, registering the service with `AuthInterceptor`, and
  "each listener mounts its own".
  **Actual:** connectrpc 0.9.1 keeps the interceptor chain on
  `ConnectRpcService` and offers no conversion back to a `Router`, so the
  factory returns the service: `service(store, preshared, AuthPolicy) ->
  ConnectRpcService<Router>`, built with
  `ConnectRpcService::new(router).with_interceptor(interceptor)`. A policy can
  therefore only be served through the service it was installed on, and the
  ticket adds `api_contracts::transport::serve_uds_service(path,
  ConnectRpcService<Router>)`, the Unix front-end that takes it. `serve_stream`
  and `serve_uds` keep their signatures and now delegate to a private
  `serve_connection`, which is the body `serve_stream` already had, so the ACL
  `serve_uds` applies still applies on the service path.
  **Impact:** a listener mounts a service, not a router, which is the opposite
  of what the spec's interface contract told dependents. The landed TCP
  front-ends (`serve_tcp`, `serve_tcp_listener`) take a bare `Router` and are
  untouched, so an authorized service cannot yet be mounted on TCP. That gap is
  now the `serve-tcp-service` ticket on `main`, which `remote-pairing` is
  blocked by, so no dependent can wire a remote listener without closing it.

- **Planned:** `Client::with_surface(self, Surface) -> Self` and the spec's
  statement that the landed `local`, `with_endpoint` and `with_remote_addr`
  signatures stay.
  **Actual:** `with_surface` returns `Result<Self, ClientError>`, because
  declaring a surface now loads that surface's stored credential, and
  `Client::pair` takes `&mut self` because attaching the minted token needs
  `config_mut()`. The constructors themselves are unchanged. `ClientError` gains
  `Credential(credential::CredentialError)`, `CredentialMissing`, and
  `CredentialRejected { cause: CredentialCause }`, and `client::credential`
  adds `Backend::{Keychain, File(PathBuf)}`, `Stored` and `CredentialError`.
  **Impact:** `with_surface` is a fallible call on a landed surface; the only
  in-tree caller, `crates/client/tests/identity.rs`, moved to `?` with no
  assertion change. Dependent tickets (`terminal`, the app) call it as a
  `Result`.

- **Planned:** the credential wire vocabulary lives in `crates/daemon/src/auth.rs`
  (`AUTH_ERROR_HEADER`, the refusal variants, `refusal_headers`).
  **Actual:** the spelling is owned by the new `crates/api-contracts/src/auth.rs`
  (`AUTHORIZATION_HEADER`, `AUTH_ERROR_HEADER`, the non-exhaustive
  `CredentialRefusal::{Missing, Invalid, Revoked}` with `as_str` and `parse`,
  and `bearer`/`bearer_token`), and `daemon::auth` re-exports both headers and
  adds `AuthError`, `RejectionCause`, `Granted`, `AuthPolicy`, `AuthInterceptor`,
  `authorize`, `pair` and `PairError`. The daemon refuses through the same
  header the client reads, and the bearer scheme is matched case-insensitively.
  **Impact:** a dependent that maps a credential refusal reads
  `api_contracts::auth`, not `daemon::auth`; the refusal meaning still crosses
  the wire as the header and never as message text.

- **Planned:** `pair(store, preshared, &str) -> Result<Token, PairError>`.
  **Actual:** four parameters, `pair(store, preshared, surface, presented)`,
  because the spec's form did not say how the target surface and the presented
  key are both supplied. The key is compared with `subtle` in constant time and
  before any write, so a wrong key pairs nothing and learns nothing.
  **Impact:** none for dependents; the handler in `service.rs` is the one caller,
  and both `WrongPresharedKey` and `AlreadyPaired` are typed.

- **Planned:** `AuthInterceptor` exempting the `Pair` procedure and calling
  `authorize` in `intercept_unary` and `intercept_streaming`.
  **Actual:** exactly that, with one private `needs_credential(path)` deciding
  the exemption by procedure path and one private `admit` shared by both
  interceptor methods, so the unary and streaming halves cannot diverge. A
  declaration the identity channel cannot read is refused inside `admit` with
  the identity module's own header, so a call with a bad declaration is
  `InvalidArgument` carrying `metis-supported-versions` rather than an
  authentication error.
  **Impact:** `event-stream` inherits its authorization, as the interface
  contract promises, and the residual `local-identity-and-acl` recorded for
  per-handler checks is closed for this service.

- **Planned:** the contract append, `Pair(PairRequest) -> PairResponse` with
  `preshared_key = 1` and `token = 1`.
  **Actual:** landed as specified, with the proto's file comment updated from
  "One ordinary RPC for now" to "Two ordinary RPCs for now". The generated
  buffa and connect trees are regenerated and committed, the drift check passes,
  and `buf breaking` reports nothing to complain about.
  **Impact:** `remote-pairing` and `event-stream` build on the same service and
  package, as the outline says.

- **Planned:** the spec's export list named `common::time` nowhere, and
  `client::skeleton` held the RFC 3339 stamp.
  **Actual:** `api_contracts::time::{rfc3339, rfc3339_now}` is a new public
  module, moved out of `client::skeleton` so the daemon's own service can stamp
  `EchoResponse.received_at` without depending on test support.
  **Impact:** additive; `client::skeleton` delegates to it, so the two services
  that answer the contract share one calendar implementation.

### Abstraction usage

- Used/was specified: yes, with one substitution the pinned framework forced
  (service in place of router, above).
  - `connectrpc::{Interceptor, Next, NextStream, UnaryRequest, StreamRequest}` and
    `ConnectRpcService::with_interceptor` carry the authorization policy instead
    of a hand-rolled tower layer, exactly as the spec directed.
  - `daemon::state::Store` and `daemon::token::{mint, Token, TokenHash}` are
    consumed, not reimplemented. The digest rule holds: the token is verified
    against the row through the landed hash, and `daemon/src/auth.rs` unit-tests
    that the returned text verifies against the stored row, so the cross-ticket
    digest contract from `state-store` is asserted here rather than assumed.
  - `identity::{from_headers, refusal_headers}` and the
    `metis-supported-versions` header are the shape the auth header copies; the
    surface comes from `PROTOCOL_SURFACE_HEADER`.
  - `transport::{ensure_private_dir, restrict_file}` are reused through
    `client::credential` for the fallback file's 0600 mode and its 0700
    directory, rather than a second mode-setting path.
  - The non-exhaustive error convention holds for `AuthError`, `PairError`,
    `KeychainError`, `CredentialError`, `Stored` and the extended `ClientError`.
  - The RAII environment and temporary-directory helper pattern from the landed
    tests is reused in `daemon/tests/{auth,secret}.rs` and
    `client/tests/credential.rs`.
  - The spec's "do not" list is respected: no credential in a URL, query string
    or log line (a test asserts a served call reaches the handler on the bare
    method path with no token and no `?`, and `Token` and `Stored` carry
    hand-written redacting `Debug` impls); no token or key in the database; no
    per-client permissions, since `Granted` exposes no scope; no TLS, pin or
    proxy rule; no keyring crate, only a `PATH` probe; and no second hashing
    rule.

### Out-of-scope changes

- New `crates/api-contracts/src/time.rs`: the RFC 3339 helpers moved out of
  `client::skeleton`, which shrank by 9 net lines. Both services that answer
  `received_at` need the same implementation, so the move is a de-duplication
  rather than new behaviour.
- New `crates/api-contracts/src/auth.rs`: the credential wire vocabulary, above.
  It is a placement divergence, not an extra feature.
- `crates/daemon/src/service.rs` implements the generated trait for the daemon's
  own service (echo and pair), and `client::skeleton` answers `pair` with
  `ErrorCode::Unimplemented` and the message "the skeleton does not pair; the
  daemon's own service does". The spec's residual list already expected the
  skeleton to remain a second implementation.
- `crates/daemon/Cargo.toml` gains `connectrpc` and `http`, plus the dev-deps
  `client` and `connectrpc`; `Cargo.lock` follows. `Store` gained `Clone`, which
  the interceptor needs to hold a store while the service holds another.
- `docs/dev-env.md` and `docs/testing.md` were extended beyond the single
  variable the spec named: the token file's path and mode, the keychain rule,
  and the run commands for the new suites.
- The proto file's top comment changed to say two RPCs.
- Nothing was removed. `serve_stream`, `serve_uds`, `serve_tcp`,
  `serve_tcp_listener`, `uds_transport` and `remote_authority` keep their
  signatures and behaviour, as do `Client::{local, with_endpoint,
  with_remote_addr, echo}`, the store API, the ABI layout and the plugin crates.

### Ticket doc update needed?

Yes. The ticket doc has no `## Implementation notes` section (it ends at
`## Blocked by`), so the land-worker must create the heading, then record:

- the landing paragraph (merge commit, landing branch, working branch, the nine
  checkpoints, and the fact that the retry's tdd run re-verified the tip and made
  no new commit);
- the criteria-to-test map: `daemon/tests/secret.rs` (12), `daemon/tests/auth.rs`
  (10), `daemon/tests/token.rs` (4), `client/tests/credential.rs` (5),
  `api-contracts/tests/keychain.rs` (5), plus the unit tests in
  `daemon/src/auth.rs` and `api-contracts/src/time.rs`;
- the divergences above, with the service-instead-of-router factory first,
  because it constrains every listener ticket;
- the residual risks: no wire test for the streaming half of the interceptor
  (`event-stream` exercises it); a store fault during verification fails closed
  as `CredentialRejected { Invalid }`; the secret file's access rule is Unix-only;
  keychain absence versus refusal is inferred from exit status and standard
  error; macOS compiles but is never run against the real `security` tool; a
  wrong preshared key reaches a client as `ClientError::Call` with
  `PermissionDenied` rather than a typed pairing variant; no mutation run beyond
  the one the tdd step made;
- the correction that the prior verify artifact miscounted `daemon/tests/token.rs`
  as 11 tests, where the suite has 4 and the workspace total of 140 is correct.

### User attention needed?

Yes, on settled facts and one open gap.

- The TCP enforcement gap is the item to know about. The spec's interface
  contract assumed a policy could travel through a `Router`, and on connectrpc
  0.9.1 it cannot, so the two landed TCP front-ends cannot mount the authorized
  service. No acceptance criterion is unmet, because this ticket's criteria are
  credential-lifecycle criteria proven over the local socket, but a TCP listener
  wired through today's surface would serve unauthenticated. The orchestrator
  already created `serve-tcp-service` on `main` (`97e0a91`) and reblocked
  `remote-pairing` onto it, so the gap is graph-visible; the spec section needs
  the reconciler's pass to record that a listener mounts a service.
- `Client::with_surface` is now fallible and `Client::pair` takes `&mut self`.
  Both are breaks to a landed surface with one in-tree caller, updated with no
  assertion change.
- Behaviour that was the operator's decision rather than the spec's: the
  preshared key travels in the `PairRequest` body, which the module docs and the
  proto comment both state as the one place a secret travels in a body, and
  authorization is enforced as connectrpc middleware rather than in handlers.

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "Deviation report written for pairing-and-tokens: repo path docs/tasks/metis-foundations/tickets/pairing-and-tokens/deviation-reports/pairing-and-tokens.md plus the runtime output path; gates re-verified on tip 7dc60b1 (fmt clean, clippy exit 0, 140 tests passed, buf lint/breaking/regen green)."
    }
  ],
  "changedFiles": [
    "docs/tasks/metis-foundations/tickets/pairing-and-tokens/deviation-reports/pairing-and-tokens.md"
  ],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "devenv shell -- cargo fmt --check",
      "result": "passed",
      "summary": "exit 0, no diff"
    },
    {
      "command": "devenv shell -- cargo clippy --all-targets -- -D warnings",
      "result": "passed",
      "summary": "CLIPPY_EXIT=0, only the pre-existing plugin-contract _FORTIFY_SOURCE warning"
    },
    {
      "command": "devenv shell -- cargo test",
      "result": "passed",
      "summary": "140 passed, 0 failed, 7 ignored across all suites"
    },
    {
      "command": "devenv shell -- buf lint",
      "result": "passed",
      "summary": "LINT-OK"
    },
    {
      "command": "devenv shell -- buf breaking --against '.git#branch=main' --against-config buf.yaml",
      "result": "passed",
      "summary": "BREAKING-OK"
    },
    {
      "command": "devenv shell -- bash scripts/regenerate-contracts.sh --check",
      "result": "passed",
      "summary": "drift check: generated trees match the contract"
    }
  ],
  "validationOutput": [
    "branch tip 7dc60b1 on ticket/pairing-and-tokens, nine checkpoints off main at 4f7c999; working tree holds only the orchestrator's docs/tasks/state.yaml pointer",
    "cargo test: passed 140 failed 0 ignored 7",
    "buf lint LINT-OK, buf breaking BREAKING-OK, regeneration drift check clean"
  ],
  "residualRisks": [
    "The landed TCP front-ends take a bare Router, so the authorized service cannot be mounted on TCP yet; the serve-tcp-service ticket on main (97e0a91) owns the fix and remote-pairing is blocked by it",
    "The streaming half of the interceptor has no wire test, because the contract declares no streaming RPC yet; event-stream exercises it",
    "A store fault during verification fails closed as CredentialRejected { Invalid }",
    "The secret file access rule is Unix-only, and keychain absence versus refusal is inferred from exit status and standard error",
    "Client::with_surface is now fallible and Client::pair takes &mut self, breaks to a landed client surface with one in-tree caller"
  ],
  "noStagedFiles": true,
  "diffSummary": "No repository changes other than this deviation report; the report records the branch 4f7c999..7dc60b1 analysis and the gates re-run on that tip.",
  "reviewFindings": [
    "blocker: crates/api-contracts/src/transport.rs and crates/daemon/src/service.rs - the policy lives on ConnectRpcService, so serve_tcp and serve_tcp_listener cannot mount the authorized service; owned by the serve-tcp-service ticket and out of this ticket's criteria",
    "note: crates/client/src/lib.rs:260 - Client::with_surface returns Result now, a fallible call on a landed surface",
    "note: crates/daemon/src/service.rs:83 - service() returns ConnectRpcService<Router> instead of the spec's Router factory",
    "note: crates/api-contracts/src/auth.rs - the credential wire vocabulary landed in api-contracts rather than daemon-only, so both ends spell the channel once",
    "note: crates/client/src/skeleton.rs:113 - the skeleton answers Pair with Unimplemented, so pairing is only exercised through the daemon's own service",
    "no other blockers"
  ],
  "manualNotes": "The chain's deviation gate checks docs/tasks/metis-foundations/tickets/pairing-and-tokens/deviation-reports/pairing-and-tokens.md, which is why this report is written at that repo path as well as at the runtime output path. The prior attempt failed exactly that gate by writing only the artifact."
}
```
