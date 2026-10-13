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
out: `plugin-contract-v1` replaces the pre-release v1 table with a revised v1
shape, so there is no
longer an older table to tolerate across the boundary. What remains here is
tolerance within the revised v1 table and the per-capability and payload promises.

## Acceptance criteria

- [ ] A host and a plugin that agree on `abi_major` but differ in a table entry
      added after this ticket lands interoperate, with the reader touching only
      the entries its own `size` covers, asserted by a test.
- [ ] The host refuses a single capability whose descriptor version it does not
      support while the rest of the plugin stays usable, asserted by a test.
- [ ] Payloads carrying unknown fields deserialise successfully, covered by a
      test.
- [ ] A plugin that exports only another ABI major's entry symbol fails at
      symbol resolution, before any table is read, with a typed error rather
      than a raw loader error.
- [ ] A plugin built against the retired pre-release v1 table exports the same
      `metis_plugin_entry_v1` symbol, so it is refused by the version gate with
      `MinorMismatch` before any table entry is read: additive tolerance is
      `found >= known`, which leaves minor 0 refused. Asserted by a test.

## Blocked by

- `plugin-abi-core`, because version tolerance is a property of the table and the
  descriptors.
- `plugin-contract-v1`, because tolerance has to be asserted against the revised
  v1 table that ticket lands, not against the pre-release table it replaces.
