# Plugin registry: session handoff

Purpose: hand the plugin-registry design discussion to a future session. This
file records what is already decided, what the prototype measured, and every
question that is still open. The actual implementation task is deliberately
left as TBD at the bottom; it gets defined when the handover is prepared.

Status: discussion paused. No repository code changed for this work, no ticket
exists, and the prototype is throwaway by construction.

## Where the prototype lives

Two copies, both kept on purpose. The user may add one to the repository later
as a reference for the real implementation.

- Working copy: `/tmp/metis-plugin-registry-prototype/`
- Durable copy, `target/` excluded:
  `~/.local/share/metis/prototypes/plugin-registry/`
- Read `FINDINGS.md` (round one: the registry shape, dyn compatibility, type
  erasure, first validation numbers) and `FINDINGS2.md` (round two: the async
  and lifecycle surface, the plugin registry proper, the five fixes).
- Verified independently: the release host binary reports 66 checks, 0
  failures, exit 0. The prototype path-depends on the real
  `/home/patric/Projects/metis-agent/crates/plugin-contract`, so it drives the
  landed ABI types and the landed `authoring::HostAdapter`, not copies.

## What already exists in the repository

- `crates/plugin-contract`: the C ABI (`abi`), the payload envelope
  (`payload`), and the feature-gated authoring layer (`authoring`, off by
  default). Finalized: `plugin-abi-core`, `plugin-authoring-trait`.
- The authoring trait is fixed at four role-named arms (`answer`, `deferred`,
  `cancellable`, `fault`), each with a capability-name constant and its own
  request and response types. That is what caps a plugin at four capabilities.
- The same four-arm shape is what makes the trait unusable as a trait object:
  the prototype captured the verbatim E0038/E0191 diagnostics for
  `const ANSWER`, `type Answer`, `impl Future` returns, `async fn`, and generic
  methods. The cap and the `dyn` incompatibility are one design decision, not
  two.

## Decided (do not reopen without cause)

- Capabilities are a registry of entries (name, kind, version, schema, handler),
  not a fixed set of trait methods. The C ABI already supports this and does not
  change: `list_capabilities` returns a list, `invoke` is name-addressed.
- The host-side seam is object safe (`Box<dyn Plugin>`, `BoxFuture` returns, no
  associated constants or types). One registry, one routing table, one
  validation point.
- Two adapters behind that seam: a native in-process Rust plugin, and a
  host-side adapter over an ABI-loaded cdylib. Trait objects never cross the C
  boundary; a cdylib cannot hand one out.
- Schema form: JSON Schema 2020-12 derived from the author's Rust types and
  shipped inline in the descriptor, so the host validates offline with no
  resolver. Schema generation belongs to the SDK, not to `plugin-contract`.
- Validation: the registry validates both directions. A request that violates
  its schema is `invalid_argument`; a response that violates the plugin's own
  published schema is an internal error attributed to that plugin. No
  quarantine.
- Naming and instances, as the user specified: each plugin gets a config-derived
  instance name, multiple instances of one plugin may be loaded, and calls are
  qualified as `<instance-name>.<tool-name>` rather than by plugin id. This
  supersedes the earlier "reject duplicate capability names at load" rule.

## Open questions to settle before the actual task

### Q1. The per-call executor on the plugin side (under discussion now)

The ABI is synchronous and callback based; the authoring trait is async. After
`invoke` returns `Pending`, something must drive the capability future.
`plugin-contract`'s executor does that with one fresh OS thread per pending call
(`executor::spawn`), and its own docs say the real executor is meant to live in
`metis-plugin-sdk`.

Measured cost: the async cdylib path is about 26 µs per call, dominated by that
thread create plus join. The validated native in-process path, which is
synchronous, is 486 ns. Under concurrency (8 threads times 200 calls) it
measured 8.9 µs per call wall clock, so thread creation partly parallelises but
still dominates. Relative to a tool that does real I/O this is noise; relative
to a cheap capability called in a loop it is the entire cost.

Options to weigh: a bounded thread pool, a shared async runtime such as tokio
inside the plugin, a synchronous fast path where the future completes on first
poll, or keeping thread-per-call and accepting the cost. Note the constraint
that `plugin-contract` must stay runtime-free, so any executor lives in the SDK.

### Q2. Host and plugin payload types disagreeing, and only failing at runtime

Only bytes cross the boundary, so the compiler never sees both sides. Host
declares `struct EchoRequest { message: String }`, plugin declares
`struct EchoRequest { msg: String }`; both compile, and the mismatch surfaces
only when the payload is decoded or validated. In the prototype the wrong
struct compiled and schema validation caught it at call time
(`"message" is a required property`). Silent misbehaviour is possible when names
coincide but types differ.

Options to weigh: generate both sides from one schema, share a payload types
crate, or accept that schema validation is the only tie between the two sides.

### Q3. The landed `HostAdapter` cannot retire a pending slot

What a pending slot is: before calling `invoke`, the host registers
`request_id -> slot` so a completion that overtakes the return still finds its
caller. In the landed code, entries are removed in exactly two places, a
successful `cancel` and the `complete` callback. There is no `unregister`, no
`Drop` for `Await`, and `wait_timeout` expiry does not clean up either.

