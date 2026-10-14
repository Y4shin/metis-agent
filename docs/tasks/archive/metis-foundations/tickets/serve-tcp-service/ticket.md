---
type: ticket
subtype: feature
title: Service-taking TCP front-end
status: deprecated
workflow_state: done
blocked_by: []
size: s
---

## What to build

The TCP twin of `serve_uds_service`: a front-end that serves a mounted
`ConnectRpcService` rather than a bare `Router`, so the remote listener can
enforce the credential policy `pairing-and-tokens` added.

`connectrpc` 0.9.1 keeps the interceptor chain on `ConnectRpcService` and offers
no conversion back to a `Router`, so an auth policy cannot travel through the
landed `serve_tcp(addr, Router)` and `serve_tcp_listener(listener, Router)`.
Until this ticket lands, a TCP listener mounted through the landed surface serves
unauthenticated, which is the hole the credential work exists to close.

This is its own ticket rather than part of `pairing-and-tokens` because it is
transport wiring with its own test, and because the credential ticket's criteria
cover the lifecycle rather than the listener surfaces.

## Acceptance criteria

- [ ] `serve_tcp_service` serves a `ConnectRpcService<Router>` over TCP,
      mirroring `serve_uds_service` and reusing the same private connection body
      as `serve_stream`, with no second accept loop. If the bind-and-report shape
      needs it, a listener-taking twin accompanies it, the way `serve_tcp`
      delegates to `serve_tcp_listener`.
- [ ] A test mounts the daemon's service with `AuthPolicy::TokenRequired` over a
      real TCP socket: a call with no credential is refused with the typed
      missing-credential error, a call with a paired token is served, and the
      `Pair` procedure stays exempt.
- [ ] The landed `serve_stream`, `serve_uds`, `serve_uds_service`, `serve_tcp` and
      `serve_tcp_listener` keep their signatures and behaviour, and the existing
      transport tests are unchanged.
- [ ] No contract, generated tree, ABI layout or plugin change: the drift, lint,
      breaking and harness gates stay green.

## Blocked by

Nothing. It completes the transport surface `pairing-and-tokens` introduced, and
`remote-pairing` is blocked by it.

## Implementation notes

Landed on `task/serve-tcp-service` as the merge commit `4f57d3d`
("ticket(serve-tcp-service): Service-taking TCP front-end"), parents `fb36073`
and `c4c2c16`. The landing branch did not exist when this ticket was landed, so
it was created at `main`'s tip (`fb36073`), which is the previous ticket's
landing branch, per the convention the effort's arch spec records; `c4c2c16` is
the working branch `ticket/serve-tcp-service`, seven `wip:` commits. The merge
was clean with no conflicts, and the landing tree is byte-identical to the
verified `c4c2c16` (`git diff c4c2c16 HEAD` is empty), so every file landed
exactly as verified. `ticket/serve-tcp-service` was deleted after the merge.

What landed, three files: `crates/api-contracts/src/transport.rs` (+65/-21),
`crates/daemon/tests/auth.rs` (+285) and `docs/testing.md` (+9/-1).
`serve_tcp_service(addr, service)` binds and delegates, the way `serve_tcp`
does, and `serve_tcp_listener_service(listener, service)` holds the one TCP
accept loop, wrapping the mounted `ConnectRpcService<Router>` in one `Arc` and
handing every accepted stream to the private `serve_connection` body that
`serve_stream` and `serve_uds_service` already share. `serve_tcp_listener` now
delegates to the service-taking twin, so the loop count stays at one; its
signature, error handling and `eprintln` text are unchanged, and
`ConnectRpcService::clone` is shallow (`Arc<D>`), so the per-connection
semantics match the previous per-connection `from_arc`. The four new tests mount
`AuthPolicy::TokenRequired` over a real TCP socket and assert the typed
missing-credential refusal with the handler proven not to run, a served paired
token, the `Pair` exemption, and Unix/TCP parity from one cloned service value.
The one landed-code change is `serve_tcp_listener`'s body; the arch spec's "no
second accept loop" and "keep their signatures and behaviour" hold together
only in this shape, and criterion 3 read literally as "not touched" would
require a duplicated loop instead.

The full gate set was run on the verified tree `c4c2c16` and is green: `cargo
fmt --check`, `cargo clippy --all-targets -- -D warnings`, the workspace `cargo
test` at 144 passed, 0 failed, 7 ignored, `buf lint`, `buf breaking`, both drift
checks and the ABI harness at `[result] PASS (73 checks)`, all exit 0. Because
the landing tree is identical, the only gate re-run on it after the merge was
`cargo test -p daemon --test auth`: 14 passed, 0 failed. The deviation report is
committed alongside this note at
`docs/tasks/metis-foundations/tickets/serve-tcp-service/deviation-reports/serve-tcp-service.md`.

`docs/tasks/state.yaml` carries the orchestration task pointer and is left
uncommitted and out of this landing commit, as in the earlier landings; the
ticket's `workflow_state` and the archive stay with `finalize-task`.

Residual risks carried forward: the four TCP tests are Unix-gated through the
shared `#![cfg(unix)]` fixture file in which they live; the bind test reserves a
loopback port by bind-and-release, so it is racy under a very busy host; and two
parallel front-end families now exist over one connection body, with the parity
test as the guard, covering the unary path only because the contract declares no
streaming RPC yet.

Knowledge harvested at finalize: the effort map's constraint bullet now records
that both transports mount a `ConnectRpcService` and enforce the credential
policy, with the `serve-tcp-service` follow-up closed. `docs/testing.md` already
carried the service-taking TCP front-ends and the parity assertion from the
implementation, so it needed no further change. The arch spec's landed section
records the measured surface, including that the accept loop clones the service
rather than the router and that `serve_tcp_service` cannot report a
kernel-assigned address.
