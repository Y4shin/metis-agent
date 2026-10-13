---
type: ticket
subtype: feature
title: Plugin contract v1, futures-only ABI with routed authoring and host enforcement
status: stable
workflow_state: ready
blocked_by: []
size: xl
---

## What to build

The approved plugin contract, `docs/plugin-contract.md`, in the current
codebase. It replaces the delivered v1 ABI and the four-arm authoring trait,
fills the empty `plugin-runtime` host side, and proves the whole boundary with a
fixture and a C implementer.

The gathered evidence is `spikes/plugin-registry/FINDINGS4.md` and its
`round-3/` lane findings and logs, where every number was re-run by the parent.
The prototype is throwaway; this ticket lands the shape, not the prototype.

Naming: the contract document is called v1 because it is the first version this
project approves, and the ABI major version it defines is 2. Both names are used
deliberately.

## Acceptance criteria

**The ABI and the contract crate**

- [ ] The plugin table is a clean revision of the v1 table, not an append: `size`, `abi_major`,
      `abi_minor`, `create_instance`, `destroy_instance`, `list_capabilities`,
      `invoke_future`, `cancel`, `release_buffer`, `shutdown`,
      `service_complete`. `invoke` and the `complete` callback are deleted, not
      deprecated, and `metis_plugin_entry_v1` with `abi_major = 1` is the only
      exported entry symbol. The entry order changed while the symbol and major
      did not, so `abi_minor` becomes 1 and the loader refuses a minor it does
      not know, which refuses a stale pre-release plugin instead of reading it
      through the new entry order.
- [ ] The host table is `size`, `user_data`, `progress`, `emit_event`, `log`,
      `service_request`, with `complete` deleted.
- [ ] `MetisFutureHandle`, `MetisFutureVtable` (poll, drop), `MetisWaker` and
      `MetisWakerVtable` (clone, wake, wake_by_ref, drop) exist, all `repr(C)`,
      and every field is a pointer, an integer, or a function pointer.
- [ ] The two unread vtable `size` fields are deleted rather than read, and the
      residual is recorded in the crate docs: a wrong-size plugin vtable is that
      plugin's own bug and is not detectable at load.
- [ ] The payload envelope constants and the table presence predicates are
      exported into the generated C header, and every new public constant is
      added to the cbindgen exclude list, because cbindgen emits public
      constants regardless of the export list.
- [ ] Rust layout asserts and the C translation unit cover every new type,
      `scripts/regenerate-abi-header.sh --check` passes, and the header is
      regenerated rather than hand-edited.
- [ ] Panic containment wraps `poll`, `drop`, and every host callback, and a
      panic inside a future's `Drop` is contained into a terminal internal error
      with no path to `process::abort`. No dependency on `async-ffi` is
      introduced anywhere.
- [ ] The ownership rules of `docs/plugin-contract.md` section 11 hold in code
      and are stated in the crate docs: the ready buffer is plugin-allocated and
      freed through `release_buffer`; the request payload is host-owned and valid
      for the call only; a `service_request` payload is plugin-owned and freed by
      the host; a `service_complete` payload is host-owned and borrowed;
      `release_buffer` is reentrant inside `service_request`.

**The authoring surface**

- [ ] `Plugin` is `capabilities`, `cancel`, `started`, `stopped`, and one
      capability-name-routed `call` taking `Request { capability, body }` and
      returning `Response { body }`. The four role-named arms, their four
      capability-name consts and their four associated request types are
      deleted.
- [ ] The four-capability ceiling is gone: a fixture or test plugin advertising
      more than four capabilities dispatches all of them, asserted by a test.
- [ ] The authoring path no longer owns an executor: the primitives-side spawn
      seam, the bounded pool, the thread-per-call fallback and the host adapter's
      pending-slot machinery are deleted, and a capability is driven to
      completion with no plugin thread spawned, asserted by a thread-count check.
- [ ] The authoring module keeps the feature gate, and the crate still builds
      with `--no-default-features`, with the crate alone compiled that way in CI
      because the fixture otherwise turns the feature on.

**The host side**

- [ ] `plugin-runtime` stops being an empty file and gains the host driver and
      registry: cdylib loading, routing by capability name, presence decided by
      table size, per-capability version gating, duplicate-name refusal, enable
      and disable, unregister, both-direction schema validation, and a
      launch-and-await API over the future seam.
- [ ] At least eight concurrent plugin futures run on one host runtime with the
      daemon as the only executor, asserted by a check that the plugin thread
      count stayed at zero.
