---
type: task
subtype: grilling
title: Compatibility policy across bundle, ABI, plugin version, and signature
status: stable
workflow_state: done
blocked_by:
- plugin-signing
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

## Decisions recorded

**C1, the container format version is a single integer and must match exactly.**
Any other value is refused. The container has no append-only size rule to make an
additive change safe, because its regions are located by declared offsets and
lengths, so tolerating a newer version would mean interpreting bytes under a
layout this build does not know. A future container change ships with the daemon
that reads it.

**C2, that version lives in the header only, at a fixed offset after the magic.**
The magic and this field are the only parts of the header pinned across versions,
because the version determines the header's field set and therefore the signed
range, so the version is read before the signature is even located. The manifest
does not repeat it, since a second copy would only be a second place to disagree
about something already decided.

**C3, an unknown container version refuses that bundle and degrades it.** The
placeholder is identified by the bundle's file name, carries no capabilities, and
records that a newer daemon is needed; start-up continues. This is the one place
where an unreadable bundle is not fatal, and the operator accepted the asymmetry
with the fatal trust failure: a hostile file gains nothing by declaring an
unknown version, because the plugin never loads.

**C4, the manifest states two integers, `abi_major` and the minimum `abi_minor`.**
The host refuses when its major differs or its known minor is lower, which is the
landed ABI rule read from the plugin's side. No range syntax. This is the check
the entry symbol cannot perform: a plugin built against a newer minor whose
entries a lower host would read as an empty tail.

**C5, the plugin's own version gates nothing.** It is recorded, listed and shown
in messages, and an already loaded instance keeps working when a newer version
appears on disk, because nothing re-reads a loaded plugin short of a daemon
restart. Gating on it would invent the first consumer of the field; a semver
range in the manifest and a plugin-family gate stay open as future changes.

**C6, the check order and the outcome of each failure.** The order is the magic,
the container format version, the signature and chain, the manifest's shape, the
capability cross-check, the library for this host's target, the ABI, the
per-capability descriptor versions, and then registration. Each failure reports
itself and never a later consequence, so the extractor is not asked to explain a
file the callers above it should never have reached. The outcomes are the landed
start-up ones: a trust failure, an absent or invalid manifest, and a refused
unload abort; an extraction refusal, a missing target library, an ABI mismatch,
and a manifest that disagrees with the library degrade to the disabled
placeholder; and an unsupported capability descriptor refuses only that
capability. Staleness, ABI and trust are three distinct typed errors, and an
invocation that reaches a disabled entry still returns `RouteError::Disabled`
with its reason.

## Completion record

**The decision in the operator's terms.** The container format version is a
single number at a fixed offset after the magic, readable before anything else
because the signature's position depends on it, and it must match exactly; a
bundle whose version this build does not know is refused, degrades, and appears
as a placeholder named from its file with a reason that a newer daemon is needed.
The manifest states the ABI it needs as a major and a minimum minor, and the host
refuses a differing major or a lower known minor. The plugin's own version gates
nothing. The checks run in one order, magic first and registration last, and each
failure reports itself: trust, an absent or invalid manifest and a refused unload
abort, the bundle-level problems degrade, and an unsupported capability is
refused on its own.

**Alternatives considered and rejected.**

- Making the bundle format version a major and minor pair with a tolerated newer
  minor, as the ABI does. Rejected: there is no append-only size rule in the
  container, so a newer minor could move a field rather than add one.
- Repeating the format version in the manifest and cross-checking it. Rejected:
  the reader has already decided by then, so it is a second place to disagree.
- Aborting start-up on an unknown container version, which would close the
  asymmetry with the fatal trust failure but would make an older daemon refuse to
  start whenever a newer bundle is installed.
- Skipping an unreadable bundle with a log line and no registry entry, which
  hides it from the operator.
- Stating the ABI requirement as one version string with a range syntax.
  Rejected: it needs a range parser and no requirement justifies one.
- Gating on the plugin version, either by a host-level semver requirement or by a
  plugin-family rule.
- One error type for every version problem. Rejected: the operator has to tell a
  stale bundle from an incompatible one from a tampered one.
- Letting whichever check runs first report, with no fixed order. Rejected: the
  reported fault would be a consequence rather than the cause.

**Constraints and rationale.** The version must be readable before the signed
range is known, which is why the header pins the magic and the version field and
nothing else across versions. Every other rule here is the landed ABI and
start-up policy read from the bundle's side, so no new outcome was introduced
except the placeholder that has no manifest to be built from. The plugin version
stays informational because a field with no consumer gates nothing, and the
startup-only loading model means no loaded instance can be invalidated by a newer
bundle on disk.

**Dependent-task implications.**

- `metis-plugin-sdk` writes `abi_major` and the minimum `abi_minor` and never a
  range, and its CLI refuses to package a bundle whose declared ABI this build's
  manifest schema cannot express.
- `metis-daemon` owns discovery, so it supplies the file-derived identity for the
  placeholder of a bundle whose container cannot be read.
- `plugin-runtime` owns the new typed error for an unknown container version, the
  fixed check order, and a placeholder construction path that does not depend on
the manifest.
- The `plugin-contract` amendment records that the container header's version
  field is an exact-match value and where it sits relative to the magic.
- The tickets must cover the ordering assertions, the three typed errors, and the
  exact-match check, and the sample-plugin gate must not depend on a newer
  container version being tolerated.

**Remaining fog, all spec level.**

- The exact integer widths in the header and the exact manifest field names,
  which `to-spec` pins.
- Whether the placeholder's reason is a fixed enum or free text.

**Completion evidence.** The six recorded decisions, the rejected alternatives,
the constraints and rationale, the dependent-task implications and the remaining
fog, confirmed by the operator.
