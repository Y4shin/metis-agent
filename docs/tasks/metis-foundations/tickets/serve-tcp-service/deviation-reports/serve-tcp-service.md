---
type: deviation report
title: Deviation report for serve-tcp-service
status: stable
---

## Deviation report: serve-tcp-service

Implemented on `ticket/serve-tcp-service` (HEAD `c4c2c16`, seven `wip:` commits
off `main` at `fb36073`). Compared against the `## Pending ticket:
serve-tcp-service` section of `docs/tasks/metis-foundations/arch-spec.md` and the
ticket doc's acceptance criteria. I re-ran the gates on `c4c2c16` through
`devenv shell --`: `cargo fmt --check` reports `FMT-OK`, `cargo clippy
--all-targets -- -D warnings` is clean, `cargo test -p daemon --test auth` passes
14 tests, the workspace `cargo test` reports 144 passed, 0 failed and 7 ignored,
`buf lint` and `buf breaking --against '.git#branch=main'` pass,
`bash scripts/regenerate-contracts.sh --check` and `bash
scripts/regenerate-abi-header.sh --check` report no drift, and
`cargo run -p plugin-fixture --bin abi-harness` reports
`[result] PASS (73 checks)`.

Three files changed: `crates/api-contracts/src/transport.rs` (65 lines),
`crates/daemon/tests/auth.rs` (285 lines), `docs/testing.md` (9 lines).

### API surface changes

- **Planned:** "`serve_tcp_listener_service(listener, service)` ... the accept
  loop, building one `Arc` for the router per accepted stream and delegating each
  to the private `serve_connection` body".
  **Actual:** the loop wraps the service in one `Arc` and hands each accepted
  stream a clone of the service (`Arc::clone(&service)` then `(*service).clone()`)
  before calling `serve_connection(stream, service)`. One `Arc` over the service
  is the same idea one level up, and it is what lets an interceptor chain travel,
  because the chain lives on the service and not on the router.
  **Impact:** none on callers. The spec's wording should follow the service,
  since the router is no longer the value the loop clones.

- **Planned:** "`serve_tcp_service(addr, service)`: bind plus delegate, mirroring
  the landed `serve_tcp` and `serve_tcp_listener` split", with the landed
  functions keeping "their signatures and behaviour".
  **Actual:** exactly that, and `serve_tcp_listener` is now
  `serve_tcp_listener_service(listener, ConnectRpcService::from_arc(Arc::new(router)))`,
  so the landed function keeps its signature and its observable behaviour but no
  longer holds an accept loop of its own. `serve_stream` was reduced the same way,
  to `serve_connection(stream, ConnectRpcService::from_arc(router.into()))`.
  **Impact:** the two new functions are the only public additions (`git diff
  main...HEAD` shows exactly two `+pub async fn` lines in that file), and the
  landed signatures are unchanged, which is what the ticket's third criterion
  requires. `client::skeleton::serve_remote` still calls
  `serve_tcp_listener(listener, router())`, no existing test changed, and the
  module doc now names both families.

- **Planned:** seam 3, "`serve_tcp_service(127.0.0.1:0, service)` serves, and the
  listener-taking twin is the entry point a caller uses when it needs
  `local_addr()` first".
  **Actual:** `serve_tcp_service` returns only `io::Result<()>` and so cannot
  report a kernel-assigned address. The bind test reserves a loopback port with
  bind-and-release, passes that address to `serve_tcp_service`, and waits for the
  listener to accept. Port 0 is exercised through
  `serve_tcp_listener_service`, which is what `serve_over_tcp` uses for the other
  three tests.
  **Impact:** the reserved-port pattern is racy in principle on a busy host, and
  it is recorded as a residual. The port-0 half of the seam is covered through
  the listener-taking twin rather than through the address-taking function.

- **Planned:** the ticket's criterion 2 asks for a refusal "with the typed
  missing-credential error" on the TCP front-end.
  **Actual:** asserted end to end through the client as
  `ClientError::CredentialMissing`, repeated on both front-ends in the parity
  test, with the `Served` recorder proving the handler never ran.
  **Impact:** none. No new error type, header or error variant appears in this
  ticket.

