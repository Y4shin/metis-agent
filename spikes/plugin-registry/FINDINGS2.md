# Plugin registry prototype, round two: findings

**Throwaway prototype. Not product code. Answers the questions round one left
open and then goes in the bin.**

Round one (`FINDINGS.md`) proved the object safe seam, the routing table, both
direction schema validation, per capability version gating, and the typed
facades, and stated plainly what it did **not** prove: the pending path, the
cancel path across the ABI, concurrency, unloading, and the placement of the
registry behind a feature gate. Round two is work on exactly those, plus the
plugin registry proper and the five fixes round one named.

## The question

Can the daemon hold host side plugins behind one object safe seam, drive a
cdylib the way the real plugin will (always `Pending`, completion later through
the host callback table), and own a first-class registry that loads, validates,
configures, discovers (fallibly), routes, enables, disables, lists, unregisters,
and tears down plugins, with an error taxonomy the caller can branch on, a
descriptor payload that is not ten times its own content, a measured answer on
name squatting, and the serde/JSON half of that registry kept out of the
primitives crate?

Sub questions that decided the shape:

1. Does the landed `authoring::HostAdapter` drive a cdylib end to end, or does
   the host need its own pending registry?
2. What actually happens on the cancel path, the dropped completion path, and
   the callback-after-shutdown path?
3. Does a plugin registry with a real lifecycle fit the landed ABI at all?
4. What does the descriptor size mitigation cost in calls?
5. Where does the feature boundary fall between the primitives and the
   registry?

## The shape

Same workspace, same three crates kept, one new crate:

| Crate | Kind | What it is now |
| --- | --- | --- |
| `proto-abi` | lib | **Kept, unused.** Round two uses the landed `plugin-contract` ABI types instead, so this copy is retained only as round one's record. |
| `proto-plugin-cdylib` | cdylib | Rewritten. Uses the landed `plugin-contract` ABI types and implements the table the way the real plugin will: `invoke` returns `Pending` and the result arrives through the host's `complete` callback from a plugin worker thread. |
| `proto-registry` | lib | **New.** The seam and the plugin registry proper. Primitives always present; the registry is behind the `registry` feature. |
| `proto-host` | lib + bin | Rewritten. The two adapters, the typed facades, the benchmark, and the host binary. The ABI adapter now drives the cdylib with the **landed** `HostAdapter`. |

The seam moved out of `proto-host` into `proto-registry`. Round one's
`proto-host/src/seam.rs` and `proto-host/src/registry.rs` are kept on disk, with
a header note marking them superseded, but are no longer crate modules, because
`FINDINGS2` is the record now.

### Where the prototype lives

- Working copy: `/tmp/metis-plugin-registry-prototype/`
- Durable copy (survives a `/tmp` wipe, `target/` excluded):
  `~/.local/share/metis/prototypes/plugin-registry/`

### The cdylib's capabilities

Thirteen routed capabilities, every one answered through the pending path:

| Capability | Version | What it exercises |
| --- | --- | --- |
| `tool.echo` | 1 | The ordinary late completion. |
| `tool.add` | 1 | A second typed tool, and the request the schema rejects. |
| `tool.uppercase` | 1 | A third. |
| `tool.bad_response` | 1 | Publishes `value: string`, returns `{"value": 42}`. |
| `provider.complete` | 2 | Provider kind; two `progress` callbacks then the completion. |
| `tool.delay` | 1 | Parks on the cancellation token, completes out of order, or suppresses on cancel. |
| `tool.races` | 1 | Completes even after a cancel, so the host has a completion to drop. |
| `tool.never` | 1 | Returns `Pending` and never completes. |
| `tool.straggler` | 1 | A detached thread that calls back after `shutdown`. |
| `tool.stats` | 1 | Per-instance counters and address, so two instances can be shown independent. |
| `tool.events` | 1 | Emits an event from inside a capability. |
| `tool.logs` | 1 | Logs at all four levels. |
| `tool.future` | 99 | Advertised at a version the host refuses. |

Three host-internal capabilities are deliberately **not** listed, so the
registry never routes them: `metis.schemas` (batched schema fetch),
`metis.configure` (configuration delivery), `metis.descriptors_inline` and
`metis.accounting` (measurement).

