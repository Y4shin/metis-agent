---
type: ticket
subtype: feature
title: Replace instance creation with asynchronous initialisation in the plugin table
status: stable
workflow_state: ready
blocked_by: [waker-lifecycle-fix]
size: l
---

## What to build

The plugin table carries asynchronous initialisation instead of instance
creation, and no counters entry. Initialisation receives one payload carrying the
configuration and the host context, and returns the instance plus a future that
must resolve successfully for the instance to be valid. The ABI minor is bumped
because the table changed, and the contract text, the generated C header, the slot
constants, the layout definitions, the C static assertions, the Rust fixture, the
C plugin and the ABI harness all move in this ticket, so the C ABI cannot drift
from its description. The new entry is proven by the ABI harness driving the C
plugin as a real host.

## Acceptance criteria

- [ ] The initialisation entry replaces instance creation and the counters entry
      is gone, with the minor bumped and the entry symbol carrying the unchanged
      major.
- [ ] The contract text, the generated header, the slot constants, the layout
      definitions and the C static assertions agree with the table, and the
      header regeneration check passes.
- [ ] Both the Rust fixture and the C plugin implement the new entry, including
      the failure case where the configuration does not match what the plugin
      declared.
- [ ] The ABI harness hand-drives initialisation through the new entry with a
      configuration and host context payload, observes an instance and a resolved
      future, and prints one line per check.
- [ ] The version tolerance checks still pass: a newer minor is read and stops at
      this build's layout, an older minor is refused before any entry is read, and
      another major is refused at symbol resolution.
- [ ] No counter of any kind is reachable through the table.

## Blocked by

- `waker-lifecycle-fix`
