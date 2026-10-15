---
type: bug
title: The plugin host's waker lifecycle leaks and mis-releases call state
status: fixed
fix_commit: dd7e1bf
---

# The plugin host's waker lifecycle leaks and mis-releases call state

## Symptom

Three defects in the host side of the plugin waker, found by provoking the ABI's
two structural hazards with a hostile fixture rather than by a user report. The
measurements are in
`docs/tasks/metis-plugins/tasks/abi-hazard-fixtures/findings.md` under "Newly
discovered work":

1. The value handed to a plugin's `poll` is one owning waker reference, and
   nothing released it. `HostInner::host_waker()` created it, the authoring
   bridge wrapped a clone in a `std::task::Waker` and dropped that clone, and
   `PluginCall::drop` did not release the original. One `Arc<CallState>`, and its
   `Arc<HostInner>`, leaked per poll. The leak was masked because the plugin's
   own clone and its release kept `clones == releases` true.
2. A `wakes_after_drop` increment could not name the call whose future was
   dropped, because `CallState` carried no request id. The counter recorded that
   a wake had arrived after a drop, and nothing about which call it belonged to.
3. A second `wake` on one consumed clone over-released the call state. The waker
   was a raw `Arc<CallState>` pointer, so `waker_wake` did `Arc::from_raw`
   before any check, and a plugin that woke one owning clone twice, or woke after
   `wake_by_ref` plus `wake`, released the state twice. That is host-side
   undefined behaviour, and the counters only showed unbalanced releases after
   the fact.

## Root cause

The host treated the waker reference it hands a poll as borrowed and expected the
plugin side to release it, while the ABI gives the plugin an owning reference
with no drop entry. Liveness was therefore the plugin's responsibility in a
contract that never asked the plugin to accept it. The same representation made
the waker's identity and its request id unavailable to the host: one raw pointer
carried the state and nothing else, so a late wake could not be attributed and a
second consume could not be detected.

## Fix summary

`waker-lifecycle-fix` landed the three fixes on the host and left the ABI
unchanged, so no minor bump was needed:

- `PluginCall::poll` releases the one owning waker reference it hands the poll as
  soon as the poll returns, whatever the poll returned, so a completed, a
  cancelled and an abandoned poll all balance. The release is on the poll return
  rather than on the drop, because a call is polled many times and dropped once.
- `CallState` carries the request id, and `Host::wakes_after_drop()` returns the
  request ids of late wakes in arrival order. The record stays a counter rather
  than an ordering-ledger line, and `docs/plugin-contract.md` section 8 and
  section 11 rule 4 now say so.
- A `MetisWaker` carries a host-side token in its `data` field instead of a raw
  `Arc<CallState>` pointer. The first `wake` or `drop` of a token removes its
  entry from a process-wide table, so a second use of one consumed clone finds
  nothing to release and is refused. `wake_by_ref` on a stale handle is safe for
  the same reason.

`Host::waker_counts()`'s first element changed meaning from "clones the plugin
made" to "every owning waker reference the host handed out", which is what makes
the leak observable at all; the tuple shape is unchanged. The fix landed as the
merge commit `dd7e1bf` on `task/waker-lifecycle-fix`, with the landing branch tip
at `3983c8d` when this record was closed, and the ticket is
`docs/tasks/metis-plugins/tickets/waker-lifecycle-fix/ticket.md`.

The one obligation the fix left standing is on the second host implementation:
`crates/plugin-fixture/src/bin/abi-harness.rs` still does not release the owning
waker reference its poll is given, and its `waker_wake` still prints an ordering
line for a dead slot. `abi-initialisation-entry` rewrites that harness, so the
obligation is recorded as carried work in the effort's arch spec.

## Impact

Before the fix: a daemon that polled a plugin leaked one `Arc<CallState>` and one
`Arc<HostInner>` per poll, so a long-lived daemon grew without bound on any plugin
traffic; a late wake was countable but not attributable, so the evidence could not
name the call; and a hostile or buggy plugin could provoke host-side undefined
behaviour by waking one owning clone twice. After the fix: the accounting
balances across a completed, a cancelled and an abandoned call, a late wake names
its request id, and a second consume of one clone is refused.

## Closing check

Run the waker suite and the two host implementations that must keep passing:

```
devenv shell -- cargo test -p plugin-runtime --test waker
devenv shell -- cargo test -p plugin-runtime --lib
devenv shell -- cargo run -p plugin-fixture --bin abi-harness
devenv shell -- cargo test
```