### How to run

```sh
cd /home/patric/Projects/metis-agent   # devenv root; cargo is not on the bare PATH
devenv shell -- cargo build --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml
devenv shell -- /tmp/metis-plugin-registry-prototype/target/debug/proto-host
```

The build of the whole workspace must come first: `cargo run -p proto-host`
does not rebuild the cdylib member, so without it the host loads a stale
library.

Captured runs: `host-output-round2.txt` (debug), `host-output-release-round2.txt`
(release), `feature-gate-builds.txt` (both feature builds and the dependency
closure), `symbol-and-feature-tree.txt` (the one exported symbol and the feature
closure).

## The host binary's output, verbatim (debug)

```
NOTE cdylib: /tmp/metis-plugin-registry-prototype/target/debug/libproto_plugin_cdylib.so

== plugin registry
PASS a plugin loaded from a library path reports a validated manifest
PASS the registry returned an instance handle
PASS capability discovery read the plugin's published list
PASS version gating refused exactly one capability and left the rest routable
PASS per-plugin configuration was delivered before discovery
PASS the registry kept the configuration it delivered
PASS schema discovery is one batched call, not one per capability
NOTE routing keys: provider.complete, tool.add, tool.bad_response, tool.delay, tool.echo, tool.events, tool.logs, tool.never, tool.races, tool.stats, tool.straggler, tool.uppercase
NOTE descriptor listing 2319 bytes by id (13 capabilities), schema fetch 5596 bytes in 1 call, inline listing 6014 bytes
PASS a native in-process plugin registered behind the same seam
PASS routing spans every enabled plugin
PASS a duplicate capability name fails the load under global uniqueness
PASS a failed load leaves the routing table unchanged
PASS fix 1: a failed capability listing fails the load with a typed discovery error
PASS fix 1: nothing was registered for the plugin whose listing failed
PASS a manifest that targets another ABI major fails the load
PASS a disabled plugin stays loaded but none of its capabilities are routable
PASS re-enabling makes the capability routable again
PASS a freshly registered plugin routes by capability name
PASS unregistering shuts the plugin down and removes its routes
PASS an unregistered capability is no longer routable
NOTE registered plugins: native-text-tools v0.1.0 abi1 instance=native-text-tools#1 caps=2 enabled=true; proto-plugin-cdylib vabi 1.0 abi1 instance=proto-plugin-cdylib#0 caps=12 enabled=true

== fix 2: machine-checkable errors
PASS a caller payload that violates the schema is its own variant
PASS a capability refused by version gating is its own variant
PASS a capability that was never registered is its own variant
PASS a response that violates the plugin's own published schema is its own variant
PASS a plugin-side failure carries the plugin's own status
PASS the five failure paths are distinguishable in code, not by message string
NOTE taxonomy: invalid argument: capability tool.add: the request violates its published schema: "not a number" is not of type "integer"; unsupported: capability tool.future advertises version 99, this host supports 2; not found: no enabled capability named tool.not_registered is registered; internal: plugin proto-plugin-cdylib returned a response for capability tool.bad_response that violates its published schema: 42 is not of type "string"; the plugin reported invalid_argument for capability tool.word_count: text must not be empty
PASS an ABI status maps onto the taxonomy with its status preserved

== fix 3: descriptor size
PASS the by-id listing is smaller than the inline listing
NOTE descriptor trade-off: inline 6014 bytes in 1 call; by id 2319 bytes + 5596 bytes of schema in 1 extra call(s), fetched once at load

== fix 4: name squatting under namespacing
PASS namespacing admits the plugin the global rule refused
NOTE namespaced routing keys: duplicate-echo-tools/tool.echo, proto-plugin-cdylib/provider.complete, proto-plugin-cdylib/tool.add, proto-plugin-cdylib/tool.bad_response, proto-plugin-cdylib/tool.delay, proto-plugin-cdylib/tool.echo, proto-plugin-cdylib/tool.events, proto-plugin-cdylib/tool.logs, proto-plugin-cdylib/tool.never, proto-plugin-cdylib/tool.races, proto-plugin-cdylib/tool.stats, proto-plugin-cdylib/tool.straggler, proto-plugin-cdylib/tool.uppercase
NOTE global keys are bare capability names, so the caller needs no plugin id but a second claimant is refused; namespaced keys are plugin_id/capability, so the caller names the plugin and both claimants coexist

== fix 5: feature gate placement
NOTE the registry lives in proto-registry behind the `registry` feature; the primitives build is `cargo build -p proto-registry --no-default-features` (no serde, no jsonschema) and the registry builds are `cargo build -p proto-registry --features registry`. Both were run and both compile; see FINDINGS2.md

== async: late completion, never inline
PASS invoke returned Pending, never an inline result
PASS the completion arrived later, through the host callback

== async: progress before completion
PASS two progress callbacks arrived before the completion, in order
PASS the completion followed the progress
PASS every progress payload was released by the plugin allocator

== async: events
PASS a capability emitted an event that reached the host with topic and payload
PASS instance start emitted an event

== async: logs at each level
PASS logs arrived at every level

== async: cancellation
PASS a cancel for a live request is accepted
PASS a second cancel is idempotent
PASS the plugin observed the token promptly while parked
PASS the plugin suppressed its completion and the host retired its pending record
NOTE the plugin observed the cancel 0 ms after the host requested it
PASS a completion that arrives after a successful cancel is dropped by the host
PASS the dropped completion's buffer was still released
PASS cancel of an unknown request id returns not found

== async: a completion that never arrives
PASS the bounded wait timed out instead of hanging
NOTE the bounded wait returned after 303 ms with Err(Internal { detail: "no completion for tool.never within 300ms" })

== async: concurrency
PASS concurrent in-flight calls to one instance and one capability all answer their own caller
PASS out-of-order completions are delivered to the right caller
PASS two instances of the same plugin have independent state
PASS two different plugins are in flight together without crossing

== async: instance lifecycle
PASS an instance can be created, used, and destroyed
PASS a double destroy is reported as not found
PASS an instance can be created again on a reused address
NOTE instance addresses: first "0x6371662e3b20", second "0x6371662e3b20"

== typed method families above the seam
PASS a typed tool method reaches a cdylib capability over the erased seam
PASS a second typed tool method reaches another cdylib capability
PASS a third typed tool method, same family, same seam
PASS the native in-process plugin answers through the same typed tool family
PASS a typed provider method family reaches a cdylib capability
PASS the native plugin answers through the same typed provider family
PASS a tool handle refuses a provider kind capability
PASS a typed wrapper whose Rust fields do not match the published schema fails at runtime, not compile time
NOTE erasure cost: the mismatch compiled; only the schema caught it (invalid argument: capability tool.echo: the request violates its published schema: "message" is a required property)
PASS the typed family surfaces a response violation as ResponseViolation

== what the registry costs
BENCH schema compile: 41 us per schema (14 routed capabilities => 28 schemas compiled at load, about 1.1 ms)
BENCH request validation: 976 ns per call (small object, 1 string field)
BENCH response validation: 1647 ns per call (small object, 2 fields)
BENCH both directions: 2623 ns per call, the number the registry adds to every routed call
BENCH baseline: serde JSON encode plus parse of the same payload, 1582 ns, which validation sits on top of
BENCH registry.invoke end to end, native in process plugin, both directions validated: 4791 ns per call
BENCH async completion latency, cdylib through the C table and the landed HostAdapter: 56980 ns per call (57.0 us), one worker thread round trip
BENCH validation share: 55% of the native call, 5% of the cdylib call
BENCH concurrency: 8 threads x 200 calls = 1600 in-flight completions, all matched to their caller, 13840 ns per call wall clock, 22.1ms total

== shutdown and unload ordering
PASS an in-flight call completed before shutdown returned (the worker was joined)
PASS shutdown joined the plugin's workers
PASS a callback arriving after shutdown is logged and ignored
PASS destroy_instance after shutdown returns not found
PASS the plugin released every buffer it allocated

== teardown
PASS the library was dropped only after shutdown had returned
NOTE teardown order: ["shutdown returned", "library unloaded"]

66 checks, 0 failures
```

