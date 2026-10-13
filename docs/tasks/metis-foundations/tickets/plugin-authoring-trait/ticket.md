---
type: ticket
subtype: feature
title: Plugin authoring trait and adapters
status: stable
workflow_state: ready
blocked_by: [plugin-abi-core]
size: m
---

## What to build

The Rust authoring experience: a plugin author implements a trait and never sees
the C boundary, with adapters translating both directions, so that plugins are
ordinary async Rust while the ABI stays stable.

## Acceptance criteria

- [ ] The authoring trait and both adapters live in the plugin contract crate
      behind a feature gate, so the runtime depends on the primitives and payload
      types alone.
- [ ] The trait exposes a cancellation token on the per-request context, plus a
      poll convenience, and a call blocked on I/O observes cancellation
      promptly.
- [ ] The adapter always returns pending and completes through the host callback,
      while the ABI keeps its inline path for plugins that implement it directly.
- [ ] A lifecycle hook exists, and plugin-originated events have a route through
      the trait, asserted by a test where an event reaches the host.
- [ ] Capability descriptors and dispatch arms are checked against each other at
      instance creation, so a missing arm fails loudly instead of appearing as a
      not-found at call time.
- [ ] The error that suppresses a completion after a successful cancel is
      documented and covered by a test.
- [ ] The fixture plugin is rewritten through the trait, passes the same harness,
      and the trait module imports no ABI type.

## Blocked by

- `plugin-abi-core`, because the adapters translate onto the primitives.
