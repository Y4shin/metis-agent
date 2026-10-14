---
type: findings
title: Do the two structural ABI hazards stay contained, or do they need a guarantee?
status: stable
---

# ABI hazard fixtures

Question: the ABI has two structural hazards that the current host counts but
nothing provokes, a wake that arrives after the plugin's future was dropped and
a drop that arrives after the library was unloaded. Do the existing counters,
the ordering ledger, and the `Arc`-based waker survive a hostile plugin that
provokes both, or does the host need a stronger guarantee than it has?

Short answer: hazard one is contained by the `Arc` chain and the counters record
it, but the ledger does not. Hazard two is not contained by anything the host
has: a copied future handle dropped after unload faults the process with
`SIGSEGV`, and no counter or ledger line exists because the process is gone.
The runtime's own API already prevents hazard two, because `Instance::invoke`
ties every future to the instance `Arc` and `unload` refuses while any future is
alive; the prototype reaches the fault only by calling the raw table, which the
ABI lets a caller do. The stronger-guarantee alternative named in the task
(refuse unload while a waker clone is outstanding) is aimed at hazard one, which
is already contained, and does not stop hazard two.

## Alternatives compared

- **Counters only, as today.** Recording a wake after a drop is enough because
  the record is the evidence.
- **A hostile fixture, host unchanged.** Provoke both hazards against the real
  runtime and measure what happens.
- **A stronger guarantee.** Refuse `unload` while any waker clone is
  outstanding, and refuse a poll whose future was dropped, so the hazards cannot
  occur instead of being recorded.

## Prototype location

Throwaway workspace, not a root workspace member, no root file touched:
`.work/prototype-abi-hazards/` (gitignored by the existing `/.work/` rule). It
was deleted after the run, per the prototype rule, so the commands below record
how it was built and run rather than pointing at live code.

- `hostile-plugin/`: a hand-rolled cdylib against the committed ABI (like the C
  plugin). It clone-stores a waker in plugin state and wakes it later, and it
  hands out a future handle a caller can copy.
- `probe/` (`src/main.rs`): hazard one through the real runtime API, plus the
  outstanding-clone unload check; then it spawns the hazard-two child.
- `probe/src/bin/hazard2.rs`: hazard two through the raw table, in its own
  process, so the probe reports the signal and survives.

The probe path-depends on `crates/plugin-contract` and `crates/plugin-runtime`
and rebuilds the cdylib itself before loading it, the same stale-library rule the
ABI harness encodes.

Environment: rustc 1.101.0-nightly, cargo 1.101.0-nightly, Linux x86_64, all
commands through `devenv shell`.

Exact commands:

```sh
cd /home/patric/Projects/metis-agent
export CARGO_TARGET_DIR="$PWD/.work/prototype-abi-hazards/target"
devenv shell -- cargo build \
  --manifest-path .work/prototype-abi-hazards/Cargo.toml --workspace --bins
devenv shell -- "$CARGO_TARGET_DIR/debug/probe"
```

`probe` rebuilds `hostile-plugin` through `cargo build --lib` and then loads
`$CARGO_TARGET_DIR/debug/libhostile_plugin.so`; `hazard2` runs as a child of
`probe`.

## Raw observations

`Host::waker_counts()` is `(cloned, released, wakes_after_drop)`.

Hazard one, variant A (plugin clones the waker, stores the clone, leaks the base
the way the shipped SDK does):

```
[h1/A] first poll: Pending
[h1/A] waker_counts after the first poll (cloned, released, wakes_after_drop): (1, 0, 0)
[h1/A] the PluginCall was dropped
[h1/A] waker_counts after the drop (cloned, released, wakes_after_drop): (1, 0, 0)
[h1/A] the wake call returned: Ok([])
[h1/A] final waker_counts (cloned, released, wakes_after_drop): (1, 1, 1)
[h1/A] ledger: call 1 started on hazard.orphan_pending
[h1/A] ledger: call 1 dropped, context reclaimed (drop returned ok)
[h1/A] ledger: call 2 started on hazard.wake_orphan
[h1/A] ledger: call 2 dropped, context reclaimed (drop returned ok)
```

The stored clone is woken after the call was dropped, the process survives, and
`wakes_after_drop` becomes `1`. The clone's wake releases exactly one owning
reference (`released` goes `0` to `1`), which is why the late wake is safe: the
clone held an `Arc<CallState>`, so `waker_wake`'s `Arc::from_raw` is a live
reference, not a dangling one. `wakes_after_drop` is the only record; the ledger
has a `dropped` line and no late-wake line, and the runtime has no `request_id`
on the waker to attribute the late wake to a call.