Release is the same 66 checks, 0 failures. The full release output is
`host-output-release-round2.txt`.

## Workstream coverage

| # | Behaviour | Status |
| --- | --- | --- |
| 1.1 | Late completion through `complete`, work on a plugin worker thread | Done, printed |
| 1.2 | `progress` callbacks before completion, in order, payloads released | Done, printed |
| 1.3 | `emit_event` from a capability and from instance start | Done, printed |
| 1.4 | `log` at each level | Done, printed |
| 1.5 | Cancel: live request accepted | Done, printed |
| 1.6 | Cancel: plugin observes the token promptly while parked | Done, printed (0 ms observed) |
| 1.7 | Cancel: plugin suppresses its completion | Done, printed |
| 1.8 | Cancel: host retires its own pending record | Done, printed (the await never resolves) |
| 1.9 | Cancel: a late completion is dropped and its buffer released | Done, printed |
| 1.10 | Cancel: unknown request id is not found | Done, printed |
| 1.11 | Cancel: a second cancel is idempotent | Done, printed |
| 1.12 | A completion that never arrives: bounded wait, no hang | Done, printed |
| 1.13 | Concurrency: several in-flight calls, one instance, one capability | Done, printed |
| 1.14 | Concurrency: interleaved out-of-order completions | Done, printed |
| 1.15 | Concurrency: two instances of the same plugin, independent state | Done, printed |
| 1.16 | Concurrency: two different plugins in flight together | Done, printed |
| 1.17 | Shutdown joins the plugin's workers | Done, printed |
| 1.18 | Callback after shutdown logged and ignored, not freed state | Done, printed |
| 1.19 | `destroy_instance` after shutdown returns not found | Done, printed |
| 1.20 | Library dropped only after shutdown returned | Done, printed (order ledger) |
| 1.21 | Instance lifecycle: create, use, destroy, create again, double destroy | Done, printed |
| 2.1 | Registry: load from a path or register in memory | Done, printed |
| 2.2 | Registry: manifest validation (id, version, ABI major) | Done, printed |
| 2.3 | Registry: per-plugin configuration delivery | Done, printed |
| 2.4 | Registry: capability discovery, fallible | Done, printed |
| 2.5 | Registry: enable and disable | Done, printed |
| 2.6 | Registry: list plugins and instances | Done, printed |
| 2.7 | Registry: unregister, shutting down and dropping | Done, printed |
| 2.8 | Registry: route by capability across every enabled plugin | Done, printed |
| Fix 1 | A capabilities listing that can fail | Done, printed |
| Fix 2 | A machine-checkable error taxonomy | Done, printed (five variants) |
| Fix 3 | Descriptor size: measured, with the by-id mitigation | Done, printed |
| Fix 4 | Name squatting: global uniqueness vs namespacing, measured | Done, printed |
| Fix 5 | Feature gate: registry behind a feature, primitives-only build | Done, printed (build captured) |
| 3 | `FINDINGS2.md` (this file) | Done |

