# Plugin registry prototype: findings

**Throwaway prototype. Not product code. Answers one design question and then goes in the bin.**

**Deviation from the prototype skill, stated up front:** the skill's logic branch
asks for a single shareable HTML file. The question here is Rust trait object
safety, a C ABI, and a JSON Schema validation seam, and none of that is
expressible in HTML, so the artifact is a three crate Rust cargo workspace with
a host binary instead. Everything else the skill asks for is kept: throwaway,
clearly marked, one command to run, no persistence, no polish, no abstractions,
and no tests beyond what proves the shape (the host binary is the check
harness).

## The question

Can the daemon hold host side plugins as `Box<dyn Plugin>` trait objects, where
a plugin's capabilities are a *registry of entries* rather than a fixed set of
trait methods, with the cdylib and native adapters behind that one seam, schema
validation in both directions, per capability version gating, and typed method
families layered on top?

Sub questions that decided the shape:

1. What object safety requirements does `Box<dyn Plugin>` actually impose on the
   landed authoring trait?
2. Where does type erasure force itself, and what does it cost ergonomically?
3. What does validating both directions cost per call?
4. Where does the design fight back against the landed ABI, the feature gate,
   and the daemon's async model?

## What was built

`/tmp/metis-plugin-registry-prototype/`, its own workspace, its own lockfile,
its own target directory, no dependency on the metis-agent workspace.

| Crate | Kind | What it is |
| --- | --- | --- |
| `proto-abi` | lib | The C ABI, copied down from `plugin-contract`: `MetisPluginTableV1` (size first, seven function pointers), `MetisHostApiV1` (size, user_data, complete, progress, emit_event, log), `MetisStatus`, `MetisBuffer`, `metis_read_prefix`, and the payload envelope. |
| `proto-plugin-cdylib` | cdylib | A plugin that implements the C table directly (no authoring trait) and advertises **six** capabilities, each publishing real JSON Schema 2020-12 documents for request and response generated from its Rust types with `schemars`. Exports exactly one dynamic symbol, `metis_plugin_entry_v1`. |
| `proto-host` | lib + bin | The object safe `Plugin` seam, the registry and routing table, the ABI adapter (libloading), the native in process plugin, the typed tool and provider method families, the validation microbenchmark, and the host binary. |

The cdylib's six capabilities, and why each is there:

| Capability | Kind | Version | Why it is in the prototype |
| --- | --- | --- | --- |
| `tool.echo` | tool | 1 | The ordinary round trip, and the small payload the benchmark uses. |
| `tool.add` | tool | 1 | A second typed tool method, and the request that validation rejects. |
| `tool.uppercase` | tool | 1 | A third, so "six capabilities" is not two capability names padded. |
| `tool.bad_response` | tool | 1 | Deliberately misbehaving: publishes `value: string`, returns `{"value": 42}`. |
| `provider.complete` | provider | 2 | A provider kind, at a version the gate accepts, so the kind split is real. |
| `tool.future` | tool | 99 | Advertised at a version the registry does not support, so it is refused while the rest of the plugin stays usable. |

Two more plugins register into the same registry: `NativeTextTools` (a native in
process plugin, no ABI, with a typed `word_count` and a typed `summarize`), and
`DuplicateEchoTools` (native, claims `tool.echo` to make the duplicate name load
failure observable).

`object_safety_probe.rs` is a standalone file, not a workspace member, that
exists only to capture verbatim which trait shapes rustc refuses to make object
safe.

## How to run

```sh
cd /home/patric/Projects/metis-agent   # devenv root; cargo is not on the bare PATH
devenv shell -- cargo build --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml
devenv shell -- cargo run   --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml -p proto-host
```

The build of the whole workspace has to come first: `cargo run -p proto-host`
does not rebuild the cdylib member, so without it the host loads a stale library
(the same trap the two earlier plugin spikes hit). The host locates the cdylib
next to its own binary and exits non zero on any failed check.

Captured runs: `host-output.txt` (debug), `host-output-release.txt` (release),
`object_safety_probe.stderr.txt` (the verbatim compiler diagnostics).

## The host binary's output, verbatim (debug build)

