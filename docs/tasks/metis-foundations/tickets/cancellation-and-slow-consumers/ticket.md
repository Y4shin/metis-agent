---
type: ticket
subtype: feature
title: Cancellation and slow-consumer handling
status: stable
workflow_state: ready
blocked_by: [event-stream]
size: m
---

## What to build

Two related guarantees on the stream: a client can stop work with an explicit
idempotent call, and a client that cannot keep up is disconnected rather than
silently missing events, with the client treating that disconnect as a resync.

## Acceptance criteria

- [ ] An explicit cancel call carrying the target request identifier is
      idempotent, returns a distinguishable result when the work already
      finished, and stops in-flight work.
- [ ] Dropping a stream cancels nothing by itself, asserted by a test that
      disconnects the client and observes the work continuing.
- [ ] A consumer that stops reading meets bounded buffering and then a plain
      stream drop, with no events skipped silently.
- [ ] The client treats a dropped stream as resync-required and never presents the
      gap as current state.
- [ ] A test drives a deliberately stalled reader and asserts the drop.

## Blocked by

- `event-stream`, because cancellation and backpressure are properties of the
  stream.