**Unfinished / not attempted:** schema evolution (old schema with a new plugin
and the reverse) is still unproven, as in round one. Codegen linking host and
plugin payload types is still not built. No large or nested payload was
benchmarked, and no schema with `$ref`/`$defs` was compiled. The
reference-string alternative to the by-id scheme was not built (by-id is the
one mitigation). Signature checks, bundle extraction, and platform selection
were deliberately left out (see below). Hot unload of a plugin that still has
live instances is not attempted; `unregister` shuts the instances down first.

## The five fixes: behaviour and cost

### Fix 1: a capabilities listing that can fail

Round one's `capabilities()` returned `Vec<CapabilityDescriptor>` with no error
channel, so a failed ABI listing registered a plugin with zero capabilities.
The seam now returns `Result<Vec<CapabilityDescriptor>, PluginError>` and the
registry turns an error into `RegistryError::Discovery`, which fails the load
and registers nothing. A native `BrokenDiscovery` plugin whose listing fails
proves it; the routing table is unchanged after the failed load.

Cost: none at runtime; one `Result` in the signature. The ABI adapter's
`capabilities()` now has somewhere to put a `list_capabilities` failure instead
of swallowing it.

### Fix 2: a machine-checkable error taxonomy

Five distinct variants, each with a check that matches on the variant (not the
message):

