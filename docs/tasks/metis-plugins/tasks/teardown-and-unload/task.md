---
type: task
subtype: grilling
title: Teardown, disable, and unload ordering for instances that share a library
status: stable
workflow_state: done
blocked_by: []
mode: human
---

## Decision to settle

The daemon-level sequence that ends an instance's life and the conditions under
which a library leaves the process:

- the full teardown sequence for one instance: stop producers, join, drop every
  future and waker clone, `shutdown`, `destroy_instance`;
- who runs it, given that the runtime starts no thread and offers the steps
  rather than a loop;
- what teardown does when a plugin ignores `shutdown` or returns a non-`Ok`
  status, since the library may then never be safe to unload;
- the rule for a library with several instances: whether unload waits for the
  last instance, and what the ledger has to show before it happens;
- what an in-flight call at teardown does: get cancelled first, get dropped, or
  block teardown until the caller finishes;
- what disabling a capability or an instance means: refuse new calls with
  `PermissionDenied`, keep the library mapped, and leave in-flight calls alone;
- what re-enabling does, and whether a configuration change requires a reload;
- the shutdown case for a quarantined instance whose abandoned poll never
  returns, which is the one case where unload may never be safe;
- whether the daemon has a shutdown deadline after which it gives up and
  leaves the process rather than waiting.

## Parent decisions

Already settled and not to be reopened:

- The contract's teardown order is observable and the host proves it: stop
  producers, join, drop every future and waker clone, then unload.
- `Instance::shutdown` refuses while any future is in flight, and
  `Instance::unload` refuses unless every future is gone and the instance
  `Arc` is the last reference.
- The ledger records every refusal, so the ordering is evidence rather than
  inference from a timer.
- A disabled registry entry still resolves, so a caller can tell "turned off"
  from "never loaded".
- Hot unloading, meaning replacing a mapped library in place, is out of scope
  for this effort.

## Choices already known

- `Loaded` owns the library handle and drops it last, so unloading is the act
  of dropping the last instance that came from it.
- `unload` takes `self: Arc<Self>` and refuses when a call still holds the
  instance, which is exactly the check a multi-instance teardown path needs to
  respect.
- Quarantine keeps the instance resident by design, so a quarantined instance
  is the case where the daemon must decide between waiting and giving up.
- The runtime has no `Drop`-based teardown loop; every step is an explicit
  call, which is what lets the daemon decide the order.

## Recommended starting answer

One teardown function per instance that performs the fixed sequence and
returns the ledger entries it added, so the daemon can assert the order. An
in-flight call is cancelled first with the cancellation task's deadline, then
dropped. A library unloads when its last instance is destroyed and no
quarantined poll is outstanding; a quarantined instance is reported and its
library stays mapped. Disabling refuses new calls without touching in-flight
ones, and re-enabling needs no reload. The daemon's shutdown has a deadline,
and past it the daemon reports the instances it could not release and exits
without forcing a library out of the process.

## Dependent-task implications

- Tickets for the teardown sequence, the multi-instance unload rule, disable
  and re-enable, the quarantined-instance report, and the ledger assertions
  the runtime's tests and the harness check.
- `cancellation-policy` supplies what teardown does to an in-flight call.
- `configuration-and-host-context` decides whether a configuration change
  needs a reload, which this task assumes it does not.
- `metis-daemon` owns the process-level shutdown deadline.

## Remaining fog

- Whether the daemon ever forces a `dlclose` after its deadline, given that
  the process is exiting anyway.
- Whether an instance can be re-created from a library that is already mapped
  without unloading it.

## Decisions recorded

**T1, one teardown function, mirroring the load function.** It unregisters the
instance, cancels or drains in-flight calls to the caller's deadline, calls
`shutdown`, drops every future, destroys the instance, unloads the library, and
reports the ledger entries it added. The daemon does not drive the steps itself.

**T2, an in-flight call is cancelled first and dropped at the deadline.** The
plugin gets the chance the contract already gives it to resolve as cancelled, and
the drop is bounded so one stubborn plugin cannot hold teardown open.

