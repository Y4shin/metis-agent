---
type: ticket
subtype: feature
title: Plugin version tolerance across compiled versions
status: stable
workflow_state: ready
blocked_by: [plugin-abi-core, plugin-contract-v1]
size: s
---

## What to build

The promise that a plugin built earlier still works on a newer daemon within the
current ABI major, and that a newer plugin's unsupported capability is refused on
its own rather than taking the whole plugin down.

This ticket used to own the tolerance for a shorter table. That criterion moved
out: `plugin-contract-v1` replaces the pre-release v1 table with a revised v1
shape, so there is no
longer an older table to tolerate across the boundary. What remains here is
tolerance within the revised v1 table and the per-capability and payload promises.

## Acceptance criteria

- [ ] A host and a plugin that agree on `abi_major` but differ in a table entry
      added after this ticket lands interoperate, with the reader touching only
      the entries its own `size` covers, asserted by a test.
- [ ] The host refuses a single capability whose descriptor version it does not
      support while the rest of the plugin stays usable, asserted by a test.
- [ ] Payloads carrying unknown fields deserialise successfully, covered by a
      test.
- [ ] A plugin that exports only another ABI major's entry symbol fails at
      symbol resolution, before any table is read, with a typed error rather
      than a raw loader error.
- [ ] A plugin built against the retired pre-release v1 table exports the same
      `metis_plugin_entry_v1` symbol, so it is refused by the version gate with
      `MinorMismatch` before any table entry is read: additive tolerance is
      `found >= known`, which leaves minor 0 refused. Asserted by a test.

## Blocked by

- `plugin-abi-core`, because version tolerance is a property of the table and the
  descriptors.
- `plugin-contract-v1`, because tolerance has to be asserted against the revised
  v1 table that ticket lands, not against the pre-release table it replaces.

## Implementation notes

Landed on `task/plugin-compatibility` as the merge commit `c9227a4`
("ticket(plugin-compatibility): Plugin version tolerance across compiled
versions"), parents `df22147` (the landing branch, created at this ticket's arch
spec commit, which is also `main`'s tip and carries every earlier landing merge)
and `602b348` (working branch `ticket/plugin-compatibility`, nine `wip:`
commits). The merge was clean with no conflicts, the landing tree is
byte-identical to `602b348` except for this note and the committed report, and
`ticket/plugin-compatibility` was deleted after the merge.

At landing, on the landing branch, through `devenv shell --`: `cargo fmt --check`
reports `FMT-OK`, `cargo clippy --all-targets -- -D warnings` is clean, the
workspace `cargo test` exits 0 with 28 targets, 55 tests passed and no failure,
`cargo run -p plugin-fixture --bin abi-harness` exits 0 with
`[result] PASS (73 checks)`, 73 `[ok]` and 0 `[fail]`, and
`bash scripts/regenerate-abi-header.sh --check` reports `HEADER-OK`. The
committed harness baseline is therefore 73 checks, up from the 62 the previous
landing recorded.

Divergences, from `deviation-reports/plugin-compatibility.md` (committed with
this note) and the implementation report:

1. `CAPABILITY_VERSION` in `crates/plugin-fixture/src/lib.rs` is private rather
   than public. Nothing outside the crate reads it, so no cbindgen `exclude`
   entry is needed. `NEWER_CAPABILITY` and `NEWER_CAPABILITY_VERSION` stay
   public for the harness and `plugin-runtime`'s test.
2. The fixture declares eight capabilities, not seven, with `tool.newer` at
   descriptor version 2. The fixture's public `CAPABILITIES` array moved from
   `[&str; 7]` to `[&str; 8]`, and `crates/plugin-runtime/tests/registry.rs`
   moved its per-name version expectation to match.
3. Criterion 5's retired pre-release plugin is driven by patching the live
   table's prefix (minor 0, the superseded seven-entry size) rather than by a
   second compiled reject plugin, because no pre-release table remains in the
   tree to build. The probe is still reached through `metis_plugin_entry_v1`, so
   "the retired table exports the same symbol" holds by construction.
4. The per-capability gate is demonstrated in the harness with a local policy
   constant (`HOST_CAPABILITY_VERSION = 1`) instead of through
   `plugin-runtime::Registry::gate`. The arch spec assigns the loader to the
   metis-plugins effort and names the fixture plus harness as this ticket's
   measured demonstration; the runtime gate keeps its unit test
   `the_version_gate_is_per_capability`.
5. The major seam gained `crates/plugin-fixture/c/other-major-plugin.c` and a
   second artifact from `scripts/build-c-plugin.sh`,
   `target/c-plugin/libother-major.so`. The tautological harness check
   `symbol.wrong_major_absent` is replaced by four `symbol.other_major_*` checks
   over that real library, and `docs/testing.md` documents both additions.
6. `crates/plugin-contract/src/abi/mod.rs` and the `MetisPluginTableV1.abi_minor`
   field doc still described the exact-minor rule this ticket relaxes, so both
   were corrected. The field doc correction regenerated one comment line in the
   committed C header; the header diff is comment only.
7. The layout is untouched: `MetisPluginTableV1` keeps its eight entries and 80
   bytes, and the Rust const assertions and `c/layout_static_asserts.c` changed
   by comment only. No loader was added to `plugin-contract`.

Residual risks, as the implementation and verification reports state them: the
three `table.*_not_read` harness checks use `mprotect` and are Linux only, so
criteria 1 and 5 lose their faulting-page canary elsewhere while their unit tests
still run; criterion 5 does not exercise a separately built minor-0 binary; the
capability gate policy exists in two places (the harness constant and the
runtime's `gate`); and a pre-existing `_FORTIFY_SOURCE requires compiling with
optimization (-O)` warning appears in `cargo test` and `cargo clippy` output,
which does not affect either exit status.

Knowledge harvested at finalize: `docs/plugin-contract.md` section 5 no longer
calls `abi_minor` advisory, and section 15 states the landed `found >= known`
rule instead of describing the relaxation as future work; ADR 0002 carries the
same one-directional amendment. `docs/testing.md` already recorded the harness
compatibility checks and the second C library from the implementation, so it
needed no further change.
