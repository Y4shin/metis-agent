---
type: ticket
subtype: feature
title: Plugin ABI primitives proven by a fixture plugin
status: stable
workflow_state: ready
blocked_by: []
size: m
---

## What to build

The plugin boundary itself: a small primitive ABI, a generated header for other
languages, and a fixture plugin that a host loads and drives, so that the plugin
host and SDK efforts have a real boundary to build on.

## Acceptance criteria

- [ ] A plugin shared library exports exactly one versioned entry symbol
      returning an append-only function table whose first field is its size, and
      the table covers instance lifecycle, capability listing, one generic
      invoke, cancel, buffer release, and shutdown.
- [ ] The host table carries a user data pointer and the complete, progress,
      emit-event, and log callbacks.
- [ ] The status space includes a pending member, and the payload envelope
      implements the one-byte format header: a codec this build does not know
      returns unsupported, while a bad format version or non-zero reserved bits
      return invalid-argument.
- [ ] Each side releases only what it allocated, including a payload the receiver
      decides to drop, and the host never frees plugin memory.
- [ ] A panic in an exported function and a panic in a host callback each become
      an internal error rather than unwinding across the boundary.
- [ ] A generated C header is committed, Rust asserts sizes, alignment, and
      offsets in pointer-size terms, and a C translation unit compiled in CI
      re-asserts the same numbers.
- [ ] A fixture plugin is loaded by a harness that prints a pass or fail line per
      check and exits non-zero on failure, and the fixture is built before the
      harness runs so a stale library is never loaded.

## Blocked by

- None (can start immediately).
