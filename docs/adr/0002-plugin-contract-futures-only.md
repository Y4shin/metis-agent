# 0002. The plugin contract is futures-only, routed, and quarantined rather than waited on

- Status: accepted
- Date: 2026-10-13

## Context

The first version of the plugin boundary handed a host a table with one generic
`invoke` and a `complete` callback. A plugin that could not answer inside the
call returned `Pending`, spawned a worker, and called the host back when the
result was ready. On top of that sat a four-arm Rust authoring trait: four
capability constants, four associated request types, and four methods, one per
arm, which put a ceiling of four capabilities on every plugin.

The `spikes/plugin-registry` prototype measured what that shape costs, and every
number below was re-run by the parent before the contract was approved:

- A release cdylib built on the callback path is 1,554,520 bytes; built on the
  host-polled future path it is 924,328.
- A ready-on-first-poll capability costs 947 to 1288 nanoseconds to answer, and
  the full host-polled path p50 is 6893 to 8165 nanoseconds.
- A naive launch-and-await bridge that hides the futures behind an
  asynchronous-looking API costs 1.7 to 2.2 microseconds per call, which is time
  a real executor would not spend.
- `async-ffi`'s future aborts the process with `SIGABRT` when a plugin future's
  destructor panics, even for a plugin that returned `Ok`, and no `catch_unwind`
  on the host side can contain it: the unwind crosses an `extern "C"` frame
  before the host ever sees it.
- Unloading on a fixed 900 millisecond wait is both too long for the common case
  and unsound for the slow one: it is a guess about when plugin code stopped
  running, in a boundary whose whole premise is that the host knows when plugin
  code runs.

The operator approved `docs/plugin-contract.md` on 2026-10-13 as a v1 on the
understanding that real use would surface what is wrong with it.
`plugin-contract-v1` is the implementation of that contract, and this record is
the contract-level decision behind it, so a later reader does not reopen it by
accident.

## Decision

**A future is the only call path.** The plugin table carries `invoke_future`,
which returns a plugin-owned context plus a poll/drop vtable, and the host polls
it on the host's own runtime. `invoke` and the host `complete` callback are
deleted rather than deprecated. A capability that must wait asks the host through
the new `service_request` callback, returns `Pending` from its poll, and is woken
when the host answers through `service_complete`. The plugin therefore needs no
runtime, no thread, and no executor: the daemon owns when plugin code runs, which
is also what makes unloading sound rather than timed.

**The authoring surface is one capability-name-routed method.** `Plugin` is
`capabilities`, `call`, `cancel`, `started`, `stopped`, where `call` receives
`Request { capability, body }` and returns `Response { body }`. The four arms,
their four capability-name constants, and their four associated request types are
deleted, which removes the four-capability ceiling. Per-capability typing is the
SDK's generated facade, not a trait constraint.

**No `async-ffi`.** The plugin side ships its own boxed-future helper: it wraps
the author's future, bridges the host's waker into `std::task::Waker`, and wraps
both `poll` and `drop` in `catch_unwind`. A panic in either becomes a terminal
internal error, and a panicking destructor cannot reach `process::abort`.

**Unload is quarantine plus one reclaim, with no fixed wait.** A poll the host
abandons quarantines the instance: it accepts no new future, and its library
stays resident. The reclaim is the caller's, on the poll return: when the
abandoned poll finally returns, the caller drops the future, which returns the
plugin's context box and every waker clone it holds. Nothing is unloaded while a
future is alive, and the ordering is recorded rather than inferred.

**The revision is clean, not an append.** The entry order changed, so
`abi_minor` becomes 1 while the entry symbol and the ABI major stay 1, and a
reader refuses a minor it does not know. That refuses a stale pre-release plugin
instead of reading its entries through the new order. Additive change inside the
major stays append-only by `size` from here.

**The envelope and the presence rule ship in the header.** The payload header
constants and the per-entry slot constants are in the generated C header, so a
non-Rust implementer does not have to guess either. The two vtable `size` fields
that were written and never read are deleted, with the residual recorded: a
plugin whose vtable is the wrong shape is that plugin's own bug, and it is not
detectable at load.

## Consequences

- A plugin is simpler than a host: it implements one trait, hands out futures,
  and answers. The measured release cdylib shrinks by about 40 percent against
  the callback path, and the plugin path no longer pulls in an executor.
- The host owns the poll loop, so the daemon is the only place that decides when
  plugin code runs. That is the precondition for unloading without a timer, and
  it is why a plugin thread is a bug rather than a design choice.
- Panic containment moves to the two frames that can actually do it: the plugin's
  poll and drop shims, and every host callback on the host side. A callback is an
  `extern "C"` frame reached from plugin code, so only that frame can catch a
  panic raised inside it; this is an obligation on every host, and it is stated
  in the crate docs of the callback table.
- The registry must publish schemas and validate in both directions, because the
  call is erased: the host has no type to check a body against, so the plugin's
  published request schema is the contract for a request, and the plugin's own
  response schema is the contract for a response. A request that violates its
  schema is the caller's error; a response that violates the plugin's own schema
  is the plugin's error and does not quarantine it.
- Cost of the change: everything written against the superseded shape is
  rewritten, including the fixture, the harness, and the authoring tests. The
  table has no compatibility shim, which is the point of landing the revision
  before the first release: no plugin exists in the wild to stay compatible with.
- Residual, recorded rather than hidden: a wrong-shape vtable is undetectable at
  load; a plugin that misses its cancel deadline is a protocol violation the host
  records and drops, not a condition it can prevent; and the reclaim path is
  proven by the harness and the runtime's tests rather than by a daemon reaper,
  which stays daemon work.

## Alternatives considered

- **Keep `invoke` and the `complete` callback.** Rejected: it is a second
  protocol for the same event, it forces a plugin-owned worker, and it is the
  half that the 40 percent release-cdylib reduction measures.
- **Keep the four-arm authoring trait.** Rejected: the four capability constants
  and the four associated request types are the ceiling, and a plugin with five
  capabilities cannot be written at all. A capability name plus a body is the
  same information with no limit.
- **Use `async-ffi` for the plugin-side future bridge.** Rejected on the measured
  destructor panic: it aborts the process for a plugin that behaved correctly,
  and no host-side containment can change that.
- **Unload on a fixed wait.** Rejected: it is a guess. Quarantine plus one
  caller-driven reclaim makes the condition observable instead of estimated.
- **Append the future seam to the old table instead of revising it.** Rejected:
  the extra entry order would have to be carried forever, the two call paths
  would both need draining, and the point of removing the callback is that there
  is one path.
