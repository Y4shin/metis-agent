---
type: ticket
subtype: feature
title: Validate the manifest and select this host's library
status: stable
workflow_state: ready
blocked_by: [abi-initialisation-entry, bundle-extraction]
size: m
---

## What to build

A verified, extracted bundle becomes a mapped library with a validated table. The
manifest is parsed and validated: identity, the plugin's own version, the ABI
major and the minimum minor it needs, the library path per target, the capability
list with its schemas, and the configuration schema. This host's target library is
selected and mapped, the entry symbol is resolved against the ABI major, and the
table is checked under the landed rules. Refusals are typed and distinguishable:
absent manifest, invalid manifest, no library for this target, missing entry
symbol, major mismatch, an older minor refused before any entry is read, and a
table too short for an entry this build needs.

## Acceptance criteria

- [ ] An absent manifest and an invalid manifest are different typed errors, and
      the invalid one names what is wrong.
- [ ] Target selection chooses this host's library and refuses a bundle that does
      not carry it, without touching another target's file.
- [ ] The ABI checks reuse the landed rules and their existing typed error, and a
      newer minor's table is read while stopping at this build's layout.
- [ ] A library mapped from a bundle yields a table the existing raw ABI checks
      accept.
- [ ] A hand-written manifest loads, so the format is not something only a
      generator can produce.
- [ ] The manifest's fields are documented in the contract as the format an author
      writes.

## Blocked by

- `abi-initialisation-entry`
- `bundle-extraction`
