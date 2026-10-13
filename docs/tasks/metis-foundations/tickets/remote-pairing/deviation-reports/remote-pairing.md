---
type: deviation report
title: Deviation report for remote-pairing
status: stable
---

## Deviation report: remote-pairing

Implemented on `ticket/remote-pairing` (HEAD `4651798`, four `wip:` commits off
`main` at `fd65ccc`, which is the landing branch's base; `task/remote-pairing`
does not exist yet). Nine files, `+577/-76`. Compared against the
`## Pending ticket: remote-pairing` section of
`docs/tasks/metis-foundations/arch-spec.md` and the ticket doc's acceptance
criteria. I re-ran the gates on `4651798` through `devenv shell --`: `cargo fmt
--check`, `cargo clippy --all-targets -- -D warnings`, the workspace `cargo test`
(146 passed, 7 ignored, 0 failed), `cargo test -p daemon` (auth 16, secret 12),
`buf lint`, `buf breaking --against '.git#branch=main'`, both drift checks, and
`cargo run -p plugin-fixture --bin abi-harness` (`[result] PASS (73 checks)`),
all green.

The approved interface contract matched almost exactly. What follows is where the
built surface differs from the plan as written, plus the additions the plan did
not name.

### API surface changes

- **Planned:** `pair_refusal_headers(&PairError)` in `daemon::auth`, with the spec
  naming the function and the three header values.
  **Actual:** `pair_refusal_headers(&PairError) -> Option<HeaderMap>`. `None` is
  the `PairError::Store` case, which carries no wire value, because a store fault
  is an internal failure rather than a pairing state the client should act on.
  The refusal is built with the headers only when there are some, so that case
  stays `ClientError::Call`.
  **Impact:** none on dependents. A later pairing refusal added to the
  non-exhaustive `PairRefusal` travels through the same function.

- **Planned:** the spec named no status code for `PairError::PairingDisabled`.
  **Actual:** `ErrorCode::FailedPrecondition`. `PermissionDenied` for a wrong key
  and `AlreadyExists` for an already paired surface are unchanged from the landed
  mapping.
  **Impact:** the status code is not what a client acts on, since the typed error
  is read from the header; a client that only inspects codes sees a new one for
  the closed state.

- **Planned:** `service(store, preshared: Option<Secret>, policy) ->
  ConnectRpcService<Router>`.
  **Actual:** that, plus the public `Service::new(store, Option<Secret>)`, because
  the service is what holds the key.
  **Impact:** any caller constructing a service directly, which today means only
  the landed test fixture, must pass `Some(key)` or `None`. The daemon effort is
  the first product caller.

- **Planned:** the spec placed the pair-error header name and its three values in
  `api_contracts::auth` without naming the type that carries them.
  **Actual:** `pub enum PairRefusal { Disabled, WrongKey, AlreadyPaired }`
  (non-exhaustive, `as_str`, `parse`, `Display`), mirroring the landed
  `CredentialRefusal`. `daemon::auth` re-exports `PAIR_ERROR_HEADER` exactly as it
  re-exports `AUTH_ERROR_HEADER`, and the client parses the header through
  `PairRefusal::parse`, so the spelling exists once.
  **Impact:** additive public surface in `api-contracts`; nothing else reads it.

- **Planned:** the interface contract's second seam is a wrong key as `WrongKey`
  and a repeated pairing as `AlreadyPaired`, read from the header.
  **Actual:** that, and four landed tests changed expectation to reach it:
  `pairing_mints_one_token_the_daemon_keeps_only_as_a_hash` now expects the typed
  already-paired refusal instead of `Call` carrying `AlreadyExists`;
  `the_pairing_gate_uses_the_configured_key_reference` and
  `a_wrong_preshared_key_is_refused_and_mints_nothing` now expect the typed
  wrong-key refusal instead of `Call` carrying `PermissionDenied`; and
  `the_configured_preshared_key_is_a_secret_reference` now expects `Ok(None)` for
  an unset or empty variable instead of `SecretError::Unresolved`.
  **Impact:** each assertion still asserts a refusal, so nothing was weakened. One
  assertion was lost rather than moved: the old test required the unset-key
  refusal text to name `METIS_PRESHARED_KEY`, and for an unset key there is no
  refusal text any more. The variable now appears only in `PairingDisabled`'s
  `Display`, which no test asserts.

