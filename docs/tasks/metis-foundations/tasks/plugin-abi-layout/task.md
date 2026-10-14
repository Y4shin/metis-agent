---
type: task
subtype: grilling
title: Source of truth for the plugin C ABI structs
status: deprecated
workflow_state: done
blocked_by: []
mode: human
---

## Decision to settle

Where the plugin C ABI is authored first: `repr(C)` Rust in
`crates/plugin-contract`, a C header, or a neutral schema that both derive
from. The follow-on choices ride along with it: where the ABI version constant
lives, how the versioned entry point is named, and which layout and ownership
tests are required in CI.

## Parent decisions

Already settled and not to be reopened:

- `metis-foundations` owns `plugin-contract`; `metis-plugins` consumes it and
  owns the host side; `metis-plugin-sdk` wraps it for Rust authors.
- `docs/rough-plan.md` ("Native plugin design", "Async boundary") fixes: one
  versioned exported entry point returning a function table; explicit instance
  creation and destruction; capability discovery and initialisation; serialised
  request and response payloads, initially JSON bytes; explicit ownership and
  release for plugin-allocated buffers; host callbacks for completion,
  progress, and plugin-originated events; no Rust trait objects, futures,
  strings, vectors, Tokio handles, or channels across the boundary; distinct
  bundle, ABI, and plugin versions.

## Choices already known

- (a) `repr(C)` Rust as the source of truth, with the C header generated
  (`cbindgen` or equivalent).
- (b) A C header as the source of truth, with Rust bindings generated
  (`bindgen`) or handwritten.
- (c) A neutral schema that both derive from.
- Other languages must be able to implement the ABI directly, so whatever is
  chosen has to produce a readable, versioned header.
- Completion may precede the return from start, and cancellation is an
  explicit request, so the layout has to carry request IDs and callback
  registration without Rust types.

## Decisions recorded

**D0, provisional status.** The user does not treat the ABI shape decisions below
as informed and has little C ABI experience. They are pinned now so
implementation can start, and they may be revised during implementation when
evidence appears, updating this task and the map rather than returning to
Wayfinder for a new planning task.

**D1, primitives.** The ABI is a small primitive set, not a C rendering of the
Rust trait: a versioned entry point returning a function table with instance
creation and destruction, capability listing, one generic
`invoke(instance, capability, request_id, payload, host_api)`, `cancel`,
`release_buffer`, and `shutdown`; plus a host table with `complete`,
`progress`, `emit_event`, and `log`. Everything else is payload.

**D2, source of truth.** `repr(C)` Rust in `crates/plugin-contract` is the
source of truth, with a `cbindgen`-generated C header committed and a CI check
that regeneration produces no diff.

**D3, trait and translation placement.** `crates/plugin-contract` holds the
primitives, the payload types, the authoring Rust trait, and the export and
host adapters. The authoring side is behind a feature gate, so `plugin-runtime`
depends on the primitives and payload types alone. `metis-plugin-sdk`
re-exports the trait and adapters and owns the async executor, the
serialisation helpers, the core plugins, and the CLI.

**D4, payload envelope.** Payloads are bytes whose first byte is a header: the
low four bits carry the payload format version, bit 4 selects the codec
(0 for UTF-8 JSON, 1 for MessagePack), and bits 5 to 7 are reserved and zero.
The ABI length field is unchanged; the header is the first byte of the buffer.
Only JSON is used at first, and the codec bit exists so MessagePack can arrive
without a format break.

**D5, compatibility across compiled versions.** Evolution within an ABI major
version is additive: unknown fields are ignored on deserialisation, new fields
are optional, and capability descriptors carry a per-capability version so the
host can refuse one capability instead of failing the whole plugin. Backward
compatibility for older plugins on newer daemons is a standing goal to preserve
as far as possible.

**D6, ownership and release.** Each side releases only what it allocated.
Plugin-allocated buffers (results, capability descriptors, events) go back
through `release_buffer` in the plugin table; host-allocated buffers are freed
by the host after the call returns. The host never frees plugin memory.
Instance handles are opaque, plugin-owned, and never dereferenced by the host.

**D7, errors and panics.** A frozen integer status enum (ok, invalid argument,
not found, permission denied, cancelled, internal, unsupported) plus an optional
UTF-8 detail buffer the receiver releases. Every exported adapter wraps its body
in `catch_unwind`, so a panic becomes an internal-error status rather than an
unwind across the boundary.

