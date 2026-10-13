---
type: ticket
subtype: feature
title: Plugin version tolerance across compiled versions
status: stable
workflow_state: ready
blocked_by: [plugin-abi-core]
size: s
---

## What to build

The promise that a plugin built earlier still works on a newer daemon, and that a
newer plugin's unsupported capability is refused on its own rather than taking
the whole plugin down.

## Acceptance criteria

- [ ] A newer host loads a plugin whose table is shorter and reads only the
      prefix it knows.
- [ ] The host refuses a single capability whose descriptor version it does not
      support while the rest of the plugin stays usable, asserted by a test.
- [ ] Payloads carrying unknown fields deserialise successfully, covered by a
      test.
- [ ] A differing ABI major version fails at symbol resolution before any table
      is interpreted, with a typed error.

## Blocked by

- `plugin-abi-core`, because version tolerance is a property of the table and the
  descriptors.
