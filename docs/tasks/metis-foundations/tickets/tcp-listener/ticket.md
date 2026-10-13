---
type: ticket
subtype: feature
title: Remote TCP listener, off by default
status: stable
workflow_state: done
blocked_by:
- local-transport-rpc-skeleton
size: s
---

## What to build

The optional remote listener: the same service reachable over TCP when the user
asks for it, with no default bind address, so that enabling remote access cannot
silently expose every interface.

## Acceptance criteria

- [ ] The service listens on TCP only when explicitly enabled in configuration.
- [ ] The bind address is required from configuration, with no default.
- [ ] With no remote configuration, no TCP socket is opened, asserted by a test.
- [ ] A test connects the client over TCP and completes the echo RPC. The
      realtime stream over TCP is asserted by the ticket that owns the first
      streaming RPC, because the contract has none until `event-stream` lands.

## Blocked by

- `local-transport-rpc-skeleton`, because the remote listener serves the same
  service and reuses the transport module.

## Implementation notes

Landed on `task/tcp-listener` as the merge commit `c2e8684`
("ticket(tcp-listener): Remote TCP listener, off by default"), parents `c63a56e`
(the landing branch, created at this ticket's arch spec commit, which is also
`main`'s tip and carries every earlier landing merge) and `1d93f42` (working
branch `ticket/tcp-listener`, four `wip:` commits, one per seam). The merge was
clean with no conflicts, `git diff 1d93f42 HEAD` is empty, so the landing tree is
the tree the verify worker gated, and `ticket/tcp-listener` was deleted after
the merge. The review artifact
`deviation-reports/tcp-listener.md` is committed with this note.

At landing, on `task/tcp-listener`, through `devenv shell --`:
`cargo test -p client --test remote` passes 3/3
(`no_remote_configuration_opens_no_tcp_socket`,
`the_listener_task_releases_its_port_when_it_is_dropped`,
`the_client_echoes_over_the_remote_listener`) and `cargo fmt --check` reports
`FMT-OK`. The verify worker's gates on the identical tree at `1d93f42`:
`cargo fmt --check` pass, `cargo clippy --all-targets -- -D warnings` pass, the
three remote tests pass three extra times with no flake, and the workspace
`cargo test` exits 0 with no failure in any target. The only clippy output is
the pre-existing `_FORTIFY_SOURCE` warning from the `plugin-contract` C
translation unit, which is not a Rust lint and does not affect the exit status.
The gates were not re-run in full here because the landing tree is byte
identical to the verified tree.

Divergences, from `deviation-reports/tcp-listener.md` and the implementation
report; all four are additive or prose only:

1. `transport::serve_tcp_listener(listener, router)` is public beside the arch
   spec's `serve_tcp(addr, router)`, which is now a bind plus a delegate to it.
   `serve_remote` must bind before returning, to report the port the kernel
   assigned for `127.0.0.1:0`, and a reserve-then-rebind scheme would race the
   client's first connect; there is still exactly one accept loop. The cost is
   that the exported front-end the daemon effort wires into `main` is the
   two-line wrapper no test drives directly.
2. `transport::remote_authority(addr) -> http::Uri` is public and is the one
   place the `http://<addr>` rule lives; `tcp_transport` and
   `Client::with_remote_addr` both call it, mirroring the landed
   `local_authority()`. Without it the connection authority and the request
   authority could drift.
3. `ConfigError` keeps the specified `InvalidRemoteAddr { variable, value,
   source }` and gains `RemoteBind { addr, source }`, because `serve_remote`
   returns `Result<_, ConfigError>` and a bind can fail. The enum stays
   `#[non_exhaustive]`.
4. The `api-contracts` module and crate docs no longer describe the module as
   "the local transport"; they name the TCP front-ends and the off-by-default
   rule. Comments only.

Unchanged, and confirmed by the six-file diff: `serve_stream`, `serve_uds`,
`uds_transport`, `local_authority`, `Client::{local, with_endpoint, echo}`, the
checked-in contract and its generated trees, the ABI layout, and every
`Cargo.toml`.

Residual risks, as the implementation and verification reports state them: the
listener is plaintext and unauthenticated until `tls-and-client-trust`,
`reverse-proxy-mode`, `local-identity-and-acl` and `pairing-and-tokens` land, so
an enabled listener serves the same surface as the local socket and the required
bind address is the only mitigation; `remote_authority` panics on an
unparseable authority, in practice never, matching `local_authority`, and the
IPv6 form is covered by the config unit test but not end to end; `serve_remote`
has no graceful shutdown, matching `serve_uds`; and
`no_remote_configuration_opens_no_tcp_socket`'s rebind probe is near-vacuous on
its own, with the `Ok(None)` assertion and the configuration unit tests carrying
criterion 3.

Knowledge harvested at finalize: the effort map gained a decision bullet for the
remote listener's rule and for the stream-over-TCP assertion now sitting with
`event-stream`; `docs/testing.md` already carried the remote run commands and
the configuration rule from the implementation, so it needed no further change.
The operator reviewed the unexercised `serve_tcp` at finalize and accepted it as
a documented gap, so the daemon effort adds its own coverage when it wires
`serve_tcp` into `main`.