**T3, plugins load at startup and are released at shutdown.** The operator's
model: a plugin is loaded and initialised at daemon startup and shut down and
deinitialised at daemon shutdown. Voluntarily deinitialising a live plugin
mid-operation is overcomplication and there is no such path. So the teardown
sequence serves the shutdown path, and the start-up failure path keeps its own
rule: destroy the instance, unload the library, and refuse to start if the unload
is refused.

**T4, an instance that cannot be released at shutdown is reported, not forced.**
A `shutdown` that returns a non-`Ok` status, a quarantined instance whose
abandoned poll never returns, and an unload refused for any other reason all mean
the same thing here: the library stays mapped, the fact is recorded, and the
operator is told. Forcing an unmap over code that may still be running is exactly
what quarantine exists to prevent.

**T5, disable and enable are registry state, not lifecycle.** A disabled entry
refuses new calls and leaves the instance, its in-flight calls and its library
alone; enabling accepts calls again. Neither tears anything down or unloads
anything, which matches the constraint already recorded, that disabling a
capability does not require unloading its library.

**T6, the daemon has a shutdown deadline.** It waits for clean teardown of every
instance and, on expiry, reports which instances could not be released and exits
anyway, leaving those libraries mapped because the process is ending.

## Completion record

**The decision in the operator's terms.** Plugins load and initialise at daemon
startup and are released at daemon shutdown, with no live teardown. Teardown is
one async function mirroring the async load function: unregister, cancel or drain
to the caller's deadline, `shutdown`, drop every future, destroy the instance,
unload the library, and report the ledger. An instance that cannot be released is
kept mapped and reported rather than forced, because the process is ending and
unmapping over possibly running code is what quarantine prevents. The daemon
bounds its own shutdown with a deadline and exits with a report of what it could
not release. Disable and enable are registry state with no lifecycle effect.

**Alternatives considered and rejected.**

- The daemon driving the teardown steps itself. Rejected: loading is one call
  that owns a sequence, so shutdown should be its counterpart.
- Dropping in-flight calls immediately. Rejected: the plugin is denied the
  cancelled resolution the contract gives it.
- Refusing to proceed until the caller has drained in-flight calls. Rejected: it
  hands every caller a state machine to get right.
- Forcing the unload when an instance cannot be released. Rejected: it unmaps a
  library that may still be running a poll.
- Treating a refused unload as fatal mid-shutdown. Rejected: at start-up the
  daemon's state is unknown before any work has happened, which is why that case
  aborts, but at shutdown the process is ending and an abort buys nothing.
- No shutdown deadline, so an unreleasable plugin keeps the daemon alive.
- Aborting at the deadline instead of reporting.
- Disable as a teardown, or as a cancel of in-flight calls. Rejected: it makes a
  routine toggle cost a full load and discards in-flight work.
- Live reconfiguration. Rejected by the same model: there is no live teardown to
  support it.

**Constraints and rationale.** The teardown order is the contract's rule 7 and it
is observable, so the runtime's ledger records each step and each refusal rather
than inferring them from a timer. A library is released only when nothing is
inside plugin code and no future or waker clone is outstanding, which is why
quarantine keeps a library resident by design and why the same condition cannot
be forced away at shutdown.

**Dependent-task implications.**

- `configuration-and-host-context` is refined, not contradicted: the daemon reads
  its configuration at startup, so a configuration change is a daemon restart,
  and the "new instance" that decision describes is the one the next startup
  creates.
- `metis-daemon` owns the shutdown deadline, its default, and the configuration
  that may change it, and it owns the report of instances that could not be
  released.
- `cancellation-policy` supplies what "drained to the deadline" means for a call.
- The tickets must cover the teardown function, the ledger assertions the runtime
  tests and the ABI harness check, the registry-only disable path, and the fact
  that a disabled placeholder has no instance to release at shutdown.

**Remaining fog, all spec level.**

- The exact ledger entry names the teardown function reports.
- The daemon's default shutdown deadline and whether it is configurable.

**Completion evidence.** The six recorded decisions, the rejected alternatives,
the constraints and rationale, the dependent-task implications and the remaining
fog, confirmed by the operator.