- [ ] Quarantine and unload are real: an abandoned blocking poll quarantines the
      instance so no new futures are accepted and the library stays resident; a
      late poll return reclaims the context box and the waker clone exactly once;
      unload happens only when nothing is inside plugin code and every future and
      waker clone has been dropped; no fixed sleep is used; the ordering is
      observable in the harness output.

**Proof**

- [ ] The fixture plugin is rewritten against the routed trait, and the ABI
      harness drives the futures path end to end, prints one PASS or FAIL line
      per check, flushes stdout per line, builds the cdylib before the harness
      runs, and exits non-zero on any failure.
- [ ] A C implementer of the revised v1 table exists and is compiled in CI with
      `-std=c11 -Wall -Wextra -Werror` under both gcc and clang, and the harness
      drives it end to end: the ready path, a host-answered timer, cancel, and a
      drop with its service reply in flight, with the C-side allocation ledger
      reporting zero live buffers, zero double frees and a balanced waker count
      after unload.
- [ ] Each hazard has an observable check: panic in poll, panic in drop, panic in
      a host callback, double drop, drop after unload, wake after drop, a poll
      that blocks, a cancel that resolves as `Cancelled`, and a plugin that
      misses the cancel deadline.

**Records**

- [ ] `arch-spec.md` is reconciled: it no longer calls `plugin-authoring-trait`
      pending, it describes the revised v1 surface as landed, and it matches the code.
- [ ] An ADR under `docs/adr/` records the contract-level decision: futures-only
      ABI, the routed authoring surface, no `async-ffi`, and the quarantine
      unload rule.

## Order of work

The ticket is deliberately one ticket and deliberately `xl`. Layer it so each
step stays green: contract crate and header, then the routed trait and the
fixture rewrite, then the host driver and registry, then quarantine and reclaim,
then the C implementer and the hazard checks, then the records.

## Decisions this ticket makes

The contract leaves five points open on purpose. This ticket resolves them as
follows, and an implementer should not silently reopen them:

- The entry symbol stays `metis_plugin_entry_v1` and the ABI major stays 1,
  because this revision lands before the first release and no plugin exists in the
  wild to stay compatible with. The entry order inside the table does change, so
  `abi_minor` becomes 1 and the loader refuses a minor it does not know until
  `plugin-compatibility` relaxes that to additive tolerance. Additive change
  inside major 1 is still append-only by `size`.
- The two unread vtable `size` fields are deleted, with the residual above.
- The control plane stays workable by keeping `list_capabilities` synchronous and
  on the table, while the per-call `metis.*` control capabilities (configure,
  counters) move onto `invoke_future`. `create_instance` therefore never needs a
  schema fetch mid-call.
- Typed facades are not in this ticket. The erased `call` plus published schemas
  is what lands; the generated typed author experience belongs to the SDK effort.
- Reclaim lands on the poll return, driven by the caller, which is what the
  prototype proved. A daemon-side reaper thread stays `metis-daemon` work.

## Out of scope

- Typed facade generation, serialisation helpers, core plugins, and the plugin
  CLI. These belong to `metis-plugin-sdk`, which also creates its own crate.
- Mapping plugin capabilities onto harness tools, providers and hooks, per-plugin
  configuration delivery, and the daemon's crash containment policy. These belong
  to `metis-daemon`.
- Bundle extraction, signature checks, platform selection, and packaging.
- Provider implementations, and the workflow crate.
- Schema evolution beyond additive change, and payload size or nesting bounds.

## Blocked by

- None. `plugin-abi-core` and `plugin-authoring-trait` are done, and this ticket
  revises their deliverables rather than depending on them.

## Implementation notes

- `plugin-compatibility` now depends on this ticket, because version tolerance is
  a property of the table this ticket replaces. Its prefix-tolerance criterion
  was removed there and the rest was restated for the revised v1 table.
- The three effort maps that carried the superseded decisions are amended in the
  same change, and `docs/plugin-contract.md` section 16 lists what was
  superseded.
- Prototype evidence worth reading before starting: the deleted-surface table and
  the contract sketch in `round-3/shape/FINDINGS-shape.md`, the reclaim ledger in
  `round-3/reclaim/FINDINGS-reclaim.md`, the ownership clause and the C plugin in
  `round-3/ownership/`, and the drop-panic probe in
  `round-3/asyncffi/FINDINGS-async-ffi.md`.
- The prototype's measured costs are the baseline to compare against: release
  cdylib 1,554,520 bytes with the legacy path and 924,328 without, ready-on-first
  poll 947 to 1288 ns, full host-polled path p50 6893 to 8165 ns, and the naive
  launch-and-await bridge costing 1.7 to 2.2 us per call that a real executor
  should not pay.

