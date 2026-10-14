---
type: ticket
subtype: feature
title: Start-up outcomes and the disabled placeholder
status: stable
workflow_state: ready
blocked_by: [async-initialisation]
size: m
---

## What to build

The daemon's start-up outcomes for a directory of bundles. A trust failure, an
absent or invalid manifest and a refused unload stop start-up. An extraction
refusal, a missing target library, an ABI mismatch, a manifest that disagrees with
the library, and an unknown container version disable that one plugin and leave
the daemon running. A disabled plugin appears as a placeholder with its reason: it
is built from the manifest when one was readable, and from the bundle's file name
with no capabilities when it was not. It is listable, it is excluded from the
capabilities an agent would see, and invoking it answers the existing disabled
route error with its reason.

## Acceptance criteria

- [ ] Each start-up outcome is produced by a test that names it, including the
      unknown container version, which is the only refused bundle that degrades
      without ever being parsed.
- [ ] A disabled plugin is listable with its name, its reason and its capabilities
      when the manifest was readable, and with the file's name and no capabilities
      when it was not.
- [ ] Disabled capabilities are excluded from what an agent would be offered, and
      invoking one answers the disabled route error carrying the reason.
- [ ] A plugin whose initialisation fails is unloaded and replaced by its
      placeholder, and a refused unload at that point stops start-up.
- [ ] One broken bundle among several leaves the others loaded and serving.
- [ ] The start-up policy is documented as the rule, not only implemented.

## Blocked by

- `async-initialisation`
