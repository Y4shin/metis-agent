---
type: deviation report
title: Deviation report for plugin-authoring-trait
status: stable
---

## Deviation report: plugin-authoring-trait

Reviewed: `main..ticket/plugin-authoring-trait` (18 files, +2957/-509) plus the
source files under `crates/plugin-contract/src/authoring/`, the rewritten
`crates/plugin-fixture/src/lib.rs`, and the harness. The landing branch
`task/plugin-authoring-trait` carried no commits at review time, so the working
branch was diffed against `main`.

### API surface changes

**The trait's capability vocabulary is fixed at four role-named arms, and the
capability names come from associated constants.**
- **Planned:** `Plugin` carries "one async method per capability taking the
  deserialised request and the per-request context", the capability methods are
  parameterised by the plugin's own serde types, and the declaration and the
  dispatch arms are separate declarations compared at instance creation.
- **Actual:** `Plugin` declares exactly four arms, named for the role rather
  than the capability: `answer`, `deferred`, `cancellable`, and `fault`. Each is
  paired with an associated type (`Answer`, `Deferred`, `Cancellable`, `Fault`,
  each `: Shape`) and a capability-name constant (`ANSWER`, `DEFERRED`,
  `CANCELLABLE`, `FAULT`, every one defaulting to `""`, meaning "not
  implemented"). The fixture maps those constants to `tool.echo`,
  `tool.pending`, `tool.cancellable`, and `tool.panic`, and `capabilities()`
  builds its descriptor list from the same constants, so the declaration and the
  arms cannot disagree in the fixture; the creation check is exercised by a
  deliberate inconsistent test plugin instead.
- **Impact:** real but narrow. A plugin may implement fewer than four
  capabilities (an unused arm stays unnamed and answers `Unsupported`), but it
  cannot implement more: a fifth capability, or two capabilities of the same
  role (two plain `tool.*` arms that each just answer), needs a trait change,
  and this ticket forbids both the macro and the runtime dispatch method that
  would avoid one. `metis-plugin-sdk` re-exports this surface, so it is the one
  item worth re-deciding before the SDK pins author-facing API. The arch spec's
  "one async method per capability" holds only for a plugin with at most four
  capabilities, one per arm.

**The authoring module ships a wider public surface than the spec listed.**
- **Planned:** `Plugin`, `Call` (with the token, the poll convenience, and the
  `progress`/`emit_event`/`log` routes), `PluginError`, `table::<P>()`, the host
  adapter, and the minimal executor.
- **Actual:** additionally public in the same module:
  `CapabilityKind`, `CapabilityDescriptor`, `CapabilityList`, and `Shape`
  (the D9 declaration the adapter serialises); `Host` and `LogLevel` (the
  routes with no request to attach them to, needed by `started`/`stopped`);
  `CancellationToken` and `Cancelled` (the awaitable half of the token);
  `Accounting` and `accounting()`; `executor::{block_on, spawn, catch_panic,
  CatchPanic, Panic, JoinHandle}`; and `host_adapter::{Await, Completed,
  HostAdapter}` where `HostAdapter` exposes harness-shaped observation and
  plumbing (`api`, `attach`, `read_and_release`, `release_plugin_buffer`,
  `logs`, `saw_log`, `events`, `callback_panics`, `register`, `wait_timeout`).
- **Impact:** the SDK must choose what to re-export from a surface that mixes
  the author contract with host-side and test-support helpers. The observation
  and plumbing methods are public in a shipped crate, so they are a compatibility
  surface from now on unless the SDK hides them.

**Descriptor ownership moved into the crate.**
- **Planned:** the fixture's `payload` module was "the one place the payload
  shapes live", and the capability declaration lived on the adapter side.
- **Actual:** `crates/plugin-fixture/src/payload.rs` re-exports
  `CapabilityDescriptor`, `CapabilityKind`, and `CapabilityList` from
  `plugin_contract::authoring`, because the adapter is what serialises the
  declaration. The request, response, and progress bodies remain fixture-owned.
- **Impact:** for `plugin-compatibility`, the fixture's payload module is the
  place the bodies live but no longer the place the descriptor lives, which its
  own section assumes. The harness still parses through the fixture's re-export,
  so nothing downstream of the fixture changed.

**Shutdown and destroy semantics became observable contract.**
- **Planned:** the spec did not state what `shutdown` does to the instance
  handle.
- **Actual:** `shutdown` releases the instance, so a later `destroy_instance` on
  the same handle returns `NotFound` (ported from the landed fixture, which the
  harness already asserted), and `create_instance` clears a reused address from
  the released set so a second instance works.
- **Impact:** `plugin-runtime` inherits a slightly stronger host-side rule than
  the ABI text states. No ABI type, table entry, or layout changed.

**Matches the plan, recorded so it is not mistaken for drift.**
- `Plugin: Default + Send + Sync + 'static`, always-pending completion with the
  ABI's inline path kept for direct implementers, the cancellation token plus
  `is_cancelled()` poll convenience, the lifecycle pair, the event route, the
  descriptor/dispatch check at creation, and `PluginError` as the status space
  minus `Ok` and `Pending` with the mapping living in the adapter all land as
  specified. The trait module names no ABI type, and that is asserted by a test.
- The ABI is untouched: no change under `crates/plugin-contract/src/abi`, no
  payload rule change, no new public constant, so no `cbindgen.toml` `exclude`
  entry was needed and the header drift check still exits 0.

### Abstraction usage

- Used/was specified: **yes.** The adapter is the shape
  `spikes/plugin-authoring-trait` measured, ported rather than depended on: the
  `const fn table::<P>()` wiring with the entry point as the plugin's whole ABI
  footprint, dispatch by capability name, the pending registry plus slot/await
  host adapter, and the minimal `block_on`/`spawn` executor with its own waker.
  It builds on the landed `crate::abi` and `crate::payload` without adding an ABI
  type, and the rewritten fixture and harness keep the rebuild-first load, the
  bounded wait, and the `[ok]`/`[fail]`/`[result]` contract. No spike crate is
  depended on, and no macro was introduced.

### Out-of-scope changes

- `docs/testing.md` gained the authoring feature's run commands, which the
  effort's convention assigns to that file.
- The fixture gained a fixture-local bounded wait (`WaitFor` + `Timer`, 1.5s
  budget) and a public `Echo` payload shape, both needed to observe the
  cancelled-completion rule and the always-pending shape through the harness.
- Four harness checks were added and two renamed (`invoke.inline_result` to
  `invoke.always_pending_completion`, `panic.exported_function_is_internal` to
  `panic.capability_panics_into_an_internal_completion`); 41 checks before, 45
  after. The spec predicted that the always-pending move would shift
  expectations.
- `Cargo.lock` grew by two lines; `serde` and `serde_json` were already
  workspace dependencies, so the root manifest is unchanged, as is `devenv.nix`,
  CI, and the generated trees.
- Additions only: the fixture's ABI-direct implementation was replaced, the
  land `src/abi/mod.rs` was not modified, and no committed generated file was
  hand-edited.

### Ticket doc update needed?

**Yes.** Append to `## Implementation notes` (do not alter the acceptance
criteria):

1. The trait's four role-named arms (`answer`, `deferred`, `cancellable`,
   `fault`) with plugin-chosen capability-name constants, and the consequence
   that a plugin can express at most four capabilities, one per arm. This is the
   item to re-decide before `metis-plugin-sdk` pins author-facing API.
