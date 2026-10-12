---
type: task
subtype: prototype
title: Do the plugin ABI primitives actually work when compiled and loaded?
status: stable
workflow_state: ready
blocked_by: []
---

## Question

Do the decisions in `plugin-abi-layout` (D1 to D13) survive contact with a
compiler and a loader? Specifically:

1. Does a `cdylib` plugin exporting `metis_plugin_entry_v1` resolve from a host
   through `libloading`, and does an append-only function table let a host read
   a shorter table safely?
2. Does the one-byte payload header survive a round trip, including the
   reserved-bit rule?
3. Do both completion shapes work: an inline `ok` with a result, and a pending
   return followed by a host `complete` callback?
4. Does every buffer cross the boundary exactly once and get released by its
   allocator, with no host-side free of plugin memory?
5. Does `catch_unwind` in the exported adapter turn a panic into an
   internal-error status instead of an unwind?
6. Does the `cbindgen` header agree with the Rust layout, as proven by a C
   translation unit that static-asserts sizes and offsets?

## Alternatives worth comparing

- The chosen design against a handwritten header with the same structs, to see
  how much manual fixing the generated header needs.
- Inline completion against callback-only completion for the same trivial call,
  to see whether the inline path is worth its branch.

## Smallest artifact that answers it

A throwaway Rust project whose source lives in `spikes/plugin-abi-mechanics/`
inside this repository, kept for reference and committed, but never added to the
root workspace's members list and never wired into CI. It holds two crates: a
`cdylib` plugin and a host binary. The host loads the plugin, creates an
instance, lists capabilities, invokes one synchronously and one asynchronously,
cancels one, releases every buffer, triggers a panic, and finishes by running
the C-side layout assertions. No product code, no integration with
`crates/plugin-contract` yet.

## Who reacts

The user reviews the findings. The `plugin-contract` implementation and the
spec consume them.

## What it unblocks

The `plugin-contract` implementation tickets and the ABI portions of the spec.
An inconclusive result does not block them: record what was learned, abandon the
spike, and keep the provisional decisions in `plugin-abi-layout`.

## Completion evidence

Findings recorded in `findings.md` beside this task: what compiled, what failed,
which of D1 to D13 needed revision, and the exact commands that reproduce the
result. The spike source stays in `spikes/plugin-abi-mechanics/` as reference.
