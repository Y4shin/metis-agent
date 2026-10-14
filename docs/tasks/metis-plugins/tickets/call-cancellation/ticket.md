---
type: ticket
subtype: feature
title: Call cancellation, its unknown ids and its drop rule
status: stable
workflow_state: ready
blocked_by: [async-initialisation]
size: m
---

## What to build

Cancellation as it was decided. The caller owns the deadline and the drop, so the
runtime holds no timer and no default deadline. Cancelling a live call marks the
plugin's token, and the plugin resolves the call as cancelled. Cancelling a
request id the host does not hold answers the frozen not-found status and never
calls the plugin. An unacknowledged cancel is recorded on the ordering ledger and
the instance keeps serving. The instance-wide form is a host-side loop over the
in-flight set. Dropping a call releases every buffer the host still holds for it
and cancels its outstanding service requests, so no completion is ever delivered
into a released context.

## Acceptance criteria

- [ ] A cancellable fixture call resolves as cancelled and reaches its caller as
      the frozen cancelled status.
- [ ] Cancelling an unknown or already finished id answers not-found without the
      plugin being called, and a second cancel of a live id is accepted.
- [ ] A call dropped before the plugin acknowledged is recorded on the ledger, the
      instance is not quarantined, and the next call to it succeeds.
- [ ] Dropping a call releases its buffers and cancels its service requests, and a
      completion offered afterwards for that id is refused rather than delivered.
- [ ] The instance-wide loop cancels every call on one instance and reports what
      it did.
- [ ] The runtime exposes no way to wait for a cancellation on its own; the caller
      drives both the deadline and the drop.

## Blocked by

- `async-initialisation`