```
== seam
NOTE the registry holds Arc<dyn Plugin>; the landed authoring trait cannot be a trait object
NOTE its four arms are an associated const and an associated type per capability, plus an RPITIT method, and all three shapes are dyn incompatible (object_safety_probe.rs)
NOTE so the capability set moved out of the trait and into a published list, which is what the C ABI already does

== cdylib through the C table
NOTE cdylib: /tmp/metis-plugin-registry-prototype/target/debug/libproto_plugin_cdylib.so
PASS the cdylib loaded with libloading and was read through metis_read_prefix
PASS the cdylib advertises six capabilities, not four
NOTE capabilities: tool.echo@1, tool.add@1, tool.uppercase@1, tool.bad_response@1, provider.complete@2, tool.future@99
NOTE tool.echo request schema arrived over the ABI: {"$schema":"https://json-schema.org/draft/2020-12/schema","properties":{"message":{"type":"string"}},"required":["message"],"title":"EchoRequest","type":"object"}
NOTE the list_capabilities payload is 2955 bytes for 6 capabilities, 492 bytes each, because both schemas travel inside the descriptors

== registry
PASS the routing table was built from the plugin's published capability list
PASS version gating refused exactly one capability and left the rest routable
PASS a native in process plugin joined the same registry behind the same seam, no ABI involved
PASS the routing table is keyed by capability name and holds every routable capability
NOTE routing table: provider.complete, provider.summarize, tool.add, tool.bad_response, tool.echo, tool.uppercase, tool.word_count
PASS a duplicate capability name fails the load
PASS a failed load leaves the routing table unchanged

== schema validation
PASS the registry rejected a request violating the published schema before the plugin saw it
PASS the plugin never saw the rejected request
PASS a valid request is routed and its response validates
PASS the plugin saw this one
PASS a response violating the plugin's own published schema is an internal error attributed to the plugin
NOTE attributed error: internal: plugin proto-plugin-cdylib returned a response for capability tool.bad_response that violates its published schema: 42 is not of type "string"
NOTE tool.bad_response published response schema: {"$schema":"https://json-schema.org/draft/2020-12/schema","description":"The response schema `tool.bad_response` publishes. The plugin does not honour\nit: see the `\"tool.bad_response\"` arm in `invoke`.","properties":{"value":{"description":"Published as a string. The plugin returns `42`.","type":"string"}},"required":["value"],"title":"BadResponse","type":"object"}
NOTE the plugin returned {"value": 42} for it
PASS a capability the version gate refused, or one that was never registered, is not routable
PASS the refused capability is not routable
PASS the plugin that advertised the refused capability still answers its routed ones

== typed method families above the seam
PASS a typed tool method reaches a cdylib capability over the erased seam
PASS a second typed tool method reaches another cdylib capability
PASS a third typed tool method, same family, same seam
PASS the native in process plugin answers through the same typed tool family
PASS a typed provider method family reaches a cdylib capability
PASS the native plugin answers through the same typed provider family
PASS a tool handle refuses a provider kind capability
PASS a typed wrapper whose Rust fields do not match the published schema fails at runtime, not at compile time
NOTE erasure cost: the mismatch above compiled; only the schema caught it (invalid argument: capability tool.echo: the request violates its published schema: "message" is a required property)
PASS the typed family surfaces the same misbehaviour as the same attributed internal error

== what validation costs
BENCH schema compile: 27 us per schema (7 routed capabilities => 14 schemas compiled at load, about 0.4 ms)
BENCH request validation: 1057 ns per call (small object, 1 string field)
BENCH response validation: 1629 ns per call (small object, 2 fields)
BENCH both directions: 2686 ns per call, the number the registry adds to every routed call
BENCH baseline: serde JSON encode plus parse of the same payload, 1613 ns, which validation sits on top of
BENCH registry.invoke end to end, native in process plugin, both directions validated: 4836 ns per call
BENCH registry.invoke end to end, cdylib through the C table, both directions validated: 5165 ns per call
BENCH validation share: 56% of the native call, 52% of the cdylib call

== teardown
PASS every registered plugin was shut down through the seam
PASS the cdylib released every plugin buffer it was handed, through its own release_buffer

27 checks, 0 failures
```

Exit code 0. The failure path was checked too, by pointing the host at a library
that does not exist:

```
FAIL setup :: could not load /tmp/does-not-exist.so: dlopen failed

