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
- From "Async boundary", **superseded by `docs/plugin-contract.md`:** futures do
  cross the ABI now. The plugin returns an opaque context plus a poll and drop
  vtable, the daemon polls it on its own runtime and awaits it, and the plugin
  keeps no executor and no thread. The old start and completion protocol
  (register a request id, return promptly, invoke a host `complete` callback) is
  deleted rather than kept alongside. Cancellation stays an explicit request with
  defined acknowledgement and race semantics.
- `plugin-contract` is owned by `metis-foundations`: it designs the manifest,
  the versioned C ABI, and the generated bindings. This effort consumes it and
  owns the host side.
- **Superseded by `docs/plugin-contract.md`:** there is no plugin-owned executor.
  The daemon is the only executor, the SDK owns a waker bridge and a boxed-future
  helper, and nothing on the plugin side spawns a worker. Bundle extraction,
  validation, loading, dispatch and the registry stay here.

## Fog

- Bundle format, file extension, supported target names, and single-target
  versus multi-target packaging.
- Manifest schema and its representation.
- Serialised payload choice, initially considering JSON bytes.
- Exact ABI struct layout, ownership rules, concurrency rules, and error
  semantics are owned by `metis-foundations` in `plugin-contract`. This effort
  owns how the host enforces them while loading and dispatching.
- Compatibility policy between bundle, ABI, and plugin versions.
- Answered by `docs/plugin-contract.md`: there is no executor model to hide,
  because the daemon polls and the plugin holds no runtime.
- Answered by `docs/plugin-contract.md`: there is no completion callback to race
  with start, so the question becomes a wake arriving before or during the poll
  that armed it, which the waker liveness rule covers. Cancellation
  acknowledgement and its races remain here.
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