**D8, versioning mechanism.** The entry point carries its major version in the
symbol name (`metis_plugin_entry_v1`). Within a major version the function table
is append-only and carries a minor or size field, so a newer host reads an older
plugin's shorter table and an older host reads the prefix it knows. Only a
differing major fails, at symbol resolution and before any struct is
interpreted. Backward compatibility for older plugins on newer daemons is
best effort within a major version.

**D9, capability model.** Capability descriptors are a payload listing entries
with a name, a `kind`, a version, and a schema reference, and `invoke` addresses
an entry by name. V1 kinds are `tool` and `provider`; a new kind never moves the
ABI.

**D10, payload version versus capability version.** The payload header version
tracks the payload byte format, and the capability descriptor version tracks the
capability's semantics. The host refuses a capability whose version it does not
support rather than failing the plugin.

**D11, async shape.** `invoke` may return the result inline when it finished
synchronously, and otherwise returns pending, with the result arriving through
host `complete`. Pending state is registered before the plugin is called.

**D12, callback rules.** Callbacks may come from any plugin thread; the host is
thread-safe and may queue them. Callbacks after a successful `cancel` are
ignored. Callbacks after `shutdown` returns are logged and ignored as a protocol
violation.

**D13, header and layout verification.** A `cbindgen`-generated header is
committed. Rust `const` assertions check the size, alignment, and field offsets
of every table, and a C translation unit compiled in CI includes the header and
static-asserts the same numbers.

**D14, validation by prototype.** Two throwaway prototypes validate the
strategy: the ABI mechanics spike and the authoring trait spike. Either may be
abandoned with an inconclusive result if it becomes too complex or long. An
inconclusive result blocks neither the remaining tasks nor the spec.

## Spike amendments (plugin-abi-mechanics-spike)

The mechanics spike ran to a conclusive result: 39 runtime checks passed, the
host exited zero, both layout-agreement tests passed, `cbindgen --check` reported
no diff, and the cdylib exports exactly one symbol. The following amendments are
taken from it. D0's provisional status covers them.

- **D1**: the host table needs a `user_data` pointer, without which the
  callbacks cannot work. Passing the host table to `invoke` per call while the
  instance also retains it works and costs nothing.
- **D2**: cbindgen needs `[export] include` to list every ABI type, because a
  crate whose surface is a table of function pointers exports no functions to
  seed its dependency graph. cbindgen 0.29.4 does not translate
  `Option<NamedFnAlias>`, which constrains how the structs are authored.
- **D4**: codec 1 returns `unsupported` while a bad version or non-zero reserved
  bits returns `invalid_argument`, so the two failures stay distinguishable.
- **D6**: releasing what you allocated also applies when the payload is ignored.
  A late completion dropped because of a successful cancel still produced one
  release.
- **D7, revised**: `pending` joins the frozen status list, since no member could
  express D11's deferred outcome. The spike used `Pending = 7`. The alternatives
  are an out parameter carrying the completion shape, which keeps the status
  space intact, and overloading `ok` with a null out buffer, which is rejected
  because it makes "ok with no result" ambiguous. `catch_unwind` is required on
  host callbacks too: a callback is itself an `extern "C"` frame reached from
  plugin code, so a panic inside it unwinds into plugin frames.
- **D8**: `size` must be the table's first field so it is readable before
  anything else; the fields a host knows form a prefix of the layout, so entries
  can only ever be appended; and the entry point signature is frozen within a
  major version, because the symbol name carries only the major. The spike used
  an argument-free entry point for that reason.
- **D12, revised**: after a successful cancel the host also retires its own
  pending entry, because no completion is guaranteed and ignoring late callbacks
  alone leaks one record per cancelled request. The rules state both halves of
  the drop path: the plugin may suppress its completion after a successful
  cancel, and the host must drop a completion that arrives anyway, since cancel
  and completion race. Only the second half is safe alone.
- **D13**: the literal sizes are 64-bit specific, so assertions must be written
  through `size_of::<usize>()` or run per target, and the C side needs at least
  `-std=c11`. The spike also exports the C-side measurements and compares them
  with Rust, which catches a stale literal as well as layout drift.
- **D14**: conclusive. The strategy stands and does not need abandoning.

Two harness findings belong in the spec rather than the decisions: the cdylib
must be built before the host runs, because `cargo run -p host` does not rebuild
the cdylib member and the host would load a stale library; and the host needs a
bounded wait with a timeout, since an unbounded wait on a completion that never
arrives hangs forever.

### Authoring trait spike (plugin-authoring-trait-spike)

