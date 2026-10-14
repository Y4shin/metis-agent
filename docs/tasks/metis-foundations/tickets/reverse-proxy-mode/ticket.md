---
type: ticket
subtype: feature
title: Reverse-proxy mode and the plaintext warning
status: stable
workflow_state: done
blocked_by:
- tls-and-client-trust
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

Knowledge harvested at finalize. The effort map's ADR 0004 bullet now records
what landed: `METIS_TRUSTED_PROXIES` names exact proxy addresses, the two
forwarded headers are read only from a named peer, the TCP front-end stamps the
peer per connection so a handler can decide, a non-loopback bind with no proxy
named warns once at bind, and a secret-bearing pairing call on a plaintext client
leg warns once per service. What the daemon still cannot do is detect an operator
who exposes the listener with no proxy, which is the ADR 0004 residual and stays.

The arch spec carries the measured details rather than the plan's: the warning
latch is once per `Service` instance and once per listener rather than once per
process, as the approved section said; the front-end's `eprintln!` is not asserted
because libtest captures stderr, so the tested artefact is the pure
`listener_warning` decision the front-end prints; and the bind-time environment
resolve is covered indirectly, because the integration test passes the trusted set
to `serve_tcp_listener_service` directly and `METIS_TRUSTED_PROXIES` is never
exercised end to end. Three behaviours the section left to the implementer became
real: a junk `x-forwarded-for` entry from a named proxy falls back to the peer
instead of inventing an address, an empty entry between commas is skipped, and an
unreadable variable refuses the TCP bind while separately turning a pairing call
into an `ErrorCode::Internal` refusal that no test asserts.

The chain history is the third instance of the same lesson in this effort's last
three tickets, and the first that was not a budget problem: the first attempt
passed its preconditions gate and then lost its tdd child to a provider error
before any work existed, and the same-protocol retry at a 50 percent larger budget
landed the ticket with eight `wip:` commits. A provider failure with no work
produced needs no split and no diagnosis beyond confirming the branch and the tree
are untouched, which is what the retry's `extra` recorded. This ticket also left
the `state.yaml` pointer inside the working branch, so a later change to that
split should note that the pointer can travel in a `wip:` commit rather than only
in the harvest commit.