1 checks, 1 failures
```

exit code 1.

## Object safety requirements actually hit

`seam.rs` holds the whole trait. It compiles as a trait object, and that is the
demonstration. What it had to give up in order to compile is the finding.

The landed `authoring::Plugin` cannot be a trait object at all, and it fails for
three independent reasons, one per shape it uses. `object_safety_probe.rs`
reproduces each one. Verbatim, from `object_safety_probe.stderr.txt`:

```
error[E0038]: the trait `HasAssocConst` is not dyn compatible
21 |         const ANSWER: &'static str = "";
   |               ^^^^^^ ...because it contains associated const `ANSWER`

error[E0191]: the value of the associated type `Answer` in `HasAssocType` must be specified
30 |         type Answer;
   |         ----------- `Answer` defined here

error[E0038]: the trait `ReturnsImplFuture` is not dyn compatible
40 |         fn answer(&self) -> impl Future<Output = ()> + Send;
   |                             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ ...because method `answer` references an `impl Trait` type in its return type

error[E0038]: the trait `UsesAsyncFn` is not dyn compatible
48 |         async fn answer(&self);
   |                  ^^^^^^ ...because method `answer` is `async`

error[E0038]: the trait `GenericMethod` is not dyn compatible
56 |         fn invoke<T: Clone>(&self, value: T);
   |            ^^^^^^ ...because method `invoke` has generic type parameters

error: aborting due to 5 previous errors
```

Read that against the landed trait: `const ANSWER` and `type Answer` are the two
reasons a plugin is capped at four capabilities *and* the two reasons no plugin
can ever be a trait object. The cap and the dyn incompatibility are the same
design decision seen from two sides, and the fix is the same fix: move the
capability set out of the trait and into a runtime list.

The requirements the prototype's seam had to satisfy:

1. **Every method takes `&self`.** An `&mut self` method is object safe but
   useless here, because the registry holds `Arc<dyn Plugin>` and hands out
   shared references. Both adapters therefore keep mutable state behind
   `Mutex` and atomics (`AbiHostState.logs`, the cdylib's invoke counter).
2. **No associated consts, no associated types, no generics on methods, no
   `impl Trait` returns, no `async fn`.** The capability set and the payload
   types both had to become data.
3. **`-> BoxFuture<'a, T>` with the lifetime tied to `&'a self`, `&'a str` and
   `&'a Call`.** This is the price of keeping the call borrowed instead of
   handing the plugin owned copies of everything. A `'static` boxed future would
   also be object safe, but then registration would have to clone the capability
   name and the call context into every call.
4. **The boxed future must be `Send`, and the trait must be
   `Send + Sync + 'static`, or `Box<dyn Plugin>` cannot be captured by a `Send`
   future at all.** This is the requirement with the longest shadow: it is what
   forces `unsafe impl Send + Sync for AbiPlugin` over two raw pointers, and it
   is what makes the daemon's choice of executor a constraint on plugin authors
   rather than an implementation detail.
5. **`capabilities(&self)` has to return owned data.** An object safe
   `-> &[CapabilityDescriptor]` is possible, but it would force every adapter to
   cache the list, and the ABI adapter has no list to borrow: it *fetches* the
   list across the FFI boundary.

Things that surprised me:

- **The failure lands at the `dyn` use site, not at the trait definition.** The
  landed trait compiles today and has `#[allow]`-free clean builds. It will
  fail, with E0038, the first time someone in the daemon writes
  `Box<dyn Plugin>`, in a file far away from `traits.rs`. There is no early
  warning for this.
- **A `where Self: Sized` method is allowed on a dyn compatible trait and
  silently uncallable through the object.** That is the escape hatch for exactly
  one convenient method, and it is a trap because the definition looks fine.
- **`Sync` is what makes `&self` usable inside a `Send` future.** I expected
  `Send` on the plugin to matter; `Sync` does. Drop `Sync` from the supertraits
  and every `async move` block that captures `route` stops being `Send`, so the
  registry's `invoke` stops compiling even though the plugin type itself is
  fine.

## Where type erasure forced itself, and what it cost

The seam erases three things, and each one surfaces somewhere else.
**The payload types.** `invoke` takes `Payload` (bytes) and a `&str`. Whoever
calls it with a concrete request type gets the type back only if they encoded it
themselves, so the host needs its own declaration of every payload type:
`types.rs` is the host's copy of all fifteen payload types (one of which is
deliberately mismatched), and nothing links them to the plugin's copies. `MismatchedEchoRequest` (field `text`
where `tool.echo` wants `message`) compiled cleanly, went through the typed
facade, and was caught only by schema validation at runtime:

