# Lane reclaim

YOUR COPY: `/tmp/metis-plugin-registry-round-3/reclaim`

## Question (one)

How does the host reclaim an abandoned blocking poll, and what is the unload
rule?

## Context

Open items O1 and O2 of the handoff. Today the per-poll watchdog abandons a
future that blocks inside `poll`, and the host leaks the plugin `ctx` box and one
stored waker clone by design, because dropping a future while a poll may still be
running inside plugin code would free memory in use. Unload waits about 900 ms
for the blocking thread to leave plugin code and then gives up; a real host
cannot wait forever, and unloading a library whose code is executing on another
thread crashes the process.

## Do this

1. **Reclaim.** Design and implement a per-poll completion signal, or a
   quarantine list, so that when a late poll finally returns, the host reclaims
   the `ctx` box and the waker clone exactly once and only then. Prove it with
   observable markers on the existing harness: the watchdog fires, the poll
   returns late, the host reclaims, and a counter check shows zero leaked `ctx`
   boxes and zero leaked waker clones at teardown. A double reclaim, a reclaim
   while a poll is still in progress, or a use-after-free must be a recorded
   violation, never UB. Show the before and after leak counters in one run.
2. **Unload rule.** Choose exactly one host rule and demonstrate it in code:
   (a) refuse to unload and keep the mapping while a thread is still inside
   plugin code, (b) quarantine the plugin, or (c) treat a blocking poll as fatal.
   Argue it against the other two in a short paragraph each. Then show the rule:
   the blocking thread is still inside plugin code, the host refuses to unload,
   the library mapping stays alive, the plugin is quarantined so no new call is
   routed to it, and the process is still alive afterwards. The old fixed 900 ms
   wait must not be the answer.
3. State what the reclaim costs in added host-side state, because this result has
   to survive the north star of a smaller surface: if reclaim needs a field per
   future or per poll, say so and count it.

## Measurable outcome

A reclaim proven once-only against a late poll return with leak counters at
zero, plus a demonstrated unload rule with the process alive. If reclaim cannot
be made safe, that is the finding: say so explicitly, recommend the quarantine
path, and name the exact evidence that made you stop.