| Path | Variant | Printed |
| --- | --- | --- |
| caller payload violates the schema | `PluginError::CallerPayloadInvalid { capability, detail }` | `invalid argument: capability tool.add: ... "not a number" is not of type "integer"` |
| capability refused by version gating | `PluginError::VersionGated { capability, advertised, supported }` | `unsupported: capability tool.future advertises version 99, this host supports 2` |
| capability never registered | `PluginError::CapabilityNotFound { capability }` | `not found: no enabled capability named tool.not_registered is registered` |
| response violates the plugin's own schema | `PluginError::ResponseViolation { plugin, capability, detail }` | `internal: plugin proto-plugin-cdylib returned a response for capability tool.bad_response ...` |
| plugin-side failure with its own status | `PluginError::PluginFailed { status, capability, detail }` | `the plugin reported invalid_argument for capability tool.word_count: text must not be empty` |

The harness collects the five `variant()` names and asserts they are five
distinct strings, which is the "distinguishable in code" check: it would fail
if two paths collapsed to the same variant.

Cost: the error enum grows from six flat variants to eleven. The ABI status
space is unchanged: the adapter maps the frozen `MetisStatus` onto the
host-side variant, and `PluginFailed` carries the status, so the ABI is not
widened. Round one's finding 5 (attribution by string) is answered: the plugin
id is a field of `ResponseViolation`, not text.

### Fix 3: descriptor size

Measured over the same ABI, at load:

| Form | Bytes | Calls |
| --- | --- | --- |
| Inline schemas (round one shape) | 6014 | 1 (`list_capabilities`) |
| By id (the mitigation) | 2319 listing + 5596 schema | 2 (listing + one batched `metis.schemas`) |

The listing that crosses the ABI falls from 6014 to 2319 bytes, a 62%
reduction, at the cost of one extra call per plugin load that pulls 5596 bytes.
The honest reading: the mitigation does **not** reduce total bytes moved at
load; it moves *more* bytes (7915 vs 6014), because the schema documents now
travel separately and are JSON-escaped once as values rather than embedded in
the descriptors. What it buys is a small, cheap-to-log, cheap-to-store
descriptor listing (2319 vs 6014), at the price of one round trip and a small
schema store the host now owns. Round one's 2955 bytes for 6 capabilities was
492 bytes each; here the inline form is 463 bytes per capability, in the same
range, and the by-id form is 178 bytes per capability.

The mitigation is implemented end to end: the plugin's `list_capabilities`
returns descriptors by id, the registry batches every id into one
`metis.schemas` call, and a missing id fails the load with
`RegistryError::SchemaFetch`. A schema id that is advertised but not returned
is a load failure, not a silently unvalidated capability.

### Fix 4: name squatting

Both rules are built and measured.