```
invalid argument: capability tool.echo: the request violates its published schema: "message" is a required property
```

So a typed method family buys ergonomics at the call site and a human readable
runtime error, not a compile time guarantee against the plugin. If the daemon
wants the compile time guarantee, the payload types have to be generated from
the same contract as the schema, which is a codegen decision the prototype did
not test.

**Any generic method on the seam.** `fn invoke<T: Serialize>(...)` is not
possible (probe error 5), so the typed layer cannot be a blanket wrapper around
the seam. It has to sit above it, per capability, hand written. In the prototype
each typed method is three lines (build the handle, encode the request, decode
the response) plus the handle's construction, and all of `typed.rs` (the
generic handle, the kind check, and both facades) is 132 code lines for seven
capabilities. The typing survives fully at the *edges* (the plugin author's
method family, the host's method family) and not at all in the middle.

**The kinds.** `CapabilityKind` survives as a data field on the descriptor, not
as a type. The prototype recovers per kind ergonomics in the facade: `Toolset`
builds handles only for `Tool`, `ProviderSet` only for `Provider`, and asking for
the wrong one fails with

```
unsupported: capability provider.complete is a Provider, not a Tool
```

That check is only possible because the descriptor still carries `kind`. Drop
`kind` from the descriptor and no typed facade can recover it, because the
routing table is keyed by name and the seam has no per kind method.

**Errors.** Everything erases to one enum. The facade cannot return a typed
error, and there is no field for "which plugin", so attribution is written into
the message string. `internal: plugin proto-plugin-cdylib returned a response for
capability tool.bad_response that violates its published schema: 42 is not of
type "string"` is readable and is not machine checkable.

**What survives intact:** capability names, kinds, versions, and the schemas.
Those are the descriptor, and the descriptor is the only real interface between
the daemon and a plugin.

## What validation costs per call

Microbenchmark loops in the host binary, no warm up discipline. Release numbers
are the honest ones; debug numbers are roughly an order of magnitude larger,
which matters because a host that measures in `dev` will get a wrong answer.
Three release runs:

| Measurement | Release | Debug |
| --- | --- | --- |
| schema compile, per schema | 4 to 6 us | 27 to 32 us |
| request validation, small object (1 string field) | 97 to 151 ns | 1057 to 1644 ns |
| response validation, small object (2 fields) | 148 to 248 ns | 1629 to 1849 ns |
| **both directions, per routed call** | **245 to 400 ns** | **2686 to 3493 ns** |
| serde encode plus parse of the same payload (the baseline validation sits on) | 193 to 220 ns | 1613 to 1709 ns |
| `registry.invoke` end to end, native in process plugin | 415 to 641 ns | 4836 to 4905 ns |
| `registry.invoke` end to end, cdylib through the C table | 622 to 916 ns | 5165 to 5488 ns |
| validation share of the call | 59 to 67% native, 39 to 46% cdylib | 56 to 71% native, 52 to 64% cdylib |

The important number is not the nanoseconds, it is the **share**: validating
both directions is roughly half of every routed call, and it is *larger* than the
serde work it is layered on because the payload is parsed into a `Value` for
validation and then parsed again by the plugin into its typed request. That
second parse is structural, not an implementation artifact: across the ABI the
only thing that can cross is bytes, so the registry cannot hand its parsed
`Value` down. Two mitigations exist and neither was prototyped: validate the
request only (halves the cost, and is the direction where the caller is
untrusted anyway), or move the response check behind a debug flag.

Load time is cheap: compiling the two schemas per capability is a fraction of a
millisecond for seven capabilities. Worst case the compile cost scales with
capability count, so a thousand capability plugin would be a few hundred
milliseconds at load, which is a startup budget question and not a per call one.

One good surprise: `jsonschema::Validator` is `Send + Sync` (inferred from the
fact that the registry's `invoke` future compiled as `Send` while capturing
`&Route`, which requires `Route: Sync`), so a route can be captured by a `Send`
future and the response can be validated in place after the await. If it were
not, response validation would have to happen outside the future, and a
`Box<dyn Plugin>` seam and an async registry would be much harder to combine.

## Assumptions made, and how hard each would be to flip

The brief said to assume sensible answers and write them down. Each one below is
implemented at exactly one place named in the table.

| # | Assumption | Where | Flip difficulty |
| --- | --- | --- | --- |
| 1 | Capability names are globally unique, and a duplicate name at registration fails the load. Nothing is committed on failure, so the routing table is unchanged. | `registry.rs::register` (stage then commit) | **Low** for the behaviour, **medium** in the real codebase: namespacing would change the routing key and every published descriptor name, so the agent core's tool invocation path and whatever advertises capabilities to a model both move. |
| 2 | JSON Schema 2020-12 generated from Rust types (a `schemars` style derive) is the schema form, and the schemas travel *inside* the descriptor rather than being referenced. | `types.rs::schema_of`, `proto-plugin-cdylib::descriptor` | **Low** to change the dialect if the generator can emit it (`schemars` 1.x can only emit 2020-12 today, so a dialect flip means a generator flip). **Medium** to flip to a reference string plus a schema store: it needs a store, a resolution step at load, and a decision about who owns the reference namespace. |
| 3 | Native in process plugins are in scope alongside ABI loaded ones. | `native.rs` | **Low**. If they were out of scope the seam still exists; only the second impl disappears. This assumption bought the whole comparison, and it is the one I would keep regardless. |
| 4 | The registry validates both directions, and a response violating its own published schema is an `Internal` error attributed to the plugin. | `registry.rs::invoke` | **Low** to drop or gate the response direction. **Medium** to make the attribution structured: the error enum has no plugin field, and adding one either forks the ABI status space or invents a host side error type that the daemon has to translate. |
| 5 | The ABI adapter uses the ABI's inline completion path (`Ok` plus an out buffer). | `abi_adapter.rs::invoke_inline` | **Medium to high**, and this is the biggest simplification in the prototype. The landed trait path always returns `Pending` plus a `complete` callback, so a faithful adapter needs a pending slot registry, a completion callback that releases the plugin buffer, a bounded wait, and a future that does not resolve on its first poll. The registry shape does not change; the adapter does, and the prototype therefore proves the registry, not the async plumbing. |
| 6 | Version gating is "advertised version <= supported version", with a per capability override map and a default. | `registry.rs::VersionPolicy` | **Low**. Exact match gating is a one line change. |
| 7 | `cancel` and `shutdown` are synchronous seam methods. | `seam.rs::Plugin` | **Low**. Neither produces a payload, so making them async would only widen the boxed future surface. |
| 8 | The registry is built once at load and is immutable afterwards, so it is shared behind an `Arc` with no lock. | `registry.rs`, `main.rs` | **Low** for the shape. **Unproven**: the prototype is single threaded and never exercises concurrent `invoke`; the `Sync` bounds compiled, which is necessary and not sufficient. |
| 9 | A capability the version gate refuses is simply not routed, and the refusal is recorded in a list. | `registry.rs::refused` | **Low**, but see the gap below: a caller cannot tell "never existed" from "existed and was refused" by the error alone. |

## Where the design fought back or surprised me

1. **`capabilities(&self)` has no error channel, and the ABI adapter needs one.**
   Listing capabilities across the ABI is a real FFI call that allocates a
   plugin buffer and can fail. The seam's `capabilities()` returns
   `Vec<CapabilityDescriptor>`, so a failed listing registers a plugin with zero
   capabilities, silently, at load. The prototype returns an empty `Vec` on
   failure, which is the wrong shape and I left it because fixing it means
   changing the trait. In the real codebase `capabilities()` needs a `Result`,
   or registration needs a separate fallible discovery step.
2. **Registering a plugin has a side effect on the plugin.** `capabilities()` is
   not a pure read: the ABI adapter allocates and releases a buffer inside it, so
   a routing table build is an FFI round trip. That is fine, and it means the
   registry cannot be rebuilt or queried from a context where calling into a
   plugin is not allowed.
3. **Global name uniqueness means one plugin can squat another's name.** A buggy
   or hostile plugin that claims `tool.echo` makes the *second* plugin's load
   fail. The failure mode is denial of a real plugin's registration, decided by
   whichever plugin loaded first. The prototype proves the check works; it does
   not answer whether failing the load is the right response, as opposed to
   quarantining the offender.
4. **A refused capability is indistinguishable from a typo.** Version gating
   removes the route, so `invoke("tool.future")` returns `NotFound`, the same as
   `invoke("tool.nope")`. The prototype keeps a `refused()` list, but nothing
   consults it on the call path. D10 wants the host to refuse a capability, and
   the agent core will most likely want to say "this tool exists but is too new
   for this daemon", which needs the refusal to be queryable by name.
5. **A response violation has to be attributed by string.** There is no error
   variant that can carry "which plugin", so the registry writes the plugin name
   into the message. This is the place where the ABI's frozen status space is
   most obviously too small for the host side registry.
6. **Embedding schemas inflates capability discovery by an order of magnitude.**
   The six capability `list_capabilities` payload is 2955 bytes, 492 bytes per
   capability, because two schema documents travel with each descriptor. The
   landed descriptor's `schema: String` reference exists for a reason. This is
   not a per call cost (discovery happens at load) but it is a payload that
   crosses the ABI and that anything storing or logging descriptors will carry.
7. **Validation parses the payload that the plugin parses again.** Covered
   above; it is the single largest cost in the routed call and it is structural.
8. **The debug and release gap is large enough to mislead.** 245 to 400
   nanoseconds versus 2.7 to 3.5 microseconds for the same validation, roughly
   an order of magnitude apart. Any daemon benchmark of this seam that runs in
   the `dev` profile will conclude validation is far more expensive than it is.

## Total line count per crate

Non blank, non `//` lines (raw counts in parentheses), same measurement the two
earlier plugin spikes used:

| Crate | File | Code lines (raw) |
| --- | --- | --- |
| `proto-abi` | `lib.rs` | 172 (225) |
| `proto-plugin-cdylib` | `lib.rs` | 292 (366) |
| `proto-host` | `abi_adapter.rs` | 273 (371) |
| | `bench.rs` | 93 (112) |
| | `lib.rs` | 8 (28) |
| | `main.rs` | 356 (410) |
| | `native.rs` | 134 (172) |
| | `registry.rs` | 256 (341) |
| | `seam.rs` | 121 (213) |
| | `typed.rs` | 132 (175) |
| | `types.rs` | 78 (101) |
| | **`proto-host` total** | **1451 (1923)** |
| | **all three crates** | **1915 (2514)** |
| | `object_safety_probe.rs` (not a workspace member) | (88) |

For scale against the landed surface: the prior authoring spike measured 833
code lines (1102 raw) for the whole translation in both directions, against a 33
line trait and a 51 line plugin implementation. This prototype adds the
registry, validation, gating, the typed facades, and the benchmark on top of a
seam that is 121 code lines, and the registry is 256 of those 1915. The registry
logic is not where the size is; the adapters and the host side scaffolding are.

## What the prototype did not prove

Stated plainly, because the interesting risks are all in here:

- **The pending path.** Progress, deferred completion, events, and the
  `complete` callback are installed as no-op stubs and never exercised. The
  cdylib answers inline. A faithful ABI adapter would always return `Pending`
  for the trait path, and nothing here measures what that does to the seam.
- **Cancellation across the ABI.** The cdylib has nothing pending, so `cancel`
  only ever returns `not_found`. The seam's `cancel` signature exists and is
  untested.
- **Concurrency.** Everything runs on one thread, one call at a time. The
  `Send + Sync` bounds compile, which is necessary and not sufficient. Whether
  `Arc<PluginRegistry>` behaves under the daemon's task pool is unproven.
- **Unloading.** The `Library` is dropped at process exit, after the registry
  was shut down. The real order (shutdown, join workers whose callbacks may still
  arrive, then unload) is not implemented or tested. The prototype would leak
  or crash if a plugin had a live worker at drop time.
- **Schema evolution.** No test that an old schema with a new plugin, or a new
  schema with an old host, degrades sanely.
- **Large or nested payloads.** Only a 1 field request and a 2 field response
  were measured, and no schema with `$ref` or `$defs` was compiled, so the
  `jsonschema` referencing path (which the prototype disabled with
  `default-features = false`) is untouched.
- **The reference string alternative.** Assumption 2's other branch (descriptor
  holds a reference, a store holds the schema) was not built, so its cost is
  unknown.
