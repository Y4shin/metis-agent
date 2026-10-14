---
type: deviation report
title: Deviation report for tcp-listener
status: stable
---

## Deviation report: tcp-listener

Implemented on `ticket/tcp-listener` (HEAD `1d93f42`, four `wip:` commits off
`main` at `c63a56e`, one per seam). The task's suggested comparison
`git diff task/tcp-listener..ticket/tcp-listener` cannot run: `task/tcp-listener`
does not exist, because nothing creates the landing branch until the land step.
The comparison below is `git diff main...HEAD` over the six touched files.

Gates I re-ran on `1d93f42` through `devenv shell --`: `cargo fmt --check`
(`FMT-OK`), `cargo clippy --all-targets -- -D warnings` (clean, no diagnostics),
the workspace `cargo test` (29 targets ok, no `FAILED` line),
`cargo test -p api-contracts` (3 passed, the configuration unit tests), and
`cargo test -p client --test remote` (3 passed). No new file carries an em-dash
or an en-dash in added lines.

### API surface changes

- **Planned:** the spec's export list names `serve_tcp(addr, router)` as the TCP
  front-end, with `serve_remote` binding and reporting the address.
  **Actual:** two public functions. `serve_tcp` is now a bind plus a delegate to
  `serve_tcp_listener(listener, router)`, and `serve_tcp_listener` holds the one
  accept loop and is what `skeleton::serve_remote` calls. `serve_tcp` is called
  by nothing today: no test, no caller, no daemon wiring yet.
  **Impact:** additive, and the "one serve loop" rule holds. The reason is
  ordering: `serve_remote` must bind first to report the kernel-assigned port
  for `127.0.0.1:0`, and a reserve-then-rebind scheme would race the client's
  first connect. The cost is that the exported front-end the daemon effort will
  wire into `main` is the two-line wrapper no test drives, while the tested path
  is the listener-taking one. `tls-and-client-trust` and `reverse-proxy-mode`
  consume `serve_tcp_listener`'s shape for the same reason.

- **Planned:** "Its authority is `http://<addr>`".
  **Actual:** `remote_authority(addr) -> http::Uri`, a public helper that
  `tcp_transport` and `Client::with_remote_addr` both call, mirroring the landed
  `local_authority()`.
  **Impact:** additive and behaviour-neutral. Without it the `http://<addr>`
  rule would be duplicated, which is how the connection authority and the
  request authority drift apart. A non-loopback authority (a proxy or a TLS SNI
  host) is the next consumer.

- **Planned:** `ConfigError` is described as the parse failure, with the variable
  and the value named.
  **Actual:** `ConfigError::InvalidRemoteAddr { variable, value, source }` as
  specified, plus `ConfigError::RemoteBind { addr, source }`, because
  `serve_remote` returns `Result<_, ConfigError>` and a bind can fail; without
  the variant the start path would have to panic. The enum stays
  `#[non_exhaustive]`.
  **Impact:** downstream transport tickets add their own settings and bind
  errors without a breaking change. No wire or ABI surface moves.

- **Unchanged, and verified as unchanged by the file list:** `serve_stream`,
  `serve_uds`, `uds_transport`, `local_authority`,
  `Client::{local, with_endpoint, echo}`, the checked-in contract and its
  generated trees, the ABI layout, and every `Cargo.toml`. The diff touches six
  files: `crates/api-contracts/src/{lib,transport}.rs`,
  `crates/client/src/{lib,skeleton}.rs`, `crates/client/tests/remote.rs`, and
  `docs/testing.md`.

### Abstraction usage

