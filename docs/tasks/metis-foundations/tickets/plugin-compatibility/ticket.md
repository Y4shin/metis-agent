---
type: ticket
subtype: feature
title: Plugin version tolerance across compiled versions
status: stable
workflow_state: ready
blocked_by: [plugin-abi-core, plugin-contract-v1]
size: s
---

## What to build

The promise that a plugin built earlier still works on a newer daemon within the
current ABI major, and that a newer plugin's unsupported capability is refused on
its own rather than taking the whole plugin down.

This ticket used to own the tolerance for a shorter table. That criterion moved
out: `plugin-contract-v1` replaces the v1 table with a clean v2, so there is no
longer an older table to tolerate across the boundary. What remains here is
tolerance within the v2 table and the per-capability and payload promises.

## Acceptance criteria

- [ ] A host and a plugin that agree on `abi_major` but differ in a table entry
      added after this ticket lands interoperate, with the reader touching only
      the entries its own `size` covers, asserted by a test.
- [ ] The host refuses a single capability whose descriptor version it does not
      support while the rest of the plugin stays usable, asserted by a test.
- [ ] Payloads carrying unknown fields deserialise successfully, covered by a
      test.
- [ ] A plugin built against the retired v1 table, or any other ABI major, fails
      at symbol resolution before any table is interpreted, with a typed error.

## Blocked by

- `plugin-abi-core`, because version tolerance is a property of the table and the
  descriptors.
- `plugin-contract-v1`, because tolerance has to be asserted against the v2 table
  that ticket lands, not against the v1 table it deletes.
