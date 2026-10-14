---
type: map
title: Metis plugins
status: stable
blocked_by: []
---

# Metis plugins

## Destination

The host side of the plugin system works. It consumes the manifest and the
versioned C ABI that `metis-foundations` delivers in `plugin-contract`: one
exported entry point returning a function table, explicit instance creation
and destruction, capability discovery and initialisation, serialised request
and response payloads, explicit buffer ownership and release, and host
callbacks for completion, progress, and plugin-originated events.
`plugin-runtime` extracts bundles, verifies their signature against the trust
policy the daemon supplies, validates manifests, selects the platform and
architecture library, checks ABI and manifest compatibility, keeps a registry,
delivers per-plugin configuration and host context, discovers capabilities,
dispatches requests, receives events and completion callbacks, and manages
request lifecycle, cancellation, shutdown, and shared-library lifetime. A
sample plugin is packaged as a bundle, loads end to end, and is exercised in
CI.

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
- A bundle is signed, and the host verifies the signature against a trust
  policy the daemon supplies before it extracts or loads anything. The
  runtime never reads the trust policy from the daemon's configuration files.
- The library stays loaded until all instances, tasks, callbacks, and
  plugin-allocated buffers are finished. Hot unloading waits until the
  lifetime guarantees exist; disabling a capability does not require
  immediately unloading its library.
- Keep dependencies small. The runtime needs only the underlying contract,
  plus the archive, manifest, and signature readers the bundle requires.

## Decisions so far

- From `docs/rough-plan.md`, "plugin-contract": manifest types, versioned C
  ABI, Rust-facing traits and adapters, and request, response, initialisation,
  tool and provider capability, and event payload contracts. Workflow
  integration uses the tool contract; plugins do not export workflow steps.
- From "plugin-runtime": bundle extraction, manifest validation, library
  selection and loading, compatibility checks, registry, initialisation,
  configuration delivery, capability discovery, dispatch, events and
  callbacks, request lifecycle, cancellation, shutdown, and lifetime.
- From "Native plugin design": the conceptual bundle layout, the proposed ABI
  direction, and the distinction between bundle, ABI, and plugin versions.
- From "Async boundary", **superseded by `docs/plugin-contract.md`:** futures
  do cross the ABI now. The plugin returns an opaque context plus a poll and
  drop vtable, the daemon polls it on its own runtime and awaits it, and the
  plugin keeps no executor and no thread. The old start and completion
  protocol is deleted rather than kept alongside. Cancellation stays an
  explicit request with defined acknowledgement and race semantics.
- `plugin-contract` is owned by `metis-foundations`: it designs the manifest
  types, the versioned C ABI, and the generated bindings. This effort consumes
  it and owns the host side.
- The serialised payload is UTF-8 JSON behind the envelope's codec bit 0,
  per `docs/plugin-contract.md` section 9, and it is already implemented:
  `plugin-contract::payload` writes the JSON header, and `plugin-runtime`
  validates both directions against the published schemas. MessagePack is
  defined by the envelope and refused as `Unsupported`, so the payload
  question is closed rather than open.
- A bundle is a ZIP archive with the extension `.metis`. Its extraction
  refuses an entry whose name is absolute or carries `..`, any symlink or
  hardlink entry, entry permissions, and any entry over a size limit.
- A bundle carries one library per target it supports, and each target is
  keyed by operating system and architecture together, for example
  `linux-x86_64`. A bundle holds one or more targets at the producer's
  choice; the host selects its own target and refuses a bundle that does not
  carry it.
- The manifest is `manifest.toml` at the bundle root. It declares the plugin
  identity and version, the ABI major and minimum minor it needs, the library
  path per target, and the capability list with each capability's name, kind,
  version, and request and response schema. The bundle format version is not
  here: it lives in the container header, because it has to be readable before
  the archive can be parsed or the signature located.
- The capability list in the manifest is required and is generated from the
  same source as the plugin's own declaration, so the SDK's export macros and
  the SDK CLI write it at bundle time. A plugin built without the SDK
  hand-writes it, and the C fixture is the proof that the format is writable
  by hand.
- The host reads the manifest before it maps the library and refuses the
  instance when the declared capability list disagrees with what
  `list_capabilities` returns. One load path, not one for installed bundles
  and another for local loads.
- The host-side error surface stays as it is: `CallError` for load and
  dispatch, `RouteError` for routing and validation, and `RegistryError` for
  registration, layered over the frozen `MetisStatus` space. The three types
  partition by lifecycle stage, `RouteError::status` already maps to the wire,
  and replacing them would rewrite a landed and tested surface for no net
  gain. The SDK may flatten at its own boundary later.
- `plugin-runtime` starts no runtime and no thread, and reclaim of a plugin
  future is the caller's on the poll return. Quarantine plus the ordering
  ledger is the landed unload-safety mechanism.
- The plugin start-up policy is settled in `configuration-and-host-context`,
  with one later amendment from `plugin-signing`. A signature that is absent or
  does not verify is fatal and the daemon refuses to start, because it is a sign
  the machine was tampered with. The other bundle-level failures disable that
  plugin and leave the daemon running when the daemon's own state is known
  intact: an archive refused by the extraction rules, an ABI mismatch, a bundle
  with no library for this host's target, and a manifest that disagrees with the
  library. A failure that leaves the state unknown aborts startup: a plugin
  defect, a missing archive path, an absent or invalid manifest, and a refused
  unload of a degraded plugin. A plugin that blocks inside `init_plugin` hangs
  the daemon, which is accepted and documented because the ABI is not a sandbox.
