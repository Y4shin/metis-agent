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
