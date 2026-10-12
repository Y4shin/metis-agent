---
type: task
subtype: prototype
title: Does a plausible authoring trait map cleanly onto the primitive ABI?
status: stable
workflow_state: done
blocked_by: []
---

## Question

Does a throwaway Rust authoring trait, roughly what `plugin-contract` will
ship, translate into the primitive ABI through a thin wrapper without dragging
trait concepts across the boundary? Specifically:

1. Can a trait with a few capability methods and a cancellation method be
   dispatched through one generic `invoke` addressed by capability name?
2. How much code does the translation take, and does it stay thin enough to
   claim the ABI is decoupled from the trait?
3. Does the barebones async path work: `invoke` returns pending, the plugin
   completes through the host callback, and the consumer's awaiting task
   resolves? The inline synchronous return is deliberately out of scope here,
   since the mechanics spike covers it.
4. Where does the translation leak? Any place the trait has to bend toward the
   ABI is a finding.

## Alternatives worth comparing

- One generic `invoke` with dispatch by capability name (the chosen design)
  against a per-method function table entry, to show what the churny
  alternative actually costs.
- Hand-written adapters against macro-generated ones, to see whether a macro is
  justified at this size.

## Smallest artifact that answers it

A throwaway Rust project whose source lives in `spikes/plugin-authoring-trait/`
inside this repository, kept for reference and committed, but never added to the
root workspace's members list and never wired into CI. It holds a throwaway
trait plus hand-written translation to the primitives, and two consumer crates:
one plugin implementing the trait, and one plugin consumer that loads it and
drives a request through the host adapter. The async path is exercised end to
end, always pending, never inline.

## Who reacts

The user reviews the trait shape and the translation code. The spec and the
`plugin-contract` authoring interface consume the findings.

## What it unblocks

The authoring trait design in `plugin-contract`, the `metis-plugin-sdk`
re-export, and the async executor questions that `metis-plugin-sdk` still owns.
An inconclusive result does not block them: record what was learned, abandon the
spike, and keep the provisional decisions in `plugin-abi-layout`.

## Completion evidence

Findings recorded in `findings.md` beside this task: the trait that was tried,
the translation size, where the trait bent toward the ABI, and the commands that
reproduce it. The spike source stays in `spikes/plugin-authoring-trait/` as
reference.
