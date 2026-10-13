---
type: ticket
subtype: feature
title: Pairing over the remote listener
status: stable
workflow_state: done
blocked_by:
- pairing-and-tokens
- serve-tcp-service
size: s
---

## What to build

Pairing from a device that has no access to the daemon's local socket, so that a
phone can set itself up over the network.

## Acceptance criteria

- [ ] The pairing call is reachable on the remote listener and gated by the same
      preshared key.
- [ ] A test pairs over the remote transport, then completes an authenticated RPC.
      The authenticated event stream is asserted by the ticket that owns the first
      streaming RPC, because the contract has none until `event-stream` lands.
- [ ] Removing the preshared key and restarting closes pairing on both
      transports, asserted by a test.

## Blocked by

- `pairing-and-tokens`, because there is no pairing call to expose remotely.
- `serve-tcp-service`, because the remote listener must mount the credential
  policy, and today only the Unix socket has a service-taking front-end.

The ticket previously listed `tls-and-client-trust` as a blocker on the grounds
that a remote pairing must not send the preshared key over plaintext. ADR 0004
removed that ticket: the daemon does not terminate TLS, a reverse proxy does, and
whether remote pairing should be gated further once a proxy is in front is part
of the deferred client-trust question. Until that is settled, pairing over the
remote listener carries the plaintext exposure the map records.

## Implementation notes

Landed on `task/remote-pairing` as the merge commit `adb4c5d`
("ticket(remote-pairing): Pairing over the remote listener"), parents `fd65ccc`
and `4651798`. The landing branch did not exist when this ticket was landed, so
it was created at `main`'s tip (`fd65ccc`), which is the previous ticket's
landing branch, per the convention the effort's arch spec records; `4651798` is
the working branch `ticket/remote-pairing`, four `wip:` commits. The merge was
clean with no conflicts, and the landing tree is byte-identical to the verified
`4651798` (`git diff 4651798 HEAD` is empty), so every source file landed exactly
as verified. `cargo test -p daemon --test auth` was re-run on the landed tree:
16 passed, 0 failed, which is the count the verify worker measured. The branch
`ticket/remote-pairing` was deleted after the merge, and the deviation report is
committed alongside this note at
`docs/tasks/metis-foundations/tickets/remote-pairing/deviation-reports/remote-pairing.md`.

The ticket delivers pairing over the remote listener. `crates/api-contracts`
carries the `metis-pair-error` header vocabulary as a non-exhaustive
`PairRefusal`, `crates/daemon` emits it on a pairing refusal and treats an absent
`METIS_PRESHARED_KEY` as `Ok(None)` rather than an error, and `crates/client`
maps the header to `ClientError::Pairing` so no client parses message text. The
two new integration tests in `crates/daemon/tests/auth.rs` pair over a real TCP
listener mounted with `AuthPolicy::TokenRequired` and prove that removing the key
and rebuilding both listeners closes pairing on the Unix socket and on TCP while
leaving already-paired surfaces served. `docs/tasks/state.yaml` is the
orchestration task pointer and is left uncommitted and out of this landing
commit, as in the earlier landings; the ticket's `workflow_state` and the archive
stay with `finalize-task`.

Residual risks carried forward unchanged: the plaintext preshared key over the
remote transport, which ADR 0004 accepts and `reverse-proxy-mode` owns; the new
TCP coverage sitting in the `#![cfg(unix)]`-gated `auth.rs` binary, so non-Unix
targets have none; the event-stream half of criterion 2, owned by
`event-stream`; closed pairing as a state and not a revocation; no rate limit or
lockout on repeated attempts; and the daemon binary still mounting no listener,
which remains the daemon effort's.

Knowledge harvested at finalize: the effort map gained the decision bullet for
remote pairing and a fog entry for the missing pairing rate limit and lockout,
which no ticket owns yet. `docs/dev-env.md` and `docs/testing.md` already carried
the optional-key rule and the remote run commands from the implementation, so
they needed no further change.