- Used/was specified: yes.
  - `serve_stream` stays the only connection path, and `serve_tcp_listener`
    clones one `Arc<Router>` per accepted stream exactly as `serve_uds` does. No
    second serve loop, no second service, no per-connection router build.
  - `skeleton::router()`: the remote test serves the same router the socket test
    serves, so it measures the transport and not a parallel path.
  - `connectrpc::ConnectRpcService` and the `hyper_util` auto builder stay
    internal to `serve_stream`.
  - `docs/testing.md` gained the remote run command and the variable, as the
    spec required.
  - Seams 1 to 5 all have a named test: `an_absent_or_empty_remote_addr_disables_the_listener`,
    `an_ip_literal_with_a_port_is_the_bind_address`,
    `a_value_that_is_not_an_ip_port_literal_is_a_typed_error` (seam 1, unit);
    `no_remote_configuration_opens_no_tcp_socket` (seam 2, with the reserved
    port rebound after the call); `the_client_echoes_over_the_remote_listener`
    (seams 3 and 4, echo value asserted through `Client::with_remote_addr`); and
    `the_listener_task_releases_its_port_when_it_is_dropped` (seam 5).
  - The off-by-default rule is twice as strong as the spec asked: the test
    asserts `Ok(None)` and then requires a successful bind on the reserved port,
    so "nothing was bound" is measured rather than inferred.
- Not as specified, and worth folding later: the RAII environment helper now
  exists three times, `crates/client/tests/echo.rs::EndpointOverride`,
  `transport.rs::RemoteAddrEnv`, and
  `crates/client/tests/remote.rs::RemoteAddrOverride`. The spec named the first
  as prior art for the new test binary, so the second copy is expected; the
  third copy, inside the unit tests, is duplication the coherence pass could
  fold. Each copy carries its own mutex, and the three live in three test
  binaries, so nothing races today.

### Out-of-scope changes

- Comment and doc-only edits the spec did not list: `crates/api-contracts/src/lib.rs`
  and the `transport` module doc no longer call the module "the local transport",
  and the `client` crate doc names `with_remote_addr` and the off-by-default
  rule. Correct, because the module now serves TCP as well; no behaviour.
- Nothing was removed. No test was weakened. No contract, generated tree, layout,
  or manifest file changed. No TLS, proxy, or authentication code was added, and
  no `enabled` flag, config crate, or public transport injection point appeared.

### Residual risks

- Plaintext only, and an enabled listener serves the same unauthenticated
  surface the local socket serves until `local-identity-and-acl` and
  `pairing-and-tokens` land. The required, defaultless bind address is the only
  mitigation today.
- `serve_tcp` is exported but unexercised; the daemon effort is its first caller.
- `remote_authority` panics on an unparseable authority, in practice never, which
  matches the landed `local_authority` style.
- The IPv6 form is covered by the configuration unit test but not by an
  end-to-end listener test.
- `serve_remote` and `serve_tcp_listener` stop only when their task or runtime is
  dropped; there is no graceful shutdown, matching `serve_uds`.

### Ticket doc update needed?

Yes. The ticket doc has no `## Implementation notes` section (it ends at
`## Blocked by`), so the land-worker must create the heading and record:

- the landing paragraph (merge commit, landing branch, working branch, as the
  two earlier plugin tickets' notes do);
- that acceptance criterion 4 was narrowed before implementation to the echo RPC,
  with the stream-over-TCP assertion assigned to the ticket that owns the first
  streaming RPC;
- the surfaces added beyond the approved spec: `serve_tcp_listener`,
  `remote_authority`, `ConfigError::RemoteBind`, and the fact that `serve_tcp`
  has no caller or test yet;
- the rule the operator approved: `METIS_TCP_ADDR` holding an `IP:port` literal
  is the whole of the switch, an absent or empty value disables the listener, and
  a hostname or a malformed value is a typed error rather than a silent disable;
- the test inventory in `crates/client/tests/remote.rs` and the three
  configuration unit tests in `crates/api-contracts/src/transport.rs`, plus the
  gates re-run here;
- the residual risks above.

### User attention needed?

Yes, on two settled facts rather than on an open decision.

- The approved export list grew by two public helpers and one error variant.
  Each is recorded above with its reason, and the arch spec needs the
  reconciler's pass to name them; no behaviour the operator approved changed.
- `serve_tcp` is the daemon's future entry point and no test drives it. If the
  operator wants it exercised before the daemon effort adopts it, the cheapest
  fix is one extra assertion in `serve_remote`'s bind step or a direct
  `serve_tcp` test; otherwise it stays a documented gap.

No spec conflict and no further operator decision is required before landing.