- **Global uniqueness** (round one's rule, the default): the routing key is the
  bare capability name. A second plugin claiming `tool.echo` fails its load
  with `RegistryError::DuplicateCapability`. The check is printed. The cost is
  what round one found: the first loader decides who may register; the caller
  needs no plugin id.
- **Namespacing**: the routing key is `plugin_id/capability`. The duplicate
  claimant registers, and both route:
  `proto-plugin-cdylib/tool.echo` answers `x`, `duplicate-echo-tools/tool.echo`
  answers `duplicate:x`. The printed namespaced key list is the observable
  change to the key format.

What it does to the caller: under namespacing every call site has to carry the
plugin id, which is a change to the routing key format, to whatever advertises
capabilities to a model, and to the agent core's tool invocation path. Under
global uniqueness the caller is unchanged but a hostile or buggy plugin can
deny a real plugin its registration. The measured trade-off is not in bytes or
nanoseconds; it is caller ergonomics against isolation. This prototype does not
choose; it shows the two shapes cost differently at exactly one seam
(`PluginRegistry::key_for`).

Secondary finding: this prototype's registry admits **one instance per plugin
id** (a second `register` with the same id is `RegistryError::DuplicatePlugin`),
so even namespacing does not let two instances of one plugin coexist here. A
real host that wants several instances of one plugin behind the registry needs
the instance id in the key, which namespacing by `plugin_id` alone does not
provide.

### Fix 5: feature gate placement

The registry lives in a new crate, `proto-registry`, with the serde and JSON
Schema half behind the `registry` feature:

- `proto-registry` primitives (no features): the seam, the manifest, the
  descriptors, the error taxonomy, the version policy, the executor. Builds
  with `--no-default-features`; `cargo tree -p proto-registry
  --no-default-features` is just the crate, no serde, no jsonschema.
- `proto-registry --features registry`: adds serde, serde_json, and jsonschema
  (31 transitive crates in the closure) and compiles the registry proper.

`proto-host` enables the feature; the primitives crate is what the daemon could
depend on without pulling a JSON stack, exactly as `plugin-contract` keeps
`authoring` off by default. The captured builds are in
`feature-gate-builds.txt`.

## Benchmark table

Release numbers (the honest ones; debug is roughly an order of magnitude
larger). Four release runs and one debug run:

| Measurement | Release | Debug |
| --- | --- | --- |
| schema compile, per schema | 14 us | 41 us |
| request validation, small object | 128 to 152 ns | 976 ns |
| response validation, small object | 191 to 255 ns | 1647 ns |
| **both directions, per routed call** | **319 to 407 ns** | **2623 ns** |
| serde encode plus parse (the baseline) | 127 ns | 1582 ns |
| `registry.invoke`, native in process, both directions validated | 486 to 735 ns | 4791 ns |
| `registry.invoke`, cdylib through the ABI, both directions validated, awaited | 21.0 to 40.4 us | 57.0 us |
| **async completion latency** (register, invoke, callback, wake, await) | **21.0 to 40.4 us** | **57.0 us** |
| concurrency: 8 threads x 200 calls = 1600 in flight, all matched | 8662 to 11190 ns/call wall, 13.9 to 17.9 ms total | 13840 ns/call, 22.1 ms total |
| validation share of the native call | 54% to 78% | 55% |
| validation share of the cdylib call | 1% to 2% | 5% |

The two numbers that matter:

- **Routing and validation are cheap.** The native round trip is 486 ns in
  release, of which validation is 319 ns. The by-id descriptor work is one
  load-time call, not a per-call cost.
- **The async completion path is the expensive one and it is not the
  registry's fault.** 21 to 40 us per cdylib call in release against 486 to 735
  ns for the native call: the difference is the pending round trip, and the
  dominant term is that this prototype's plugin-side executor is
  `thread::spawn` **per call**, exactly as the landed `plugin_adapter`'s
  `executor::spawn` is. The spread across runs is that thread spawn. A real SDK
  owning a thread pool or a real executor would not pay a thread spawn per
  call. The prototype measures the landed stand-in, not the final design, and
  that is a finding: the current landed executor is a prototype-grade
  scheduler, and the per-call thread spawn is the whole async cost.

Under concurrency the per-call wall time drops to 8.7 to 11.2 us, because 8
threads overlap the spawn and park time, which is the expected signature of a
spawn-bound path.

## Line counts per crate

Non blank, non `//` lines (raw counts in parentheses), the same measurement the
earlier spikes used. `proto-host/src/seam.rs` and `proto-host/src/registry.rs`
are counted but **not compiled**: they are round one's files retained on disk.

| Crate | File | Code lines (raw) |
| --- | --- | --- |
| `proto-abi` | `lib.rs` | 172 (225) |
| `proto-plugin-cdylib` | `lib.rs` | 820 (977) |
| `proto-registry` | `lib.rs` (primitives) | 272 (415) |
| | `registry.rs` | 510 (647) |
| `proto-host` | `abi_adapter.rs` | 417 (541) |
| | `bench.rs` | 125 (144) |
| | `lib.rs` | 5 (27) |
| | `main.rs` | 885 (971) |
| | `native.rs` | 327 (391) |
| | `typed.rs` | 125 (164) |
| | `types.rs` | 90 (114) |
| | **`proto-host` compiled total** | **1974** |
| | **compiled total, all crates** | **3748** |
| `proto-host` (round one, not compiled) | `seam.rs` + `registry.rs` | 377 (554) |

The registry proper is 510 code lines against a 272 line primitives half. The
host binary is 885 lines, of which about 700 are the check harness. As round
one found, the adapters and the host-side scaffolding are where the size is,
not the registry.

## Reuse, reimplementation, and what could not be reused

**Reused from the landed code:**

- `plugin-contract::abi` for every ABI type, both directions. The cdylib and
  the host share one definition of the table, the buffer, the status space, and
  the payload envelope, instead of the round one copy in `proto-abi`.
- `plugin-contract::payload` for `metis_encode_payload` / `metis_payload_body`.
- `plugin-contract::authoring::HostAdapter` **to drive the cdylib**. This is the
  big one: the pending slot, the waker, the blocking bounded wait
  (`Await::wait_timeout`), the dropped-completion path, and the
  plugin-buffer release are all the landed code. The host adapter wrapper here
  is ~70 lines around it. Round one's assumption 5 (the host would need to
  invent a second pending registry) turned out to be unnecessary.