2. `PluginError`'s status mapping lives in `plugin_adapter`, not on the error
   type, which is what keeps `traits.rs` free of ABI types.
3. The allocation ledger moved into the adapter (`authoring::alloc`); the
   fixture reports it from its `stopped` hook as
   `FIXTURE_ACCOUNT allocs=... frees=... live=... violations=... suppressed=...`,
   which is what the harness's ownership checks read.
4. `shutdown` releases the instance (`destroy_instance` afterwards is
   `NotFound`), and `create_instance` clears a reused address so a second
   instance works.
5. The fixture's `payload.rs` re-exports the descriptor types from
   `plugin_contract::authoring`; the request, response, and progress bodies stay
   fixture-owned. `plugin-compatibility` should not assume the payload module
   owns the descriptor.
6. New public items beyond the spec's list: `Shape`, the three descriptor types,
   `CancellationToken`/`Cancelled`, `Host`/`LogLevel`, `Accounting`/
   `accounting()`, the `executor` helpers, and `HostAdapter` with its harness
   observation methods. No new public constant, so no `exclude` entry was
   needed.
7. Feature-unification caveat: the workspace `cargo test` compiles the authoring
   tests only because the fixture enables the feature; the feature-off path is
   compiled by `cargo test -p plugin-contract` alone. A
   `cargo check -p plugin-contract --no-default-features` CI step would make it
   explicit and was not added.