Consequence: a synchronous non-`Pending` error from `invoke` (reachable through
a bad capability name or a rejected payload envelope) leaves the entry in the
map forever, one per such call, plus a stale request id that a later completion
could match. The same happens when a completion simply never arrives and the
wait times out. In a long-lived daemon this is unbounded growth.

Options to weigh: add an explicit `unregister(request_id)`, retire on timeout
expiry, give `Await` a `Drop`, or require `invoke` to retire its own slot. This
lands in `plugin-contract`, so it is a ticket in this effort.

### Q4. Descriptor size: inline schemas or by-id

Inline (chosen form): one `list_capabilities` payload carries each capability
with both schemas. For 13 capabilities that is 6014 bytes in one call.

By-id alternative: the listing carries schema ids (2319 bytes) and the host
fetches schema bodies in one extra call (5596 bytes) into a host-side store.
Total transferred 7915 bytes against 6014, so splitting costs data rather than
saving it; what it buys is a 61% smaller listing, which is the part a host logs,
a UI shows, and a model's tool list holds, plus schemas that can be cached
across reloads.

Options to weigh: stay inline, switch to by-id with a schema store, fetch
schemas lazily per capability on first use, or compress the listing. Also
unsettled and related: there is no payload size limit or nesting bound anywhere
in the contract yet, and validating adversarial documents needs both.

### Q5. Instance naming details

The direction is decided (config-derived instance name, multiple instances per
plugin, `<instance-name>.<tool-name>` qualification). Still to pin:

- Is the instance name unique across the registry, and what happens on a
  duplicate: refuse the load, or suffix it?
- Does the descriptor carry the instance name, or does the host qualify
  capability names when it builds the routing table?
- What happens to an in-flight call when its instance is unregistered, and are
  routes for a disabled instance removed or merely not routed?
- Does the plugin itself ever see the instance name, or is it host-side only?

### Q6. Error taxonomy against the frozen ABI status space

The registry needs to distinguish at least: a request that violates its schema,
a capability refused by version gating, a capability that was never registered,
a response that violates the plugin's own schema, and a plugin-side failure
with its own status. The frozen `MetisStatus` set cannot express these
separately, so the prototype distinguished them in its own error enum and
mapped onto the status space at the adapter, which means the distinction
survives only where both sides agree.

Options to weigh: carry the taxonomy in a typed detail payload under one status,
widen the status enum (a major-version ABI change), or accept a lossy mapping
and keep the taxonomy host-side only. This is ADR-shaped.

### Q7. Which effort owns what

- `plugin-contract` (this effort): the plugin-side handler registry, an
  enforceable schema field on the descriptor, and the object-safe seam the host
  consumes. Must stay dependency-small and runtime-free.
- `plugin-runtime` (`metis-plugins` effort): the `PluginRegistry`, the native
  and ABI adapters, routing, version gating, edge validation, plugin lifecycle
  and library lifetime. Its rough-plan scope already says "plugin registry,
  capability discovery, and dispatch".
- `metis-plugin-sdk`: the per-kind typed method families, typed handler
  wrappers, schema derivation, and the async executor.

Open: confirm this split, and confirm that the prototype's shape becomes the
written interface contract between the three.

### Q8. Unverified areas, each of which needs a decision or a spike

- Real integration with the daemon's async executor. The prototype used its own
  `block_on`; nothing shows a registry call can be awaited without blocking an
  executor.
- Schema evolution within a capability's advertised version.
- Large or nested payloads, and `$ref`/`$defs` in the published schemas.
- A plugin implemented in another language, which is the ABI's premise and was
  never exercised.
- Hot unload, and disabling versus unloading a plugin.
- All packaging: bundle extraction, signature checks, platform selection, and
  the manifest's real field set.

### Q9. The ABI's inline completion path has no in-tree exerciser

`plugin-authoring-trait` rewrote the fixture through the authoring trait, whose
adapter always returns `Pending`. The ABI keeps its inline path for direct
implementers, but nothing in the tree exercises it any more. Decide whether that
stays an accepted gap or gets one direct-implementation test.

## Overhead summary (release, measured in the prototype)

| Path | Cost |
| --- | --- |
| Schema compile at load | ~14 µs per schema, ~0.4 ms for 28 |
| Edge validation, both directions | ~320 ns per call |
| Validated native in-process call | ~490 ns |
| Validated cdylib call, async through the landed `HostAdapter` | ~26 µs, dominated by a per-call thread spawn |
| Same, 8x200 concurrent in-flight calls | ~8.9 µs per call wall clock |
| Descriptor listing | 6014 bytes inline, or 2319 bytes by id plus 5596 bytes of schemas in one extra call |

## TBD

The actual task. To be defined when the handover is prepared: the plugin-side
registry and enforceable schema in `plugin-contract`, the `PluginRegistry` and
edge validation in `metis-plugins`, the SDK's typed facades and schema
derivation, and the ADR for the error taxonomy. It depends on Q1 to Q7 being
settled first.