**Reimplemented (with the reason):**

- The cdylib's table, rather than the landed
  `authoring::plugin_adapter::table::<P>()`. The landed plugin adapter is
  capped at four capabilities (one associated type and one associated const
  per arm), and this prototype deliberately carries thirteen so the capability
  set is a runtime registry and not a fixed four. The landed adapter is also
  the trait path; the cdylib here is the "plugin that implements the table
  directly" the adapter's own doc comment keeps room for. So the async plumbing
  on the plugin side (worker threads, the cancel token, the completion guard,
  the detached straggler, the accounting) is hand written in the cdylib, and
  the landed adapter's plugin-side pending registry is what it mirrors.
- The registry itself, which does not exist in the landed tree yet
  (`plugin-runtime` is empty). This is the deliverable.

**Could not reuse:**

- The landed `PluginError` type as the registry's error type. It is the six ABI
  statuses flat; the registry needs five distinct variants the ABI cannot
  express, so `proto-registry` has its own `PluginError` and the ABI adapter
  maps `MetisStatus` onto it.
- The landed `CapabilityDescriptor` as the registry's descriptor. It carries
  one `schema: String` reference; the registry needs a request and a response
  schema id plus the fetched documents. The registry's descriptor is its own.
- The landed authoring trait as the seam, for the same reason round one found:
  it is not dyn compatible (three independent reasons), so `Box<dyn Plugin>`
  needs the moved-out capability list. That decision stands.

## Where the design fought back

1. **The landed `HostAdapter` has no way to retire a slot for a synchronous
   error.** `register` inserts the slot; `invoke` only consumes the slot's
   result through the callback, and the only path that removes a slot is a
   successful `cancel`. If `invoke` returns a non-`Pending`, non-`Ok` status
   (a plugin that rejects the request inline), the slot stays in the map
   forever. The ABI adapter here avoids the leak by only calling `invoke` for
   capabilities the registry has already admitted and validated, so a
   synchronous error is impossible on that path, but the gap is real: a real
   host wants `invoke` to retire its own slot on a synchronous error, or an
   explicit `unregister(request_id)`.
2. **A callback that arrives after `shutdown` needs a guard the landed adapter
   does not provide on the plugin side.** The host-side `HostAdapter` handles a
   late completion (drops it and releases its buffer), but the *plugin* side
   has to refuse to call back at all once its state may be gone. The cdylib
   does this with an `Arc<InstanceState>` the worker holds plus a `shut_down`
   flag checked before any callback, and a detached `tool.straggler` thread
   that fires after `shutdown` proves it: the callback is logged and ignored
   instead of dereferencing freed state. This is the single most load-bearing
   invariant in the whole ABI, and it is not something the frozen table can
   enforce; it is a discipline the SDK has to impose.
3. **The straggler thread forces the ordering of unload.** Because a detached
   plugin thread may still run plugin code, the library must not be unloaded
   while it does. The prototype waits for the straggler before dropping the
   `Library`, and the order ledger (`["shutdown returned", "library
   unloaded"]`) proves `shutdown` came first. A real runtime needs to account
   for *every* plugin thread, including ones `shutdown` cannot join, before
   unload; joining the workers is necessary and not sufficient.
4. **The descriptor size finding is subtler than round one thought.** Making
   the listing smaller with schema-by-id moves *more* total bytes at load
   (7915 vs 6014), not fewer. The win is a smaller descriptor listing, not less
   data. If the real goal is less data, the answer is a schema store the host
   caches across loads, or a schema hash the plugin returns and the host
   already has, neither of which this prototype built.
