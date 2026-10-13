---
type: ticket
subtype: feature
title: Event stream with snapshot and identifier
status: stable
workflow_state: ready
blocked_by: [local-transport-rpc-skeleton]
size: m
---

## What to build

The realtime path: a server-streaming events RPC, the daemon-wide event
identifier ring, the snapshot that opens every stream, and the client's stream
drain, so that a surface receives a snapshot and then events without hand-rolling
framing or resume.

The event's payload carries structured content, and its wire type is chosen during
implementation: JSON text in a `string` is the accepted fallback, and a more
native encoding is welcome if it does not pull a schema or well-known-type
dependency into generation for little gain. The choice is recorded, because
`buf breaking` runs in FILE mode and changing the type later is expensive.

## Acceptance criteria

- [ ] The document declares a server-streaming events RPC whose request selects
      one or more channels and carries a cursor field.
- [ ] The service keeps a bounded in-memory ring and assigns a daemon-wide
      monotonic identifier to every event.
- [ ] A stream's first message is a snapshot for the selected channels, later
      identifiers increase, and heartbeats arrive as ordinary stream messages.
- [ ] The client exposes the stream and drains it, and the example prints the
      snapshot and one event.
- [ ] A test over a temporary socket asserts the snapshot, at least one event,
      and monotonically increasing identifiers.

## Blocked by

- `local-transport-rpc-skeleton`, because the stream needs the service and the
  local transport to run over.

## Implementation notes

Landed on `task/event-stream` as the merge commit `0727964`
("ticket(event-stream): Event stream with snapshot and identifier"), parents
`4235803` and `64bd7d5`. The landing branch did not exist when this ticket was
landed, so it was created at `main`'s tip (`4235803`, "docs(event-stream):
approve the arch spec"), following the convention recorded when
`remote-pairing` landed. `64bd7d5` is the working branch `ticket/event-stream`,
seven `wip:` commits. The merge was clean with no conflicts, and the landing
tree is byte-identical to the verified `64bd7d5` (`git diff 64bd7d5 HEAD` is
empty), so every source file landed exactly as verified. The branch
`ticket/event-stream` was deleted after the merge, and the deviation report is
committed alongside this note at
`docs/tasks/metis-foundations/tickets/event-stream/deviation-reports/event-stream.md`.

The TDD worker and the verifier both changed nothing: this landing carries the
prior attempt's seven commits and the verification results recorded against
`64bd7d5` (workspace suite 163 passed, `api-contracts` 42 passed, daemon stream
3 passed, client stream 2 passed, the `events` example `[result] PASS`, `buf
lint` green, `buf breaking` green). Because the landing tree equals the verified
tip byte for byte, those results hold for the landed tree without a re-run.

Residuals carried into the effort, not fixed here: the payload is `string
payload = 4` carrying JSON text (the operator-delegated fallback, and `buf
breaking` runs in FILE mode, so a later native encoding is expensive);
`client::SkeletonDaemon` changed shape from a unit struct to `new()`/`with_ring()`,
which is landed test-support surface and a coherence-pass candidate; heartbeats
are unconditional rather than idle-detected, the ring is per-process and lost on
restart, the slow-consumer policy is the library default, and `last_event_id`
only floor-suppresses live events where the placeholder `stream-resume-and-expiry`
replaces it; two `HEARTBEAT_DEFAULT` constants (`daemon::service` and
`client::skeleton`) can drift in value; the stream tests are timing dependent
(25 ms heartbeat against 5 s bounds) and showed no flake in three repeats; and
`crates/client/Cargo.toml` carries a cosmetic double blank line, which no gate
reads.
