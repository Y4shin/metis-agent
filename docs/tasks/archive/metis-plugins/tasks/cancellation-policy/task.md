---
type: task
subtype: grilling
title: Cancellation acknowledgement, its deadline, and its races
status: deprecated
workflow_state: done
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

## Decisions recorded

**A1, the caller owns the deadline.** The runtime holds no timer, no default
deadline and no thread, because it starts neither a runtime nor a thread. The
caller awaits the call with its own timeout, calls `cancel`, and drops the future
when the plugin has not resolved. The daemon's policy lives in the daemon, and
the runtime's own test drives the sequence against `tool.stubborn`.

**A2, an unacknowledged cancel is recorded and does not quarantine.** This
revises the starting answer, which had it quarantine the instance. Dropping the
future runs the plugin's own drop path and reclaims its context, which is the
mechanism the contract provides for exactly this case, and the contract forbids a
plugin from keeping work running outside a call. Quarantine stays reserved for an
abandoned poll and an unreleasable teardown. The operator's fail-fast posture was
put to them and this is the deliberate exception: the host cannot distinguish a
plugin that ignores the token from one that is merely busy, so quarantining would
make one slow call fatal to a plugin under a tight caller deadline.

**A3, `cancel` for an id the host does not hold returns the frozen `NotFound`.**
The host checks its own in-flight set first and never calls the plugin, since the
set is authoritative for what is in flight and poking a plugin about a dead
request risks a plugin bug for no information. `CallError::Status(MetisStatus::NotFound)`
is the existing way to say it; `CallError` has no not-found variant of its own.
An id belonging to another instance cannot occur, because `cancel` is a method on
the instance and the set is per instance. A second `cancel` for an id still in
flight is accepted and marks the token again, and the runtime does not retry
`cancel` on its own.

**A4, cancellation is per call, and the instance-wide form is a host-side loop.**
The plugin table stays frozen. The loop runs over the instance's own in-flight
set, and `teardown-and-unload` uses it for cancelling or draining to the caller's
deadline.

**A5, a dropped call releases its buffers and cancels its outstanding service
requests.** Every plugin buffer the host still holds for that call is released
before the context is reclaimed, and a completion is never delivered for a call
that was dropped, because the context it would be written into has been released.

**A6, the acknowledgement is the status return, and the frozen status is the
surface.** `cancel` returning `Ok` acknowledges that the token is marked;
resolution as `Cancelled` is the confirmation; a call the plugin cancels surfaces
`CallError::Status(MetisStatus::Cancelled)`; and a call the caller dropped yields
nothing to that caller, which already knows it dropped. No new host error variant
is introduced for cancellation.

## Completion record

**The decision in the operator's terms.** The caller that starts a call owns the
deadline and the drop; the runtime owns no timer and no default. Cancelling marks
the plugin's token and the plugin is expected to resolve `Cancelled`; if it does
not before the caller's deadline, the caller drops the future, the runtime
records the unacknowledged cancel on the ledger, and the instance keeps serving,
because the drop path reclaimed the context and a slow call is not an unsafe one.
`cancel` for an id the host does not hold answers with the frozen `NotFound` and
the plugin is not called. Cancellation is per call, with an instance-wide loop on
the host side for teardown. A dropped call releases its buffers and cancels its
service requests, so nothing is ever completed into a released context. A call
the plugin cancels reaches its caller as the frozen `Cancelled` status.

**Alternatives considered and rejected.**

- A default deadline held by the runtime, or a daemon-wide timer policy the
  runtime enforces. Rejected: it is a policy the daemon cannot see, and the
  runtime cannot time anything without the thread it does not start.
- Quarantining the instance on an unacknowledged cancel. Rejected as the default,
  though it was the starting answer: it makes every slow call fatal.
- Disabling the instance until a reload on the same event. Rejected twice over:
  the same over-reaction plus a registry write for a protocol note.
- Forwarding every `cancel` to the plugin and letting it answer, including for
  ids the host does not hold.
- A new host-side not-found error rather than the frozen status.
- An instance-wide cancel as a new ABI entry, or no instance-wide form at all for
  teardown to use.
- Delivering a service completion for a call that was dropped.
- Waiting for the plugin to resolve `Cancelled` before the host may drop, and a
  new host error variant for cancellation.

**Constraints and rationale.** The runtime starts no runtime and no thread, so no
timing can live in it. `PluginCall`'s drop already reclaims the plugin's context
and removes the request id from the in-flight set, which is what makes a recorded
non-quarantining outcome coherent: after the drop there is no live call to be
unsafe about. The frozen status space already carries `Cancelled` and `NotFound`,
so the policy adds no vocabulary.

**Dependent-task implications.**

- `plugin-runtime` owns the in-flight pre-check in `cancel`, the ledger entry for
  an unacknowledged cancel, the instance-wide loop, and the buffer and service
  release at drop.
- `metis-daemon` owns the per-call deadline, any user-visible cancel request, and
  the decision to drop.
- `teardown-and-unload` consumes the instance-wide loop for its "cancel or drain
  to the caller's deadline" step.
- The tickets must drive `tool.cancellable` and `tool.stubborn` past a deadline,
  assert the ledger entry, and assert that a second cancel of a live id is
  accepted and that an unknown id answers `NotFound` without touching the plugin.

**Remaining fog, all spec level.**

- The daemon's deadline values and whether they are configurable, which belongs
  to `metis-daemon` rather than to this effort.

**Completion evidence.** The six recorded decisions, the rejected alternatives,
the constraints and rationale, the dependent-task implications and the remaining
fog, confirmed by the operator.
