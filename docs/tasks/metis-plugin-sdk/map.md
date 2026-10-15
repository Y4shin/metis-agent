---
type: map
title: Metis plugin SDK
status: stable
blocked_by: [metis-plugins]
---

# Metis plugin SDK

## Destination

Authoring a Metis plugin is practical. A Rust plugin authoring SDK sits on top
of `plugin-contract`: it re-exports that crate's authoring trait and export
adapters, and adds the serialisation and async scheduling helpers a plugin
needs, kept out of the runtime's dependency footprint. A set
of core plugins ships with Metis, exercising the SDK rather than reaching past
it. A plugin CLI scaffolds a new plugin, builds it for the supported
platforms, packages it into a bundle, and installs it into the daemon. Other
languages can still implement the C ABI directly; the SDK is the Rust path,
not a requirement.

## Constraints

- The SDK wraps the C ABI. It does not change it, and the runtime does not
  depend on it.
- SDK functionality is feature-gated or extracted into its own crate so that
  export helpers, serialisation, or async scheduling do not weigh down the
  runtime.
- Keep dependencies small.
- Core plugins use only the SDK and the host context they are given. They do
  not read daemon configuration or reach around the plugin interface.
- The CLI produces the bundle formats that `metis-plugins` accepts; it does
  not invent a second format.

## Decisions so far

- From `docs/rough-plan.md`, "plugin-contract": extract an optional
  `plugin-sdk` crate if export helpers, serialisation, or async scheduling
  bring substantial dependencies; the runtime should only need the
  underlying contract.
- From "Native plugin design": Rust authors use the SDK's trait and export
  adapters, while other languages implement the C ABI directly.
- From "Async boundary", **superseded by `docs/plugin-contract.md`:** the daemon
  is the only executor. The SDK owns the waker bridge from the host waker to
  `std::task` and the boxed-future helper that wraps an author's future, but no
  executor, and the plugin holds no runtime.
- The authoring trait and the export adapters live in `plugin-contract` behind a
  feature gate. This effort re-exports them and owns the waker bridge, the
  boxed-future helper, the typed facades and schema derivation, serialisation
  helpers, core plugins, and the CLI.
- The authoring trait this effort re-exports changed underneath it, and no
  `metis-plugins` ticket owns the migration. `abi-initialisation-entry` (working
  tip `c6666ff`, merge `9c9fd2a`) replaced instance creation with asynchronous
  initialisation: `Plugin` now requires `fn init(host: Host, initialisation:
  Initialisation) -> impl Future<Output = Result<Self, PluginError>>`, `Default`
  is gone, `started` is gone, and `stopped` stays. `Initialisation` and
  `HostContext` are public in `plugin_contract::authoring` behind the
  `authoring` feature, the initialisation body is the `{"config",
  "host_context"}` object `docs/plugin-contract.md` section 18 fixes, and the
  table's slot 0 is `init_plugin` at ABI minor 2 with the major and
  `metis_plugin_entry_v1` unchanged. The old shape is not reachable behind a
  flag, so the SDK's re-export, its waker bridge and boxed-future helper, its
  typed facades and the CLI's scaffold are all written against the new trait,
  and the fixture's `FixtureConfig` is the landed pattern for a plugin declaring
  its own configuration.
- The bootstrap deliberately left `plugin-sdk` uncreated; this effort creates
  it when the boundary proves useful.

## Fog

- Error taxonomy, partly implemented: `CallError`, `RouteError` and
  `RegistryError` are exported from `plugin-runtime` today, before the open item
  that would decide the taxonomy. The author-facing error surface this effort
  owns may keep that shape or replace it with its own, but replacing it needs the
  operator's agreement first. The capability descriptor this effort re-exports is
  the shared `plugin-contract::capability::CapabilityDescriptor`, ungated, which
  a plugin publishes and a host parses; the host's own duplicate type was deleted
  and the two sides cannot drift, so re-export it rather than defining a third
  shape.
- Which core plugins ship first, and what they demonstrate.
- CLI responsibilities: scaffold, build, package, install, publish, and how
  much cross-compilation the build step covers.
- Answered by `docs/plugin-contract.md`: no executor is exposed. The SDK bridges
  the host waker and boxes the author's future, and authors see the erased `call`
  surface plus schema-derived typed facades.
- Serialisation helpers and their dependency footprint.
- The plugin version, ABI version, and bundle version policy as seen by
  authors.
- How the SDK and CLI are tested against the runtime without a full daemon.
- Whether the SDK is a crate inside this repository, a separately published
  crate, or both.
- Further plugin-adjacent followups that this effort surfaces are charted in
  later Wayfinder sessions rather than guessed here.

## Out of scope

- The network contract, generated types, client, and the realtime design are
  owned by `metis-foundations`. It proves the transports end to end with one
  dummy request and one dummy event. The real endpoints are implemented in
  `metis-daemon`, not here.
- The host side of the plugin system: the ABI definition itself, manifest
  validation, bundle extraction, library loading, the registry, dispatch, and
  lifetime management, all of which belong to `metis-plugins`.
- The workflow crate and the agent harness.
- The daemon composition layer and its configuration resolution.