### Abstraction usage

- Used/was specified: yes.
  - The private `serve_connection` body is the one connection path:
    `serve_stream`, `serve_uds_service` (unchanged) and
    `serve_tcp_listener_service` all reach it, and the repository still holds
    exactly two accept loops, the Unix one in `serve_uds_service` and the TCP one
    in `serve_tcp_listener_service`.
  - `serve_uds_service`'s service-taking shape is mirrored rather than
    reinvented, and the ACL application inside `serve_uds_service` is untouched.
  - `ConnectRpcService<Router>` from `daemon::service::service(...)` is mounted
    directly, which is the point of the ticket: the interceptor chain lives on
    the service and now has a TCP front-end to travel through.
  - `daemon::auth::{AuthInterceptor, AuthPolicy}`, the auth fixture, the `Served`
    recorder, `fixture.remote_client`, `client::credential` and `Client::pair`
    are consumed, not reimplemented, and no credential logic is re-tested here.
  - Test prior art: the new TCP helpers (`serve_over_tcp`,
    `reserve_loopback_port`, `wait_for_tcp`) copy the loopback bind and
    `local_addr()` pattern from `crates/client/tests/remote.rs`, and the four new
    tests are appended to the existing auth suite, 10 tests before and 14 after.
  - Additivity: the contract, the generated Rust, the committed C header, the
    layout assertions and the plugin crates are untouched, and `buf lint`,
    `buf breaking`, both drift checks and the 73-check harness are green.
  - The implementer also ran a mutation check: making
    `serve_tcp_listener_service` ignore the mounted service failed exactly the
    four new tests, and the file was restored byte for byte. That is the
    strongest evidence available here that the new tests are load-bearing rather
    than vacuous.

### Out-of-scope changes

- `docs/testing.md`: one bullet extended to describe the service-taking TCP
  front-ends, why they exist, and the parity assertion. No acceptance criterion
  asks for it; it follows how the earlier transport tickets documented their run
  paths.
- `crates/api-contracts/src/transport.rs` module and function doc comments were
  updated to describe the two front-end families. Comments only.
- `docs/tasks/state.yaml` remains modified and unstaged in the working tree,
  exactly as the orchestrator left it, matching the `pairing-and-tokens` chain.
- Nothing was removed, and no file outside the three named was touched.

### Ticket doc update needed?

Yes. The ticket doc has no `## Implementation notes` section yet, so the
land-worker must create the heading, then record:

- the landing paragraph (merge commit, landing branch, working branch, as the
  earlier notes do);
- that `serve_stream` and `serve_tcp_listener` now delegate to the new body and
  loop, with signatures and behaviour unchanged and no existing test changed;
- the two public additions, and that they are the only ones;
- the test inventory: four new tests in `crates/daemon/tests/auth.rs`
  (`the_tcp_service_front_end_refuses_a_call_with_no_credential`,
  `the_bind_and_delegate_front_end_serves_the_mounted_service`,
  `pairing_stays_exempt_over_the_tcp_service_front_end`,
  `a_service_mounted_on_both_front_ends_answers_the_same_way`), 10 tests to 14,
  with the three TCP helpers;
- the residuals: the four new tests are Unix-gated because that binary is
  `#![cfg(unix)]`; the bind seam's reserved-port race; the delegation change to
  two landed function bodies;
- the gates re-run here, including the mutation check the implementer ran.

### User attention needed?

Yes, on three settled facts rather than on an open decision.

- `serve_tcp_listener`'s body changed from holding the accept loop to delegating
  to it, so "keeps its signatures and behaviour" holds for both signature and
  behaviour but not for byte-identity of the function.
- The four new tests live in a Unix-gated test binary, so the service-taking TCP
  front-end has no test coverage on non-Unix targets, even though TCP is the
  first transport every target has.
- The bind seam's reserved-port pattern is racy in principle, and port-0
  reporting is covered only through the listener-taking twin.

No spec conflict and no further operator decision is required before landing.