- **Codegen.** Nothing links the host's payload types to the plugin's, which is
  the actual fix for the type erasure finding.

## What would fail in the real codebase

Specific constraints from the landed surface, and what the prototype says about
each:

1. **The `authoring` feature gate and the dependency direction.** The landed
   `plugin-contract` keeps the authoring surface behind a non default feature,
   keeps serde out of the C ABI surface, and carries no async runtime, on purpose
   (`plugin-contract/Cargo.toml`, D3). The registry as prototyped is
   serde/JSON shaped end to end: descriptors carry `serde_json::Value` schemas,
   validation is `jsonschema`, and `Payload` is bytes that are always JSON in
   practice. That belongs in a crate that already depends on serde, so
   `plugin-runtime` (the host side crate) is the natural home, not
   `plugin-contract`. The prototype did not exercise a feature gate at all, so
   this is a placement recommendation rather than a result.
2. **The landed ABI imposes the two direction validation cost.** Because the ABI
   is bytes shaped, the registry cannot pass a parsed value to the plugin.
   Validation is therefore a second parse of the same payload, which is where
   half the per call cost goes. Making validation cheaper means changing what
   crosses the ABI, which is a major version question.
3. **The landed `PluginError` and the ABI's frozen status space are too small for
   the registry's new failure modes.** "The plugin advertised a capability whose
   response does not match its schema", "the capability was refused by version
   gating", and "the capability was never registered" all have to be expressed as
   `Internal`, `NotFound`, or `Unsupported` plus a string. If the daemon needs to
   act on these differently (retry, downgrade, tell the model), it needs either a
   new status member or a host side error type above the ABI, and that is an ADR
   level decision the prototype surfaces but does not settle.
