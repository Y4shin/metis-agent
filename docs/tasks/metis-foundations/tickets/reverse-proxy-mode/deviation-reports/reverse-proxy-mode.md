---
type: deviation report
title: Deviation report for reverse-proxy-mode
status: stable
---

## Deviation report: reverse-proxy-mode

Implemented on `ticket/reverse-proxy-mode` (HEAD `7927f61`, eight `wip:` commits
off `main` at `a81ec4f`). Compared against the approved
`## Pending ticket: reverse-proxy-mode` section of
`docs/tasks/metis-foundations/arch-spec.md` (approval record `.work/approval.md`,
committed as `a81ec4f`) and the ticket doc's three acceptance criteria. No landing
branch existed when this report was written, so the comparison is against `main`.

I re-ran the ticket's own two test binaries on `7927f61` through `devenv shell
--`: `cargo test -p api-contracts --lib proxy` (12 passed, 0 failed, 48 filtered
out) and `cargo test -p daemon --test proxy` (3 passed, 0 failed). The workspace
suite, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `buf
lint` and the ABI harness (`[result] PASS (73 checks)`) are the tdd worker's
reported results on the same commit, and the slice verifier gates them
independently.

### API surface changes

Every planned export landed with the planned signature. The items below are the
places where the actual surface is wider, narrower, or differently placed than
the section said, in the section's own order.

- **Planned:** `crates/api-contracts/src/proxy.rs` exporting
  `TRUSTED_PROXIES_ENV`, `TrustedProxies` with `resolve`, `parse`, `contains`,
  `is_empty` and `Display`, `ClientAddress { Peer, Forwarded }` with `address()`
  and a provenance `Display`, `client_address`, `ClientScheme`, `client_scheme`,
  `Warning { PlaintextListener, PlaintextSecret }` with a remedial `Display`, the
  two decisions `listener_warning` and `secret_warning`, and `WarnOnce`.
  **Actual:** all of it, at `crates/api-contracts/src/proxy.rs`, mounted as
  `pub mod proxy;` in `crates/api-contracts/src/lib.rs` with a module paragraph
  in the crate docs. The additions the section did not name are small:
  `WarnOnce::new()` is `const` and the type is `Default`, `warn(&Warning) ->
  bool` reports whether this call was the one that emitted (which is what makes
  the latch testable without capturing stderr), `ClientAddress` is `Copy` with
  `Debug` and `PartialEq`, and `TrustedProxies` is `Debug` and `Clone`.
  `parse` skips an empty entry between commas rather than refusing it, and
  `Display` writes the addresses comma separated with no trailing separator.
  **Impact:** none on dependents. `windows-named-pipe` and the daemon effort read
  `client_address` and the two decisions, and both keep their planned shapes.

- **Planned:** `ConfigError::InvalidTrustedProxy { variable, value }` on the
  non-exhaustive enum, with a `Display` arm.
  **Actual:** exactly that, including the matching `source()` arm that returns
  `None`.
  **Impact:** none. The variant is the third configuration error on the same
  enum, which is what the enum was declared to gain.

- **Planned:** `serve_tcp_listener_service` reads `stream.peer_addr()` and stamps
  `connectrpc::PeerAddr(peer)` into each request's extensions before the mounted
  service sees it; the private `serve_connection` and the single accept loop keep
  their shape; the front-end resolves `METIS_TRUSTED_PROXIES` where it binds and
  prints `listener_warning(listener.local_addr(), &trusted)` once before
  accepting.
  **Actual:** the stamp happens, but through two private items the section did
  not name: a `PeerStamped<P>` tower service that inserts the extension in
  `call`, and a generalized `async fn serve_connection<S, P>(stream: S, service:
  P)` whose `P` is any `tower::Service<http::Request<hyper::body::Incoming>,
  Response = http::Response<connectrpc::ConnectRpcBody>, Error = Infallible>`.
  The wrapper is not a `ConnectRpcService`, so the body could not keep its old
  concrete parameter. The bind-time resolve and the single `eprintln!` warning
  are as specified, and the accept loop is still the only one.
  **Impact:** the generalized helper is private and both callers are in the same
  file, so no dependent ticket sees it. The real coupling to record is that the
  bound now names `connectrpc::ConnectRpcBody`, so a connectrpc upgrade that
  renames the body type reaches this private signature. Behaviour change worth
  naming: a `METIS_TRUSTED_PROXIES` value this build cannot parse now refuses the
  bind with the `ConfigError` wrapped in `io::Error` instead of being ignored,
  which the front-end documents and a unit test pins.

- **Planned:** the daemon's `Pair` handler asks `secret_warning` with the
  request's peer and scheme and emits through the service's `WarnOnce`, with no
  wire change and no signature change.
  **Actual:** as planned, with two additions: `Service` gained a private
  `warnings: WarnOnce` field (`Service::new`'s signature is unchanged, so every
  caller and foreign test is unaffected), and a new private
  `refuse_trusted_proxies(ConfigError) -> ConnectError` maps an unreadable set to
  `ErrorCode::Internal`. The handler resolves `TrustedProxies` per call rather
  than holding it as configuration, which the section did not call for and
  `Service::new`'s landed signature left open.
  **Impact:** none on dependents. A valid configuration behaves identically, and
  on an unreadable one a pairing call fails as `Internal` rather than judging
  exposure on configuration nobody could parse. That path is a behaviour the
  approved section did not specify and no test asserts; it is a residual rather
  than a contract.

- **Planned:** `crates/client/src/skeleton.rs` unchanged in shape.
  **Actual:** untouched, byte for byte.
  **Impact:** none. The client's remote path reaches the stamped front-end, so
  the peer stamp arrives there for free.

- **Planned:** `docs/dev-env.md` gains the variable beside the remote address and
  `docs/testing.md` gains the rule and warning commands.
  **Actual:** `docs/dev-env.md` gained a "Remote exposure and reverse proxies"
  section covering the variable, the trust rule, the left-most-entry rule, the
  no-TLS decision and the two warnings; `docs/testing.md` gained a
  "Reverse-proxy mode" command list naming the two test binaries and what each
  asserts. The wire contract, the identity headers, the credential and the event
  stream are untouched, `buf lint` is green, and the ABI harness stays at 73
  checks.

### Abstraction usage

- Used/was specified: yes.
- `ConfigError`'s non-exhaustive enum and `resolve_remote_addr`'s shape (absent
  or empty means off, a malformed value is a loud typed error) are what
  `TrustedProxies::resolve` and `parse` follow, and the new variant is the one
  the enum's own doc reserved for the transport tickets that follow.
- The landed accept loop is extended rather than duplicated: one loop, one
  `serve_connection`, one private stamping wrapper, and `serve_tcp`,
  `serve_tcp_listener`, `serve_tcp_service` and `serve_tcp_listener_service` keep
  their public signatures and reach it.
- `connectrpc::PeerAddr` with `RequestContext::peer_addr()` is the stamp and the
  read, not a hand-rolled extension type, so a handler reads the address the
  library's own accept loop would have given it.
- `serve_uds_service`'s `PathAcl::from_env()` is the precedent for reading
  listener configuration where the listener binds, and the front-end's existing
  `eprintln!` is what the warning uses.
- Deviation in usage worth naming: the daemon's `Pair` handler resolves the
  environment per call instead of holding the set as service configuration. The
  section named no configuration field and `Service::new` is landed API that
  three foreign test suites call, so the implementer left the signature alone.
  The consequence is that the front-end and the handler each read the variable,
  and a value that changes between them (or that cannot be parsed at all) is
  resolved twice.
- The integration test carries its own `Observer`, `serve_over_tcp` and
  `tcp_client` in `crates/daemon/tests/proxy.rs` rather than reusing the daemon
  stream test's fixture, which follows the repo's per-binary fixture convention
  and keeps the new binary independent of the stream suite.

### Out-of-scope changes

- **`docs/tasks/state.yaml` is inside commit `d069baa`.** A `git add -u` swept
  the harness task pointer, so one `wip:` commit contains `task: null` to
  `task: reverse-proxy-mode`. Its content equals what the harness wrote into the
  working tree, so nothing is dirty now, but every earlier landing left the
  pointer out of the landing commit and cleared it at finalize. The landing step
  should drop that hunk or the finalize sequence will commit it twice.
- Two additions beyond the seam list, both additive test coverage rather than
  new surface: the TCP front-end refuses to bind when the configured set cannot
  be read (`a_malformed_trusted_proxy_value_refuses_the_tcp_front_end`), and
  `Warning`'s `Display` is asserted to read as one sentence naming what is
  exposed and the remedy.
- A new test binary, `crates/daemon/tests/proxy.rs` (288 lines, three tests),
  rather than adding cases to `daemon/tests/auth.rs`. The section asked for
  integration tests over a real loopback listener in the daemon's suite and did
  not name a file, so this is a placement choice, not a scope change.
- No removals, no renamed existing API, no dependency change (no new crate in any
  manifest), and nothing outside `api-contracts` and `daemon` except the two
  documentation files.

### Ticket doc update needed?

Yes. Append to `## Implementation notes`:

- The private items the plan did not name: `PeerStamped<P>` and the generalized
  `serve_connection<S, P>`, with the `connectrpc::ConnectRpcBody` coupling its
  bound carries.
- The per-call `TrustedProxies::resolve()` in the `Pair` handler, `Service`'s new
  private `warnings: WarnOnce` field, and the `ErrorCode::Internal` refusal for an
  unreadable set, which no test asserts.
- The two rules the section left open: a forwarded entry that is not an IP
  literal falls back to the peer (`the_rule_never_invents_an_address`), and an
  empty entry between commas is skipped.
- The `docs/tasks/state.yaml` hunk in `d069baa` and what the landing step should
  do about it.
- The residual that `WarnOnce` is per `Service` and per listener rather than once
  per process, and that the front-end's `eprintln!` itself is not asserted
  because libtest captures stderr; the tested decision is what the front-end
  prints.

### User attention needed?

Yes, briefly, and none of it changes the three acceptance criteria or a
dependent surface. Three details the approved section left to the implementer
became behaviour: a junk `x-forwarded-for` entry from a named proxy falls back to
the peer instead of inventing an address; an unreadable `METIS_TRUSTED_PROXIES`
now refuses the TCP bind and, separately, turns a pairing call into an
`ErrorCode::Internal` refusal; and the "once per process" warning the section
described is once per service instance and once per listener in reality. Also
worth a decision at landing: the `state.yaml` hunk inside `d069baa`.