8. Test-harness detail worth keeping: the authoring unit tests leak a
   `table::<P>()` static and serialise behind one mutex, because the adapter's
   pending registry, worker list, and ledger are process-wide; an early parallel
   version SIGSEGV'd on a test-local table.
9. Residual risks: the fixture's 1.5s cancellation budget and the harness's
   250ms promptness bound have wide margins but are timing assumptions; nothing
   in the tree exercises the ABI's inline `invoke` result any more (recorded as
   accepted in the arch spec); and `HostAdapter::callback_panics` stays 0 because
   nothing in-tree panics inside the contract's own callback bodies.

### User attention needed?

**Yes**, one item, and it does not block landing:

1. The trait's fixed four-arm vocabulary with plugin-chosen names is the only
   place the landed surface is narrower than the plan. It constrains every
   plugin to at most four capabilities, one per arm, and adding a fifth is a
   trait change rather than a plugin choice. Since `metis-plugin-sdk` re-exports
   this surface, confirm the shape before that effort pins author-facing API.
   The alternatives the ticket's own constraints left open are a macro over a
   per-capability trait (rejected as unjustified at this size) or a runtime
   dispatch method on the trait (rejected by the "no second dispatch path"
   reading of D15).

Two further notes for the record rather than decisions: the ABI's inline
`invoke` result now has no in-tree exerciser, and the authoring module ships
harness-shaped public methods (`HostAdapter`'s observation and plumbing) that a
dependent effort will have to decide whether to re-export or hide.

### Verification evidence

Read from the implementation, not copied from the worker report:

- `crates/plugin-contract/Cargo.toml` declares `authoring =
  ["dep:serde", "dep:serde_json"]` with both dependencies `optional = true` and
  no `default`, and `src/lib.rs` mounts `pub mod authoring` behind that feature.
- `src/authoring/mod.rs` re-exports exactly the items listed under API surface
  changes; `grep` over `src/authoring/traits.rs` shows no `crate::abi` and no
  `Metis` identifier.
- `plugin_adapter::check_declaration` runs from `create_instance`, rejects a
  descriptor with no arm and two arms claiming one name, and
  `plugin_adapter::dispatch` returns `NotFound` for a name no arm answers.
- `crates/plugin-fixture/src/lib.rs` implements `Plugin` with the four `const`
  names, `static PLUGIN_TABLE_V1: MetisPluginTableV1 = table::<Fixture>()`, and
  one `metis_plugin_entry_v1`, with no buffer, status, callback, or thread in
  the author's file.
- Nothing under `crates/plugin-contract/src/abi`, the committed header, CI, or
  `devenv.nix` appears in the diff.
- The worker's gate numbers (14 new tests, 30 workspace tests, 45 harness checks,
  header drift clean) were not independently re-run here; the slice-verifier
  owns that, and this report's scope is the surface diff.