4. **The capability descriptor changes shape.** `schema: String` becomes two
   embedded schema documents. `plugin-runtime`, the daemon, and anything that
   advertises tools to the model in `metis-agent-core` all read descriptors, so
   this is one wire shape change with three consumers.
5. **The daemon's async model is the untested half.** The seam compiles with a
   `Send` boxed future and a `Sync` registry, which is what a `tokio` based
   daemon needs. But the ABI adapter's completion path is callback driven and the
   prototype stubbed it, so nothing here proves that a `Box<dyn Plugin>` call can
   be awaited without blocking the daemon's executor, or that a plugin thread's
   callback can wake a daemon task safely. That is the risk I would retire next,
   and it is the one the landed `HostAdapter` already solves, so the honest
   reading is that the registry can probably borrow that adapter rather than
   inventing a second one.
6. **The daemon will want the typed facades, and they need codegen.** Seven
   capabilities in the prototype cost fifteen hand written host side payload
   types and seven three line wrappers, and the mismatch demo shows what happens
   without a link: silent drift until runtime. If the daemon calls tools through
   typed facades, the payload types have to come from the same contract that
   produces the schemas.

## Reproduction

```sh
cd /home/patric/Projects/metis-agent

# 1. build the whole workspace. This MUST precede any run: `cargo run -p
#    proto-host` does not rebuild the cdylib member.
devenv shell -- cargo build --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml

# 2. drive every behaviour. One PASS/FAIL line per check, a summary, non zero exit
#    on failure.
devenv shell -- cargo run -q --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml -p proto-host

# 3. the release numbers for the benchmark section
devenv shell -- cargo build --release --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml
devenv shell -- /tmp/metis-plugin-registry-prototype/target/release/proto-host

# 4. the cdylib exports exactly one dynamic symbol
devenv shell -- nm -D --defined-only /tmp/metis-plugin-registry-prototype/target/debug/libproto_plugin_cdylib.so

# 5. the object safety diagnostics, verbatim
#    (regenerates the captured object_safety_probe.stderr.txt)
devenv shell -- bash -lc 'cd /tmp/metis-plugin-registry-prototype \
  && rustc --edition 2021 --crate-type lib object_safety_probe.rs -o /tmp/probe.rlib \
     2> object_safety_probe.stderr.txt'

# 6. the line counts quoted above (no devenv needed, the tools are on the bare PATH)
cd /tmp/metis-plugin-registry-prototype
for f in $(find crates -name '*.rs' | sort); do
  printf "%-34s raw=%4s code=%4s\n" "$f" "$(wc -l < "$f")" \
    "$(awk '/^[[:space:]]*$/{next} /^[[:space:]]*\/\//{next} {n++} END{print n+0}' "$f")"
done
```
