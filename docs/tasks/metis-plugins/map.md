---
type: map
title: Metis plugins
status: stable
blocked_by: [metis-foundations]
---

# Metis plugins

## Destination

The host side of the plugin system works. It consumes the manifest and the
versioned C ABI that `metis-foundations` delivers in `plugin-contract`: one
exported entry point returning a function table, explicit instance creation
and destruction, capability discovery and initialisation, serialised request
and response payloads, explicit buffer ownership and release, and host
callbacks for completion, progress, and plugin-originated events.
`plugin-runtime` extracts bundles, validates manifests, selects the platform
and architecture library, checks ABI
and manifest compatibility, keeps a registry, delivers per-plugin
configuration and host context, discovers capabilities, dispatches requests,
receives events and completion callbacks, and manages request lifecycle,
cancellation, shutdown, and shared-library lifetime. A sample plugin loads
end to end.

## Constraints

- Rust trait objects, futures, strings, vectors, Tokio handles, and channels
  never cross the C ABI boundary.
- The Rust trait is internal to each compiled plugin. Export helpers and
  adapters translate an implementation into the C ABI.
- The runtime never reads the daemon's configuration files; the daemon
  resolves configuration and secrets and supplies each plugin its own
  configuration plus an explicit host context.
- Native plugins run in the daemon process and can crash it. The C ABI is not
  a sandbox; the contract does not pretend otherwise.
- Bundle format version, ABI version, and plugin version are distinct.
- The library stays loaded until all instances, tasks, callbacks, and
  plugin-allocated buffers are finished. Hot unloading waits until the
  lifetime guarantees exist; disabling a capability does not require
  immediately unloading its library.
- Keep dependencies small. The runtime needs only the underlying contract.

## Decisions so far

- From `docs/rough-plan.md`, "plugin-contract": manifest types, versioned C
  ABI, Rust-facing traits and adapters, and request, response,
  initialisation, tool and provider capability, and event payload contracts.
  Workflow integration uses the tool contract; plugins do not export workflow
  steps.
- From "plugin-runtime": bundle extraction, manifest validation, library
  selection and loading, compatibility checks, registry, initialisation,
  configuration delivery, capability discovery, dispatch, events and
  callbacks, request lifecycle, cancellation, shutdown, and lifetime.
- From "Native plugin design": the conceptual bundle layout, the proposed ABI
  direction, and the distinction between bundle, ABI, and plugin versions.
- From "Async boundary": do not pass futures across the ABI. The
  start/completion protocol registers a request ID and completion channel
  before calling start; the plugin schedules work, returns promptly, and
  invokes a host completion callback. Pending state is registered first
  because completion may precede the return from start. Cancellation is an
  explicit request whose acknowledgement and race semantics are defined.
- `plugin-contract` is owned by `metis-foundations`: it designs the manifest,
  the versioned C ABI, and the generated bindings. This effort consumes it and
  owns the host side.
- Plugin-owned executors hidden behind the Rust SDK are the initial preferred
  approach.

## Fog

- Bundle format, file extension, supported target names, and single-target
  versus multi-target packaging.
- Manifest schema and its representation.
- Serialised payload choice, initially considering JSON bytes.
- Exact ABI struct layout, ownership rules, concurrency rules, and error
  semantics are owned by `metis-foundations` in `plugin-contract`. This effort
  owns how the host enforces them while loading and dispatching.
- Compatibility policy between bundle, ABI, and plugin versions.
- The async executor model and how the SDK hides it.
- Completion-before-start handling and cancellation acknowledgement and
  completion races.
- Panic containment inside exported adapters, callback thread-safety, and
  shutdown ordering.
- How a sample plugin is built and loaded in CI.

## Out of scope

- The network contract, generated types, client, and the realtime design are
  owned by `metis-foundations`. It proves the transports end to end with one
  dummy request and one dummy event. The real endpoints are implemented in
  `metis-daemon`, not here.
- The plugin authoring SDK, the core plugin set, and the plugin CLI, which
  belong to `metis-plugin-sdk`.
- Workflow step implementations. Plugins provide tools, not step types.
- Workflow step executors and the agent loop, which belong to
  `metis-agent-core`.
- Daemon configuration resolution and secret distribution beyond the explicit
  host context handed to the runtime.
