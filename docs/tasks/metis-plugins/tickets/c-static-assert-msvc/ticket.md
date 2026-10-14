---
type: ticket
subtype: bug
title: Fix the C static assertion so the Windows target compiles
status: stable
workflow_state: ready
blocked_by: [abi-initialisation-entry]
size: s
---

## What to build

The contract's C translation unit compiles under MSVC's C mode, so the runtime
and the daemon can be built for the Windows target instead of only the API
contracts crate. The assertion's intent is preserved: a layout mismatch still
fails the build, in a form MSVC accepts. The Windows CI job widens to the daemon.

## Acceptance criteria

- [ ] The pinned MSVC invocation compiles the C translation unit with warnings
      denied.
- [ ] A deliberate layout mismatch still fails the build under both gcc and
      clang, so the fix does not trade the check for compilation.
- [ ] The Windows CI job compiles the daemon, not only the API contracts crate.
- [ ] The recorded testing gap is narrowed or closed in `docs/testing.md`.
- [ ] The bug file moves to the archive with its resolution recorded.

## Blocked by

- `abi-initialisation-entry`
