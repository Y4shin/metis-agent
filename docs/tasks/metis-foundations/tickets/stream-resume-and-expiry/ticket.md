---
type: ticket
subtype: feature
title: Stream resume and expired cursors
status: stable
workflow_state: ready
blocked_by: [event-stream]
size: s
---

## What to build

Reconnection without loss: the cursor is honoured, missed events replay from the
ring, an expired cursor is reported distinctly, and the client answers it with a
resync instead of a retry loop or a silent gap.

## Acceptance criteria

- [ ] A stream opened with a cursor delivers only events after that identifier.
- [ ] An identifier older than the ring produces the distinct expired-cursor
      error, and the client turns it into a resync, meaning a fresh stream that
      begins with a snapshot.
- [ ] A test disconnects mid-stream and reconnects with the last seen identifier,
      asserting no gap and no duplicate.
- [ ] A test requests an expired cursor and asserts the resync path.
- [ ] A service restart expires every cursor, asserted by restarting the service
      in a test.

## Blocked by

- `event-stream`, because there is nothing to resume until a stream and its ring
  exist.