Landed on `task/plugin-contract-v1` as the merge commit `83c0660`
("ticket(plugin-contract-v1): Plugin contract v1, futures-only ABI with routed
authoring and host enforcement"), parents `c1650a5` (the landing branch, created
at this ticket's arch spec commit, which is also `main`'s tip and carries every
earlier landing merge) and `af09b33` (working branch
`ticket/plugin-contract-v1`). The merge was clean with no conflicts, the landing
tree is byte-identical to `af09b33` except for this note and the committed
report, and `ticket/plugin-contract-v1` was deleted after the merge.

At landing, on the landing branch: the verification gate's full workspace
`cargo test` run is green on the identical tree (46 passed, 0 failed) together
with `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test -p plugin-contract --features authoring` (33 passed, covering abi,
authoring, payload, feature_gate, c_layout and no_plugin_thread),
`cargo test -p plugin-runtime` (6 passed), the fixture harness
`[result] PASS (62 checks)` with exit 0, the gcc and clang C builds under
`-std=c11 -Wall -Wextra -Werror`, and `cargo check -p plugin-contract
--no-default-features`. Re-run on the landing branch:
`scripts/regenerate-abi-header.sh --check` reports that the ABI header matches
the crate.

Divergences, from `deviation-reports/plugin-contract-v1.md` (committed with this
note) and the implementation report:

1. `CapabilityDescriptor` carries `request_schema: Schema` and
   `response_schema: Schema` rather than schema text. The wire payload is
   unchanged; a Rust author names `schemars::Schema`.
2. The C-side presence rule is written from per-entry slot constants in the
   header (`METIS_PLUGIN_TABLE_V1_*_SLOT`, `METIS_HOST_API_V1_*_SLOT` and the two
   counts), because cbindgen renders neither a `const fn` nor a `size`-computed
   constant. The Rust predicates exist as `const fn` and
   `c/layout_static_asserts.c` asserts the C form.
3. `Accounting` lost its public `suppressed` field; the rule it counted died
   with the completion callback. Only the fixture and the harness constructed
   it.
4. `plugin-runtime` parses the capability declaration into its own
   `registry::Capability` rather than enabling `plugin-contract`'s `authoring`
   feature, keeping the host on the primitives and the payload envelope alone. A
   test asserts the two declarations agree against the real fixture.
5. The C implementer is compiled by `scripts/build-c-plugin.sh` plus a CI step,
   not a `build.rs`, because it is a second cdylib for the harness rather than
   something the Rust build links. CI also gains the `--no-default-features` step
   for `plugin-contract`.
6. Added surface beyond the plan text: `MetisFutureHandle` is `Copy`, `CallError`,
   `RouteError` and `RegistryError` are exported, `plugin-runtime` exposes
   `Loaded`, `Host`, `Instance` and `PluginCall`, and `Call::service_request`
   returns a `ServiceAnswer<R>` future. `plugin-fixture` gained a direct
   `schemars` dependency for its derives.

Residual risks, as the implementation and verification report them: a wake after
drop and a drop after unload have counters and ordering refusals but no fixture
capability that deliberately provokes either; the missed-cancel check covers a
plugin that ignores its token, not one that never answers; the per-capability
version gate is unit-tested in `plugin-runtime` but not driven across the ABI
(the fixture advertises version 1 only); `plugin-runtime`'s integration tests and
the harness both rebuild the fixture cdylib, so those targets serialise on the
cargo locks; event and progress callbacks are read and released but only logged,
pending the daemon effort; and the C plugin keeps file-scope globals with a
single-instance assumption. There is no `async-ffi` dependency anywhere outside
`spikes/`.

Landing lessons. The version gate refuses a minor it does not know, and the
variant `plugin-compatibility` relaxes to additive tolerance is `MinorMismatch`,
while `EntryMissing` and `Unreadable` cover the size cases. Two items stay open
for deliberate review rather than being inherited by accident: the duplicated
capability declaration (`registry::Capability` against `CapabilityDescriptor`,
tied by a test) and the error taxonomy realised partly by `CallError`,
`RouteError` and `RegistryError` ahead of the item that would decide it.

Pointers: the ADR is `docs/adr/0002-plugin-contract-futures-only.md`, the C
implementer is `crates/plugin-fixture/c/c-plugin.c`, the build is
`scripts/build-c-plugin.sh` with its CI step, the arch spec's
`plugin-contract-v1` section is reconciled to the landed code, and the review
artifact is `deviation-reports/plugin-contract-v1.md`. The remaining cosmetic
item in that report is a stale rustdoc link to `Call::service` in
`crates/plugin-contract/src/authoring/traits.rs`; the method is
`Call::service_request`, and no deny is configured for broken links.
