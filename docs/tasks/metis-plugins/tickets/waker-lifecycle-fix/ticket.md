---
type: ticket
subtype: bug
title: Fix the waker lifecycle defects the hazard prototype measured
status: stable
workflow_state: ready
blocked_by: []
size: m
---

## What to build

The authoring bridge's waker lifecycle is correct, so a plugin authored with it
can be polled, cancelled and dropped without leaking or over-releasing a call's
shared state. Three measured defects go: the value handed to the plugin's poll
leaks one reference per call because it is never released, the wake-after-drop
record cannot name the call it belongs to, and a second wake on one consumed
clone over-releases the call state. A wake after the future was dropped is
recorded as a counter rather than an ordering-ledger entry, and the contract says
so.

## Acceptance criteria

- [ ] A call whose future is dropped releases every reference it took, and the
      fixture's reference counts balance after a full call, a cancelled call and
      an abandoned call.
- [ ] A wake delivered after the future was dropped is recorded and names the
      request id it belongs to.
- [ ] A second wake on one consumed clone does not release the call state twice.
- [ ] The contract states that a wake after drop is a counted violation rather
      than an ordering-ledger entry.
- [ ] The fixture, the ABI harness and the workspace test suite pass.

## Blocked by

- None (can start immediately).