Hazard one, variant B (same, but the plugin also releases the borrowed base
waker the host passes to `poll`):

```
[h1/B] waker_counts after the first poll (cloned, released, wakes_after_drop): (1, 1, 0)
[h1/B] final waker_counts (cloned, released, wakes_after_drop): (1, 2, 1)
```

This shows the counters conflate two different releases. `cloned_wakers` counts
only explicit `clone` calls, but `released_wakers` counts every `drop`/`wake`
that consumes an owning reference, including the borrowed base reference the
host creates in `host_waker()`. So `clones == releases` is not an invariant once
a plugin handles the base waker correctly, and the base reference is real and
releasable.

Outstanding-clone unload check, the stronger-guarantee alternative:

```
[h1/unload] before unload, counts = (1, 0, 0) (cloned, released, wakes_after_drop): 1 clone outstanding
[h1/unload] instance.unload() returned: Ok(())
[h1/unload] after unload, counts = (1, 0, 0)
[h1/unload] ledger: call 9 started on hazard.orphan_pending
[h1/unload] ledger: call 9 dropped, context reclaimed (drop returned ok)
[h1/unload] ledger: instance destroyed
```

`unload` returns `Ok` with one waker clone still outstanding. The host keeps no
outstanding-clone state, so refusing unload on one would cost new state, exactly
as the task anticipated. It is also not needed for safety: the waker's vtable
lives in `plugin-runtime` (host code), not in the plugin, so a wake after unload
never enters plugin code.

Hazard two (raw-table invoke, handle copied, library unloaded, then the copy's
`drop` called), run as a child process:

```
[h2] create_instance: Ok
[h2] invoke_future: Ok
[h2] copied future: ctx=0x55c9dac34fe0 vtable=0x7ded687f8768 drop=0x7ded687bc8b0
[h2] drop address mapped before unload: 7ded687bb000-7ded687f8000 r-xp .../target/debug/libhostile_plugin.so
[h2] destroy_instance: Ok
[h2] the Loaded value was dropped (the library is unloaded)
[h2] drop address is UNMAPPED after unload: calling it should fault
[h2] calling the copied future drop now
[h2] child exit code: None
[h2] child signal:    Some(11)
[h2] child succeeded: false (a fault is the expected outcome)
```

(`ctx`, `vtable`, and `drop` addresses differ per run; the shape is stable.)
`/proc/self/maps` shows the plugin's executable segment before unload and no
mapping for the address after, so `dlclose` really unmapped it. The child dies
with signal 11 (`SIGSEGV`), exit code `None`; the parent probe exits 0.

## What each alternative shows

- **Counters only.** Enough for hazard one and not enough for hazard two. The
  counter records the wake after the drop (`wakes_after_drop = 1`), and the
  `Arc` chain is what keeps the wake from being a dangling dereference, so
  `docs/plugin-contract.md` section 11 rule 4 is confirmed by measurement. But
  the ledger itself carries no late-wake line, and the counters cannot attribute
  a late wake to a request or tell a base-waker release from a clone release. For
  hazard two the counters record nothing: the process faults before any line is
  written. "The record is the evidence" holds for one hazard and fails for the
  other.
- **Hostile fixture, host unchanged.** The measurement above. Hazard one is
  contained with a record; hazard two is a hard fault, and the only reason the
  probe survives is that it subprocesses the fault. The runtime's guard is real
  but narrower than the ABI: `Instance::invoke` inserts the request id into
  `in_flight` and the returned `PluginCall` holds an `Arc<Instance>`, so
  `unload` refuses both while a future it created is alive. A handle copied out
  of the raw table is not in `in_flight`, holds no instance `Arc`, and is not
  visible to either guard.
- **A stronger guarantee.** The named version (count outstanding waker clones
  and refuse unload) addresses hazard one, which is already safe, and costs live
  state the host does not keep: measured, `unload` returns `Ok` with a clone
  outstanding, and the ledger's only clone evidence is counts. It does nothing
  for hazard two. The poll-after-drop half is already structural: the host is
  the only poller, and `PluginCall::poll` already refuses once `dropped` is set.

## Recommended direction

