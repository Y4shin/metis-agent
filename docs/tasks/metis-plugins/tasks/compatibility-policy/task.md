---
type: task
subtype: grilling
title: Compatibility policy across bundle, ABI, plugin version, and signature
status: stable
workflow_state: ready
blocked_by: [plugin-signing]
mode: human
---

## Decision to settle

What the host refuses and what it tolerates when a bundle's format version, the
ABI the plugin needs, the plugin's own version, and the bundle's signature
disagree with the host:

- the bundle format version rule: refuse any version other than the one this
  build knows, or accept a newer one additively as the ABI does;
- how the manifest states the ABI it needs, and how that combines with the
  landed rule that the major must match and a newer minor is accepted while an
  older one is refused;
- what the plugin version means: informational for humans, a semver contract
  the host gates on, or a gate only inside a plugin family;
- the order of the checks, so a failure names the first thing that is wrong
  rather than a later consequence;
- what ends up in the manifest's format version compared with the ABI minor,
  so the two are not confused by an author;
- what an untrusted or unsigned bundle does relative to a version mismatch:
  one refusal path or two, and which error a caller sees;
- whether a loaded instance keeps working after the registry learns that a
  newer plugin version exists.

## Parent decisions

Already settled and not to be reopened:

- The signature scheme and trust policy, from `plugin-signing`.
- The ABI version rule is landed: the major is part of the entry symbol and
  must match, and a newer minor inside the major is tolerated by the
  size-prefixed table while an older minor is refused before any entry is
  read.
- Capability gating is per capability: a capability whose descriptor version
  the host does not support is refused on its own while the rest of the plugin
  keeps working.
- The manifest is required, lives at the bundle root, and declares the
  capability list, which is cross-checked on load.
- The bundle format version is distinct from the ABI version and from the
  plugin version, and all three appear in the manifest in some form.

## Choices already known

- The host already has the typed version error `MetisAbiError`, with
  `Unreadable`, `MajorMismatch`, `MinorMismatch`, and `EntryMissing`, and
  `plugin-runtime` inherits it as `CallError::Version`. A bundle format
  version check is a new error, not a reuse of that one.
- The ABI check happens when the table is read, which is after extraction and
  after the library is mapped. A bundle format check and a signature check can
  both happen strictly earlier.
- `plugin-runtime`'s checks today stop at the table and the capability
  descriptor. A manifest check is the new earliest gate.
- The plugin version has no consumer yet, so gating on it would create the
  first one.

## Recommended starting answer

Refuse any bundle format version other than the one this build knows, because
the container has no append-only size rule to make an additive change safe. The
manifest states the ABI major it needs and the minor it requires, and the host
refuses when its major differs or its known minor is lower, which is the
landed ABI rule read from the plugin's side. The plugin version is
informational and never gates a load. The check order is signature, then
bundle format version, then manifest shape, then capability cross-check, then
library and ABI, then the registry's per-capability gate, so the first true
failure is the one reported.

## Dependent-task implications

- The manifest and bundle tickets carry the checks in that order, each with its
  own typed error.
- `configuration-and-host-context` decides where the host's supported
  capability versions live, since that is the other half of the per-capability
  gate.
- `metis-plugin-sdk` inherits the rules for what its CLI writes and what it
  refuses to package.

## Remaining fog

- Whether the bundle format version is a plain integer or a major and minor
  pair, once the manifest's field set is written.
- Whether a plugin version range can be expressed in the manifest for a future
  host that wants to gate on it.

## Completion evidence

The policy recorded as decisions, with the check order explicit, the
alternatives considered, and the reason each was rejected.