Also conclusive. A trait with several capability methods and a cancellation
method dispatches through one generic `invoke` addressed by capability name, the
always-pending async path works end to end, and the trait module imports no ABI
type. Translation size: about 780 code lines, of which only about 90 know about
the trait (33-line trait, 51-line plugin, and 6 lines of ABI wiring through a
`const fn`), so no macro is justified at this size.

Two decisions `plugin-contract` must settle before pinning the authoring surface:

- Cancellation: the spike used a poll flag with an adapter-rebuilt context, which
  has no waker and rebuilds the per-request context, so a plugin blocked in I/O
  will not notice cancellation promptly. The alternative is a cancellation future
  or signal.
- Inline completion: an async trait method cannot hand back a synchronous result,
  so the adapter always returned `Pending`. Recovering D11's inline shape means
  polling the future once under `invoke` and falling back, which changes the cost
  model.

Contract gaps the trait exposed, to settle in the spec:

- A lifecycle hook is missing. The trait needs `Default` for construction, so
  construction cannot fail or take configuration, and there is no `started` or
  `stopped` hook. `emit_event` also had no route through the trait, so the spike
  emitted zero events where the mechanics spike used it.
- Descriptor names and dispatch names are separate declarations with no
  consistency check. Record a single `(name, kind, version, handler)` table per
  plugin, or a creation-time check that every descriptor has an arm.
- The error type mirrors the status space minus `ok` and `pending`, and
  `Cancelled` carries control flow: returning it after a successful cancel
  suppresses the completion. That rule belongs in the contract.
- The authoring request and response types are serde types, so the plugin author
  surface is a serialisation surface and D3's placement of serialisation is
  load-bearing.
- The trait forces `Send + Sync + 'static` on the plugin and `+ Send` on every
  capability future, because the worker hop moves the future. This constrains
  plugin authors and belongs on the trait documentation.

**D15, authoring surface decisions.** Both questions the trait spike left open are
settled. The authoring trait exposes a cancellation token on the per-request
`Call` context, so a plugin can select on it and a call blocked in I/O is dropped
promptly, with the poll flag kept as a convenience. The trait adapter always
returns pending and completes through the host callback, which is what the spike
measured; the ABI keeps its inline path for plugins that implement the ABI
directly, so nothing is lost and the trait path stays uniform.

## Alternatives considered

- Mirroring the Rust trait in C. Rejected by the user's design principle: the
ABI would inherit trait churn and turn every trait refactor into an ABI risk.
- A C header as the source of truth, with generated or handwritten Rust.
Rejected in favour of `repr(C)` Rust plus a generated header, though noted as
defensible at this size.
- A neutral schema with codegen written here. Rejected: no third consumer yet.
- Payload encoding defined as a document under `contracts/`. Rejected: the
network contract is a different boundary, and payload types in
`plugin-contract` are the contract.
- One shared allocator, or caller-provided buffers with size negotiation.
Rejected in favour of each side releasing only what it allocated.
- Always-async `invoke`. Rejected in favour of inline completion with a pending
path.
- Typed C entries per capability kind. Rejected in favour of one generic
`invoke`.
- Closing the ABI to tools in V1. Rejected: V1 carries `tool` and `provider`.
- Separate version registries for payloads and capabilities. Rejected in favour
of one idea used in two places.
- A handwritten header with assertions. Rejected in favour of a generated header
plus a C-side compile check.

## Dependent-task implications

- `metis-plugin-sdk` re-exports the trait and adapters and owns the async
executor, serialisation helpers, core plugins, and CLI. Its map has been
updated.
- `metis-plugins` consumes the ABI. Capability discovery now covers `tool` and
`provider`, and descriptors carry versions, so the host gates per capability.
- `metis-daemon` adapts plugin capabilities into harness tools, providers, and
hooks.
- `metis-agent-core` owns the provider contract; provider capabilities must be
expressible through capability payloads.
- The two prototype tasks must be completed or abandoned before the spec pins
the ABI.
- `cbindgen` and the C compile check join CI.
- The generated header is the artifact non-Rust authors implement against.

## Remaining fog

- Whether the append-only table needs a formal minor registry or documented
rules suffice.
- Payload size limits and allocation bounds.
- The exact descriptor fields beyond name, kind, version, and schema reference.
- Whether providers need callbacks beyond `complete`, `progress`, and
`emit_event`, for example token streaming.
- Windows `cdylib` loading specifics beyond the per-side release rule, such as
CRT and allocator boundaries.
- Whether a JSON Schema generated from the payload types is published for
non-Rust authors.