- The `metis.*` control plane is unwound. Configuration becomes a lifecycle
  entry, `init_plugin`, on both the C ABI and the Rust trait, replacing
  `create_instance` and sitting before registration; the schema-fetch member is
  redundant because `list_capabilities` already returns schemas; and `counters`
  is removed, because the `plugin-counters-purpose` research traced the word to
  the round 3 spike's plugin-side metrics capability, found no `metis.counters`
  string and no consumer anywhere, and found both candidate needs already
  covered by the daemon's request-id policy and the host's own waker counts.
  This effort therefore amends the contract text, the plugin table, the
  generated header, the C static asserts, the fixture, the C plugin and the ABI
  harness, and it moves the `plugin-contract` table from "consumed only" to
  "amended here".
- A bundle is signed. It is one file with a fixed-size prefix header carrying
  the magic, the container format version, the suite, the offsets and lengths of
  the chain and the signature, and the ZIP length, followed by the chain, the
  payload signature, and the ZIP bytes. The chain is X.509 certificates over
  Ed25519 keys, and the leaf key signs the payload with raw Ed25519 over the
  header, the chain and the ZIP bytes. The daemon holds a root set and a
  revocation list, both compiled in with empty defaults and modified through the
  configuration file, and it recomputes every key id from the public key rather
  than trusting a carried one. A signature that is absent or does not verify, or
  an expired certificate, aborts start-up. The issuing tooling is standard, so
  the SDK CLI signs the payload and embeds a chain it was given or issued with a
  parent key.
- Plugins load and initialise at daemon start-up and are released at shutdown.
  There is no live teardown: one async function mirrors the load function and
  unregisters, cancels or drains in-flight calls to the caller's deadline, calls
  `shutdown`, drops every future, destroys the instance, unloads the library,
  and reports the ledger. An instance that cannot be released stays mapped and is
  reported rather than forced. Disable and enable are registry state only, so a
  configuration change is a daemon restart. The daemon bounds its own shutdown
  with a deadline.
- The compatibility policy reads the landed ABI rule from the bundle's side. The
  container format version is a single integer at a fixed offset after the magic
  and must match exactly, since the container has no append-only size rule; an
  unknown version refuses and degrades that bundle, with a placeholder named from
  its file. The manifest states `abi_major` and a minimum `abi_minor`, and no
  range syntax. The plugin's own version gates nothing. The checks run in one
  order, magic, container version, signature and chain, manifest shape,
  capability cross-check, target library, ABI, capability descriptor versions,
  registration, and each failure reports itself rather than a later consequence.

## Fog

- Trust roots and the revocation list: the daemon holds both, compiled in with
  empty defaults and modified through the configuration file. The file
  representation, the root set's rotation story and the revocation entry's shape
  are spec-level detail for `to-spec`.
- The exact manifest field names and types, the extraction limits (entry
  count, uncompressed size, permission bits), and whether a v1 bundle may
  carry assets. These are spec-level detail for `to-spec`, not decisions that
  block a ticket.
- Which artifact is the sample plugin: the existing `plugin-fixture`
  packaged as a bundle, or a new crate. The recommendation is the fixture,
  packaged by a script and loaded through `plugin-runtime` in an integration
  test, so the C plugin's hand-written manifest is exercised in the same
  gate. `to-tickets` settles it.
- `docs/bugs/plugin-contract-c-static-assert-msvc.md` is open and sits in
  this effort's territory: `plugin-contract`'s build script compiles a C
  translation unit whose `_Static_assert` message form MSVC rejects, so
  `plugin-runtime` cannot be compiled for the Windows target and the
  `windows` CI job is narrowed to `api-contracts`. It is a filed defect, not
  a decision, and the tickets must absorb it.
- Three defects the hazard prototype found in the host waker lifecycle, measured
  in `docs/tasks/metis-plugins/tasks/abi-hazard-fixtures/findings.md`: the
  authoring bridge leaks one `Arc<CallState>` per call, because the `MetisWaker`
  value handed to `poll` is never released and `PluginCall::drop` does not
  release it either; a `wakes_after_drop` increment cannot name the call whose
  future was dropped, because `CallState` carries no request id; and a second
  `wake` on one consumed clone over-releases the `Arc`. These are tickets, not
  decisions.
- The hazard prototype settles two things the tickets must respect. Hazard one,
  a wake after the future was dropped, is contained by the `Arc` chain and
  recorded as a counter rather than a ledger line, so `docs/plugin-contract.md`
  section 11 rule 4 needs its wording adjusted. Hazard two, a drop after unload,
  faults with `SIGSEGV` but is reachable only through the raw table, because
  `Instance::invoke` ties every future to the instance `Arc` and `unload` refuses
  while one is alive; it lands as a subprocess fault test, and an
  outstanding-clone gate on `unload` is not wanted. `teardown-and-unload` owns
  the unload rule this feeds.

## Out of scope

- The network contract, generated types, client, and the realtime design are
  owned by `metis-foundations`. The real endpoints are implemented in
  `metis-daemon`, not here.
- The plugin authoring SDK, the core plugin set, and the plugin CLI, which
  belong to `metis-plugin-sdk`. The CLI writes the manifest this effort
  defines; it does not define a second format.
- Workflow step implementations. Plugins provide tools, not step types.
- Workflow step executors and the agent loop, which belong to
  `metis-agent-core`.
- Daemon configuration resolution and secret distribution beyond the explicit
  host context and trust policy handed to the runtime.
- Hot unloading, meaning replacing a mapped library while the daemon runs.
  The lifetime guarantees land here; in-place replacement does not.
