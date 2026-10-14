# Approval: `plugin-compatibility` architecture spec

- Approved: 2026-10-13, in chat, by the operator.
- Approved artifact: `docs/tasks/metis-foundations/arch-spec.md`, section
  `## Pending ticket: plugin-compatibility` (verified at commit `58d84b1`).
- Ticket: `docs/tasks/metis-foundations/tickets/plugin-compatibility/ticket.md`.

## State verified before approval

- HEAD `58d84b1`, branch `main`, working tree clean. The handoff named
  `1efe1c2` as HEAD; one docs commit landed after it.
- Plugin ABI gates pass on that commit: `cargo test -p plugin-contract -p
  plugin-runtime`, `cargo run -p plugin-fixture --bin abi-harness` (`[result]
  PASS (62 checks)`), `bash scripts/regenerate-abi-header.sh --check`.
- Contract gates pass: `bash scripts/regenerate-contracts.sh --check`,
  `buf lint`, `buf breaking --against '.git#branch=main'`.
- The spec section no longer states tolerance for the retired shorter table;
  its remaining "shorter table" mentions are the prefix-canary seam.

## Decision taken at approval

1. The spec is approved unchanged. Its Interface contract already fixes the
   tolerance direction: `found >= known` for `abi_minor`, so a newer minor is
   tolerated and the retired pre-release minor 0 stays refused.
2. Ticket acceptance criterion 4 was reworded into its two real clauses:
   - another ABI major fails at symbol resolution, before any table is read,
     reported as a typed error rather than a raw loader error (spec seam 2);
   - the retired pre-release v1 table exports the same
     `metis_plugin_entry_v1` symbol, so it resolves and is then refused by the
     version gate with `MinorMismatch` before any entry is read (spec Interface
     contract, and the minor-0 half of
     `crates/plugin-contract/tests/abi.rs::a_minor_this_build_does_not_know_is_refused`).
   The original wording claimed symbol-resolution failure for both cases, which
   is only true for the other-major case: `METIS_ABI_MINOR` moved 0 to 1 while
   `METIS_PLUGIN_ENTRY_V1` stayed `b"metis_plugin_entry_v1\0"`.

## Not settled here, by design

- The unknown-kind refusal (a capability declaring a kind the host does not
  know) stays unit-tested at the gate predicate only; this ticket's fixture
  change adds a refused-version capability, not an unknown kind, so that debt
  carries forward.
- The structural-only hazards recorded in the arch spec (a wake after drop, a
  drop after unload) keep counters and ordering refusals but no provoking
  fixture capability. Not this ticket's scope.