5. **A refused capability is now distinguishable from a typo.** Round one's
   finding 4 (a version-gated capability looked exactly like a missing one) is
   fixed: the registry keeps the refusal and returns `VersionGated` for it
   while returning `CapabilityNotFound` for a name no plugin ever published.
   The check prints both variants.
6. **The registry admits one instance per plugin id.** Two instances of the
   same plugin are demonstrated at the adapter level (two `AbiPlugin`s, two
   instances, independent counters and addresses), not inside the registry:
   `register` refuses a duplicate plugin id, and under global uniqueness a
   second claimant of the same capability fails regardless. A daemon that wants
   several instances of one plugin routed needs an instance-qualified key,
   which neither rule in this prototype provides.
7. **`metis.*` capabilities have to be invisible to routing.** Schema fetch,
   configure, inline descriptors, and accounting are host-internal protocol
   calls that travel as ordinary capabilities. They are not listed by the
   plugin, so the registry never routes or validates them. This works, but it
   means the host reaches the plugin through a second path (the ABI adapter's
   `call_pending`) that bypasses the registry's validation. That second path is
   trusted host code, and a real design should say so explicitly rather than
   leave it implicit.
8. **`Sync` is still what makes the seam usable.** Round one found that `Sync`
   on the plugin is what makes `&self` usable inside a `Send` future; round two
   confirms it under real concurrency: the registry is shared behind an `Arc`
   across eight host threads and sixteen hundred in-flight completions, all
   delivered to the right caller, and every `unsafe impl Send + Sync for
   AbiPlugin` over two raw pointers holds under that load. Nothing in the ABI
   checks the C side is thread safe; the prototype relies on it.

## Packaging concerns deliberately left out

These belong to the real `plugin-runtime`, not to this prototype, and none was
built: bundle extraction, signature and integrity checks, platform and
architecture selection inside a bundle, manifest file format and extension,
and hot unload of a library that still has live instances. The registry here
takes a library path and an injected loader closure; it does not know about
bundles.

## What remains unproven

Stated plainly:

- **Schema evolution.** Still no test that an old schema with a new plugin, or
  a new schema with an old host, degrades sanely.
- **Large or nested payloads.** Only small objects were measured, and no schema
  with `$ref` or `$defs` was compiled.
- **Codegen.** Nothing links the host's payload types to the plugin's; the
  mismatch check shows the drift is caught only at runtime.
- **Several instances of one plugin behind the registry.** Demonstrated at the
  adapter level, not in the registry.
- **A real executor.** The async latency is dominated by a per-call thread
  spawn, which is the landed stand-in, not a final design.
- **Hot unload.** `unregister` shuts instances down first; concurrent unload is
  not attempted.
- **Cross-language plugins.** Only Rust was built against the ABI.

## Reproduction

```sh
cd /home/patric/Projects/metis-agent

# 1. build the whole workspace. This MUST precede any run: `cargo run -p
#    proto-host` does not rebuild the cdylib member.
devenv shell -- cargo build --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml

# 2. drive every behaviour. One PASS/FAIL line per check, a summary, non zero
#    exit on failure.
devenv shell -- /tmp/metis-plugin-registry-prototype/target/debug/proto-host

# 3. the release numbers
devenv shell -- cargo build --release --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml
devenv shell -- /tmp/metis-plugin-registry-prototype/target/release/proto-host

# 4. the feature gate: primitives-only then registry
devenv shell -- cargo build --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml -p proto-registry --no-default-features
devenv shell -- cargo build --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml -p proto-registry --features registry
devenv shell -- cargo tree  --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml -p proto-registry --no-default-features

# 5. the cdylib still exports exactly one dynamic symbol
devenv shell -- nm -D --defined-only /tmp/metis-plugin-registry-prototype/target/debug/libproto_plugin_cdylib.so

# 6. the line counts (no devenv needed)
cd /tmp/metis-plugin-registry-prototype
for f in $(find crates -name '*.rs' | sort); do
  printf "%-52s raw=%4s code=%4s\n" "$f" "$(wc -l < "$f")" \
    "$(awk '/^[[:space:]]*$/{next} /^[[:space:]]*\/\//{next} {n++} END{print n+0}' "$f")"
done
```
