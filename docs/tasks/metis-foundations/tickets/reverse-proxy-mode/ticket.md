---
type: ticket
subtype: feature
title: Reverse-proxy mode and the plaintext warning
status: stable
workflow_state: ready
blocked_by: [tls-and-client-trust]
size: s
---

## What to build

The deployment shape where a reverse proxy terminates TLS, and an honest warning
in every other plaintext case.

## Acceptance criteria

- [ ] Configuration names trusted proxy addresses, and forwarded headers are
      honoured only when the peer is one of them and ignored otherwise.
- [ ] A non-loopback plaintext listener without named trusted proxies emits the
      startup warning, and naming a trusted proxy suppresses it.
- [ ] A test asserts forwarded information is accepted from a named proxy and
      ignored from an unnamed peer.

## Blocked by

- `tls-and-client-trust`, because proxy mode is the alternative to terminating
  TLS in the daemon and shares the listener configuration.

## Implementation notes

Landed on `task/reverse-proxy-mode` as the merge commit `2d87824`
("ticket(reverse-proxy-mode): Reverse-proxy mode and the plaintext warning"),
parents `a81ec4f` and `7927f61`. The landing branch did not exist when this
ticket was landed, so it was created at `main`'s tip (`a81ec4f`,
"docs(reverse-proxy-mode): approve the arch spec"), the convention the shared
conventions section of `docs/tasks/metis-foundations/arch-spec.md` records and
the one the earlier landings used. `7927f61` is the working branch
`ticket/reverse-proxy-mode`, eight `wip:` commits off `main` at `a81ec4f`. The
merge was clean with no conflicts, and the landing tree is byte-identical to the
verified tip (both trees hash to `c20fdb02`, and `git diff 7927f61 HEAD` is
empty), so every source file landed exactly as verified and nothing was re-run
at landing. The branch `ticket/reverse-proxy-mode` was deleted after the merge,
and the deviation report is committed alongside this note at
`docs/tasks/metis-foundations/tickets/reverse-proxy-mode/deviation-reports/reverse-proxy-mode.md`.

The gates recorded against `7927f61` through `devenv shell --` therefore hold
for the landed tree: `cargo fmt --check` exit 0; `cargo clippy --all-targets -- -D
warnings` exit 0 (the only stderr is the pre-existing `plugin-contract` C
build-script `_FORTIFY_SOURCE` line); the workspace `cargo test` exit 0 with no
`FAILED` target and no failures anywhere; `cargo test -p api-contracts --lib
proxy` at 12 passed, 0 failed; and `cargo test -p daemon --test proxy` at 3
passed, 0 failed. The tdd worker also reports `buf lint` exit 0 and the ABI
harness at `[result] PASS (73 checks)`, both consistent with the wire contract,
the ABI crate, and the plugin crates being untouched by this ticket.

What landed, 8 files and +1210/-12 against `main`: `crates/api-contracts/src/proxy.rs`
(new, mounted as `pub mod proxy;` in `crates/api-contracts/src/lib.rs`) with
`TRUSTED_PROXIES_ENV` (`METIS_TRUSTED_PROXIES`), `TrustedProxies`
(`resolve`, `parse`, `contains`, `is_empty`, `Display`),
`ConfigError::InvalidTrustedProxy`, `ClientAddress { Peer, Forwarded }` with
`client_address`, `ClientScheme { Plaintext, Encrypted }` with `client_scheme`,
`Warning { PlaintextListener, PlaintextSecret }` with the remedial `Display`, the
two pure decisions `listener_warning` and `secret_warning`, and `WarnOnce`;
the TCP accept loop in `crates/api-contracts/src/transport.rs` now resolves the
trusted set where it binds (a malformed value refuses the bind), prints
`listener_warning` once before accepting, and stamps `connectrpc::PeerAddr` per
accepted connection through a new private `PeerStamped` tower service, with
`serve_connection` generalized over the wrapped service; `crates/daemon/src/service.rs`
carries a `WarnOnce` and the `Pair` handler warns once on its plaintext leg;
the new `crates/daemon/tests/proxy.rs` at the peer-stamp seam; and
`docs/dev-env.md` plus `docs/testing.md` carrying the variable, the forwarded
rule, the warning rule, and the run commands. Each of the three acceptance
criteria above traces to behaviour-asserting tests, as the deviation report
records, and the accepted-versus-ignored assertion is present twice, as a unit
test of the rule and over two real loopback listeners.

Residual risks carried into the effort, all recorded by the tdd worker, the
deviation report, and the verifier. The front-end's `eprintln!` of the listener
warning is not asserted directly because libtest captures stderr; only the pure
decision and the wiring are covered, the wiring indirectly by every TCP test
that binds through it. The integration test builds the trusted set with
`TrustedProxies::parse` and hands it to the observer, so `METIS_TRUSTED_PROXIES`
env resolution is unit-tested and wired at bind but not exercised end to end. The
`Pair` handler resolves the env per request, so a malformed value turns a pairing
call into `ErrorCode::Internal`; that path is unspecified by the arch spec and
untested. `WarnOnce` lives on `Service` and `listener_warning` runs per bind, so
a multi-listener process can emit one warning per listener rather than exactly
one per process. A named proxy's `x-forwarded-proto` is believed with no way to
verify it and a multi-hop chain is not walked, both of which are the ADR 0004
residuals. Finally, the secret-bearing `Pair` handler kept its landed
`Service::new` signature, which three foreign test suites call, so it reads the
trusted set per call rather than from a configuration field.

`docs/tasks/state.yaml` came into the landing branch through the merged working
branch (commit `d069baa` set `task: reverse-proxy-mode`, matching what the
harness wrote into the working tree). It is not touched by this landing commit,
which holds only the ticket doc and the deviation report, and its final value
stays with `finalize-task` together with the ticket's `workflow_state` and the
archive.
