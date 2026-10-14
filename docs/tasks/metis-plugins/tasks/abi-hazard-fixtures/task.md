---
type: task
subtype: prototype
title: Do the two structural ABI hazards stay contained, or do they need a guarantee?
status: stable
workflow_state: done
blocked_by: []
---

## Question

The ABI has two structural hazards that the current host counts but nothing
provokes: a wake that arrives after the plugin's future was dropped, and a drop
that arrives after the library was unloaded. Do the existing counters, the
ledger, and the `Arc`-based waker survive a hostile plugin that provokes both,
or does the host need a stronger guarantee than it has?

## Alternatives worth comparing

- **Counters only, as today.** `Host::waker_counts` reports cloned, released,
  and wakes-after-drop, and no fixture ever makes the third non-zero. The claim
  is that recording is enough because the record is the evidence.
- **A hostile fixture, host unchanged.** One capability that clones a waker,
  returns `Pending`, and wakes after the caller dropped the future; one that
  keeps a reference and drops it after `unload`. The host's behaviour is then
  measured rather than assumed.
- **A stronger guarantee.** Refuse `unload` while any waker clone is
  outstanding, and refuse a poll whose future was dropped, so the hazards
  cannot occur instead of being recorded. This costs state the host does not
  keep today, since the ledger tracks counts rather than outstanding clones.

## The smallest artifact that can answer it

A fixture capability or a second fixture cdylib that provokes both hazards
against the real `plugin-runtime`, plus a run that prints the resulting
counters and ledger. The prototype is throwaway: it exists to answer whether
the host's current evidence is enough, not to become the shipped fixture.

## Who must react to the result

The operator, on whether recorded evidence is the intended guarantee or whether
the stronger version is wanted, and on whether the hazard fixtures belong in
the shipped ABI harness rather than in a throwaway prototype.

## The decision or tickets it should unblock

- The ticket that lands the hazard fixtures in the ABI harness or the runtime's
  tests, if the prototype shows the current host survives with a record.
- A change to `teardown-and-unload`'s unload rule, if the stronger guarantee is
  wanted, because that rule would then have to count outstanding waker clones
  before it allows an unload.
- The claim in `docs/plugin-contract.md` section 11 rule 4 that a wake after a
  drop is a recorded violation and never a dangling dereference, which this
  prototype either confirms by measurement or refutes.

## Prior art and boundaries

- `spikes/plugin-registry/round-3/` and `spikes/plugin-registry/FINDINGS4.md`
  are the measured basis for the current waker design, and the two hazards are
  named there as counted but unprovoked.
- `crates/plugin-fixture` is the existing hostile fixture (`tool.panic`,
  `tool.panic_drop`, `tool.blocking`, `tool.stubborn`) and the pattern to copy.
- `crates/plugin-fixture/src/bin/abi-harness.rs` is the check harness, and its
  counters are the ones to read.
- `crates/plugin-runtime/src/lib.rs` holds the waker vtable, `CallState`, and
  the counters the prototype drives.
