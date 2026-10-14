---
type: task
subtype: grilling
title: Cancellation acknowledgement, its deadline, and its races
status: stable
workflow_state: ready
blocked_by: []
mode: human
---

## Decision to settle

What cancelling a call means at the daemon's level, rather than for one call:

- who owns the deadline: the caller that starts the call, the registry, or the
  daemon's policy;
- what the host does when the plugin's future does not resolve as `Cancelled`
  before the deadline, given that the contract permits a drop and requires the
  protocol violation to be recorded;
- whether that record also quarantines the instance, which would stop every
  later call to it, or leaves it serving;
- what `cancel` returns for a request id that is unknown, already completed, or
  belongs to another instance;
- whether cancellation is offered per call, per instance, or both, and whether
  cancelling an instance cancels every call on it;
- what happens to a plugin buffer or a service the plugin already requested
  when the call is dropped, since the plugin's future is dropped and its
  context reclaimed;
- whether the host waits for the plugin to acknowledge at all, or treats the
  drop as the acknowledgement;
- what a cancelled call returns to its caller: the wire status `Cancelled`, a
  host-side error, or both.

## Parent decisions

Already settled and not to be reopened:

- Cancellation is an explicit request, and the plugin's future is expected to
  resolve as `Cancelled` on its next poll rather than be dropped mid-flight.
- A wake that arrives before or during the poll that armed it is legal and is
  covered by the waker liveness rule; a wake after the future was dropped is
  recorded as a violation and never a dangling dereference.
- Reclaim is the caller's, on the poll return, and an abandoned poll
  quarantines the instance.
- The status space is frozen, and `Cancelled` is one of its values.

## Choices already known

- `Instance::cancel` already marks the token and returns `CallError::Status`
  when the plugin refuses it, and `PluginCall`'s drop already reclaims the
  plugin's context and removes the request id from the in-flight set.
- The runtime's ordering ledger already records a call starting, a call
  dropping, and a quarantine, so a missed cancel can be recorded in the same
  ledger rather than in a new structure.
- The fixture has a `tool.cancellable` capability and a `tool.stubborn` one,
  which is the pair a deadline policy is tested against.
- The missed-cancel path was recorded as a residual by the previous effort and
  has no test driving it.

## Recommended starting answer

The caller that starts a call owns its deadline and passes it in, so no
daemon-wide timer policy lives in the runtime. On expiry the host drops the
future, records the missed cancel on the ledger, and quarantines the instance,
because a plugin that will not honour a cancel is the same class of problem as
one whose poll was abandoned. `cancel` for an unknown or completed request id
returns `NotFound` and changes nothing. Dropping a call cancels nothing on the
plugin's side: the plugin learns about cancellation only through `cancel`, and
a plugin that ignores the token keeps its own state until its future is
dropped. A cancelled call surfaces the frozen `Cancelled` status wrapped in the
host's error for the caller.

## Dependent-task implications

- Tickets for the deadline, the missed-cancel record, the quarantine decision,
  the `NotFound` cases, and a fixture-driven test that drives
  `tool.stubborn` past its deadline.
- `teardown-and-unload` has to state whether an in-flight call at shutdown is
  cancelled first or dropped.
- `metis-daemon` owns the per-call deadline policy and any user-visible cancel
  request.

## Remaining fog

- Whether a quarantine from a missed cancel can ever be lifted without a
  reload.
- Whether the host should retry `cancel` once before dropping.

## Completion evidence

The deadline owner, the missed-cancel consequence, the `cancel` result for
each request-id case, and the drop-versus-cancel rule, recorded as decisions
with the alternatives considered.