- **Planned:** seam 4, "the same assertions run over the Unix socket and TCP
  through the landed fixture".
  **Actual:** no separate parity test for the typed refusals. The closed-pairing
  test runs its pairing assertions over both transports in one body, and the typed
  refusals are asserted on the remote transport by the new test and on the local
  socket by the landed ones. A third test repeating wrong-key and already-paired
  over both transports was judged verbatim duplication.
  **Impact:** the two-transport claim rests on the closed-pairing test rather than
  on a dedicated parity test; the `serve-tcp-service` parity test still covers the
  interceptor path itself.

### Abstraction usage

- Used/was specified: yes.
  - `serve_tcp_listener_service` and `serve_tcp_service` mount the policy on the
    remote listener, with `AuthPolicy::TokenRequired`; no second accept loop or
    front-end was added.
  - `AuthInterceptor`'s existing exemption of the `Pair` procedure is what lets
    the preshared key reach the handler on TCP. The exemption was not modified.
  - `api_contracts::auth` owns the channel shape as specified: the new header and
    `PairRefusal` mirror `AUTH_ERROR_HEADER` and `CredentialRefusal`, and the
    client reads the header instead of parsing prose.
  - `identity` headers still supply the surface the token row is filed under.
  - `daemon::state::Store` and `daemon::token::mint` are consumed unchanged: no
    second pairing path, mint, or store write.
  - The landed fixture in `crates/daemon/tests/auth.rs` is reused, including
    `serve_over_tcp`, `remote_client` and `reserve_loopback_port`; `AuthEnv`
    supplies the RAII environment handling.
  - No contract, generated tree, ABI layout, or plugin change, which the drift,
    lint, breaking and harness gates confirm.

### Out-of-scope changes

- `crates/daemon/tests/auth.rs` test support was strengthened rather than
  extended: `RecordingDaemon` is now a decorator that records the request path and
  then delegates to the daemon's own `Service`, instead of answering with canned
  payloads, so the fixture now exercises the real handler. `AuthEnv::unset`,
  `Fixture::unset_var` and `Fixture::serve_configured_daemon_over_tcp` were added.
  No landed assertion was dropped or loosened by that change.
- `docs/dev-env.md` and `docs/testing.md` were updated, which the spec asked for.
- Nothing was removed. The ticket's acceptance criteria, the wire contract, and
  the `Pair` procedure path are untouched.
- `docs/tasks/state.yaml` (the orchestration pointer) is left modified and
  uncommitted, as in the earlier landings; nothing is staged.

### Ticket doc update needed?

Yes, and it needs a `## Implementation notes` section, which the ticket doc does
not have yet. It should record:

- the landing paragraph (merge commit, landing branch, working branch, as the
  earlier tickets' notes do);
- the three acceptance criteria and their tests, with the second criterion's
  event-stream half named as `event-stream`'s;
- the closed-pairing rule as a state rather than a revocation, which is the
  behaviour the second test pins;
- the typed pair-error channel and the four landed tests whose expectations moved
  to it;
- the signature changes: `resolve_preshared_key` returning `Option`, `Service::new`
  and `service(...)` taking `Option<Secret>`, and the new `PairRefusal`;
- the fixture decorator change and the new helper methods;
- the gates re-run here, and the residuals below.

### User attention needed?

Yes, on settled facts rather than on an open decision.

- Two landed signatures changed rather than gained a sibling:
  `resolve_preshared_key() -> Result<Option<Secret>, SecretError>` and
  `Service::new`/`service(...)` taking `Option<Secret>`. Both are the approved
  contract, and both are visible to the daemon effort, which is the next
  consumer; a caller that ignores the `None` arm would treat a closed pairing as a
  configured one.
- Pairing over a network-reachable listener has no rate limit and no lockout, and
  this ticket is what makes that reachable. The constant-time comparison is
  unchanged from `pairing-and-tokens`, and the plaintext exposure is the one ADR
  0004 accepts, but brute force over a reachable listener is a posture the effort
  has not decided on.
- The two new tests live in `crates/daemon/tests/auth.rs`, whose `#![cfg(unix)]`
  gates the whole binary, so remote pairing has no test coverage on non-Unix
  targets. That is the gate `serve-tcp-service` recorded and it is unchanged.

### Residual risks carried forward

- Pairing over the plaintext remote transport exposes the preshared key to anyone
  who can read the connection. `reverse-proxy-mode` owns the warning and the
  trusted-proxy rule.
- The event stream half of criterion 2 is unproven by design; the contract
  declares no streaming RPC until `event-stream` lands, and that ticket inherits
  both the token requirement and the pairing exemption through the interceptor.
- Closing pairing revokes nothing, as approved and asserted.
- The daemon binary still mounts no listener, so wiring the optional key and
  `AuthPolicy::TokenRequired` into a real remote listener remains the daemon
  effort's work.