Keep the counters and the `Arc` chain as the guarantee for hazard one, and land
the hostile fixture as a record rather than hardening the waker path. The
containment is not incidental: the clone is an owning `Arc<CallState>`, the
waker vtable is host code, and a late wake only flips `wakes_after_drop`. No new
state is needed, and the stronger-guarantee alternative would buy nothing for
this hazard.

For hazard two, the guarantee the design needs is a lifecycle rule, not a
counter: the plugin table, and any `MetisFutureHandle` copied out of it, is dead
after `destroy_instance`; the daemon must never call the table, or drop a copied
handle, once the library is unloaded. That rule already exists in spirit
(`docs/plugin-contract.md` section 5: "Only valid after `shutdown`"; section 7:
"`drop` is called by the host exactly once"; the ABI type comment "a destroyed
handle is not used again"), and the runtime's `unload` enforces it for every
handle the runtime itself created. The residual is the raw-table path, which the
host cannot detect cheaply because `MetisFutureHandle` is deliberately a copyable
two-field C struct. Do not add an outstanding-clone counter to `unload`: it does
not address this hazard, and the measured cost is real.

If the operator wants the raw path to stop being able to fault, the cheaper
options are contract- and API-level, not counting: state the after-unload rule
as a hard precondition with a named undefined behavior, and keep the runtime's
`unload` guards as the enforced path. A process boundary for untrusted plugins
is the only real containment and is already out of scope (the map says the C ABI
is not a sandbox).

## Consequences

- **`teardown-and-unload`.** Its unload rule does not need to count outstanding
  waker clones. The "drop every future and waker clone" step in contract rule 7
  stays, but the host cannot verify waker clones by count (the counters conflate
  base and clone releases, and the base reference is not counted at all). Keep
  the existing `in_flight` plus last-`Arc` guards, and add the explicit rule that
  the table is dead after `destroy_instance`, including handles copied out of it.
  The task's "library unloads when its last instance is destroyed and no
  quarantined poll is outstanding" starting answer stands; the change is the
  added table-dead rule, not a clone counter.
- **The ticket that lands the hazard fixtures.** Two halves. Hazard one belongs
  in the ABI harness or the runtime tests as a record test: a capability that
  clone-stores the waker, a settled `Pending`, a dropped call, then a second
  capability that wakes the stored clone; assert `wakes_after_drop == 1`, that
  `clones` and `releases` still balance under the SDK's leak-the-base convention,
  and that the process is alive. Hazard two cannot be a check inside the harness
  process, because the fault is the point: run it as a subprocess from the
  harness and assert the child dies by signal, and document the fault as defined
  undefined behavior for a handle used after unload. The existing
  `registry.rs` test that asserts `unload` refuses while a future is in flight
  stays; the new test covers the raw path that bypasses it.
- **`docs/plugin-contract.md` section 11 rule 4.** Confirmed by measurement: a
  wake after a drop is recorded (`wakes_after_drop` becomes non-zero) and never a
  dangling dereference. Worth the same wording change as above: the record is a
  counter, not a ledger line, and it cannot name the call.

## Newly discovered work

- **The SDK leaks one `Arc<CallState>` per poll.** `HostInner::host_waker()`
  creates an owning reference, and the SDK's `future_poll_body` wraps a `clone`
  of it in a `std::task::Waker` and drops that clone, but never releases the
  borrowed waker passed to `poll` (the `MetisWaker` value has no `Drop` and is
  dropped as a no-op). `PluginCall::drop` does not release it either, despite the
  comment in `host_waker()`. Variant B proves the base reference is real and
  releasable (releasing it moved `released` from `0` to `1` at first poll); the
  SDK chooses not to. The result is one leaked `Arc<CallState>` (and its
  `Arc<HostInner>`) per poll, masked by the fact that `clones == releases` still
  holds for the SDK's own clone. This needs its own ticket and a
  `Drop`-instrumented confirmation.
- **The counters cannot attribute a late wake.** `CallState` carries no
  `request_id`, so a `wakes_after_drop` increment cannot name the call whose
  future was dropped. If the ledger is meant to be the evidence, the late wake
  should append a ledger line with an id, which needs state the host does not
  keep today.
- **A double `wake` on one consumed clone is an over-release.** `waker_wake`
  does `Arc::from_raw` and then checks `dropped`; a plugin that calls `wake`
  twice on the same owning clone (or `wake` after `wake_by_ref` plus `wake`)
  over-releases the `Arc` before any counter check, which is host-side UB. The
  counters would show unbalanced releases, but too late. Not measured here; worth
  a note or a defensive check if hostile plugins are in scope.
