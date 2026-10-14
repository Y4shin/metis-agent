---
type: ticket
subtype: feature
title: One asynchronous load entry that initialises and registers a plugin
status: stable
workflow_state: ready
blocked_by: [manifest-and-library]
size: l
---

## What to build

One asynchronous entry turns a bundle path into a registered, serving plugin. It
composes verification, extraction, manifest validation, library selection and the
ABI check; it delivers the configuration and the host context to the plugin's
initialisation entry and awaits that future to completion; it cross-checks the
manifest's capability list against what the library reports; and it registers the
instance's capabilities so a call dispatches through the registry and returns a
response. The caller's runtime drives the future and the runtime crate starts
nothing.

## Acceptance criteria

- [ ] One call loads a bundle and returns a registered instance with its
      capabilities, or a typed refusal that names the first thing that was wrong.
- [ ] The plugin receives the configuration and the host context before it is
      registered, and a setup failure in either of the two classes is
      distinguishable by the caller.
- [ ] A plugin whose declared capability list disagrees with its library is
      refused, and the refusal says which capability differed.
- [ ] An instance type needs no `Default`, and the plugin-side constructor is
      asynchronous.
- [ ] A capability from a bundled fixture serves a call through the registry and
      returns its response.
- [ ] The runtime crate starts no thread and no runtime of its own.

## Blocked by

- `manifest-and-library`
