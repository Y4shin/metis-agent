---
type: deviation report
title: Deviation report for plugin-compatibility
status: stable
---

## Deviation report: plugin-compatibility

Implemented on `ticket/plugin-compatibility` (HEAD `602b348`, nine `wip:`
commits off `main`). Compared against the `## Pending ticket:
plugin-compatibility` section of `docs/tasks/metis-foundations/arch-spec.md` and
the ticket doc's acceptance criteria. I re-ran the gates on that commit: `cargo
fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
(workspace), `bash scripts/regenerate-abi-header.sh --check`, and `cargo run -p
plugin-fixture --bin abi-harness` (`[result] PASS (73 checks)`), all green.

### API surface changes

- **Planned:** the interface contract says `plugin-compatibility` "relaxes the
  exact-minor refusal `metis_read_table` shipped to additive tolerance
  (`found >= known`)".
  **Actual:** exactly that, in `crates/plugin-contract/src/abi/version.rs`: `if
  prefix.abi_minor < METIS_ABI_MINOR`. No signature change, no new variant. The
  `MinorMismatch` doc now states the one-directional rule, and `abi/mod.rs`, the
  generated header and the C static-assert comment were updated to match.
  **Impact:** `plugin-runtime` inherits the relaxation through the shared
  `MetisAbiError` with no code change; only its `CallError::Version` doc text
  moved. Minor 0 stays refused, so no dependent ticket sees a behavior change
  beyond "newer minor accepted".

- **Planned:** "The fixture gains one capability descriptor with a version the
  harness refuses, which moves the harness's expected capability list and its
  check count."
  **Actual:** the fixture declares eight capabilities, not seven, with
  `tool.newer` at descriptor version 2. It adds two public constants,
  `NEWER_CAPABILITY` and `NEWER_CAPABILITY_VERSION`, and one private
  `CAPABILITY_VERSION`. The spec's "any public constant it adds needs a cbindgen
  `exclude` entry" is satisfied trivially: the constants live in
  `plugin-fixture`, which cbindgen never processes, and the generated header
  contains no `NEWER_` symbol (verified).
  **Impact:** the fixture's public `CAPABILITIES` array changed from `[&str; 7]`
  to `[&str; 8]`. `crates/plugin-runtime/tests/registry.rs` is the only consumer
  and moved its "every descriptor is version 1" assertion to expect the gated
  version for that one name. Any later consumer must read the array, not the
  length.

- **Planned:** seam 2, "the `metis_plugin_entry_v2` lookup fails, the harness
  reports the typed error rather than a raw loader error".
  **Actual:** the harness's old check `symbol.wrong_major_absent` is removed and
  replaced by four checks that load a real other-major library:
  `symbol.other_major_library_present`, `_library_loaded`,
  `_exactly_one_symbol`, `_typed_error`, yielding `MajorMismatch { expected: 1,
  found: 2 }`. The harness's check count moved from 62 to 73.
  **Impact:** the harness baseline for the plugin line is 73. The removed check
  was tautological (it looked up a symbol the fixture never exported), so the
  replacement is the stronger form the spec's "do not add a gating check the
  fixture cannot make fail" spirit asks for.

- **Planned:** seam 1's prefix seam, "driven with a shortened table whose
  uncovered tail is a canary that would be observable if it were touched".
  **Actual:** two mechanisms. In `plugin-contract`'s tests, an
  `ExtendedTableV1` with an appended entry and a canary counter plus an
  `an_appended_entry_and_a_newer_minor_still_interoperate` test. In the harness,
  `table.appended_entry_not_read`, `table.uncovered_entry_not_read` and
  `table.stale_minor_entries_not_read` place the table in an `mmap`ed page whose
  uncovered tail is `PROT_NONE` via a local `mprotect` binding, so a read past
  the reported size faults instead of reading a canary value.
  **Impact:** none on the interface. The three harness checks are Linux only; on
  other targets they print a note and are skipped rather than failed. The Rust
  canary test runs everywhere, so seam 1 keeps cross-platform coverage.

- **Planned:** seam 5, "the header regenerates with no diff".
  **Actual:** the committed header does change, by doc comments only: the
  `METIS_ABI_MINOR` block and the `abi_minor` field doc. `--check` passes
  because the header was regenerated from the Rust docs, which this ticket
  rewrote for the relaxed rule.
  **Impact:** cosmetic. Any branch carrying the pre-relaxation header must
  regenerate it; the drift check is already in CI.

- **Planned:** "The per-capability gate, demonstrated in the fixture and the
  harness: a host refuses a capability whose descriptor version it does not
  support with `Unsupported` for that capability alone."
  **Actual:** the harness applies its own gate policy,
  `HOST_CAPABILITY_VERSION = 1` in `capability_gate`, and the checks
  `gate.declared_versions`, `gate.version_refused_for_one_capability`,
  `gate.refusal_is_the_hosts` and `gate.rest_of_plugin_still_serves` show that
  `tool.newer` alone is refused while the plugin lists and serves the rest.
  `plugin-runtime::Registry::gate` keeps its unit test
  `the_version_gate_is_per_capability`, which drives synthetic names, not the
  real fixture.
  **Impact:** the refusal is now falsifiable end to end, which was the stated
  gap. The gate policy still exists in two places (harness constant and the
  runtime's `gate`); the runtime loader is another effort's work by the spec's
  own words, and the fixture's `tool.newer` is the capability that effort can
  reuse when it applies the gate at load time.

### Abstraction usage

- Used/was specified: yes.
  - `metis_read_prefix` and the landed size-based presence predicate: the
    version check stays between the prefix read and the entry size checks, and
    the check order (major, then minor, then size) is now documented in
    `version.rs` as part of the contract.
  - `MetisAbiError`: consumed, not re-created. `MajorMismatch`, `MinorMismatch`
    and `EntryMissing` are all exercised; `Unreadable` keeps its existing test.
  - Harness prior art: `Checks::new()`, one `[ok]`/`[fail]` line per check, the
    `[result]` line, the rebuild-first step and the real library load all stay.
    `HostState::invoke` was split into a typed body encoder plus `invoke_body`,
    which takes raw bytes, so the unknown-field check can send a body this build
    has no type for.
  - `crate::payload`: the unknown-field rule is unchanged, and its tests were
    added in the payload module rather than as a new envelope rule.
  - Layout: `MetisPluginTableV1` keeps eight entries and 80 bytes; the Rust const
    assertions and `c/layout_static_asserts.c` changed by comment only; the
    harness still reports `table.size_matches_layout: size=80` and
    `symbol.exactly_one_metis_symbol`.
  - No loader was added to `plugin-contract`; the symbol probe stayed
    harness-owned.

### Out-of-scope changes

- New file `crates/plugin-fixture/c/other-major-plugin.c`: a 19-line C library
  whose only export is `metis_plugin_entry_v2`, built so the major seam has a
  real failing case. Not named in the spec, but the spec's seam 2 requires
  something that can fail.
- `scripts/build-c-plugin.sh` now builds a second artifact,
  `target/c-plugin/libother-major.so`, with gcc only, since the header has
  already been through both compilers. The harness's rebuild-first step picks it
  up, so no CI step is added.
- `docs/testing.md` documents both additions: the compatibility checks in the
  harness and the second C library. The spec did not ask for the doc update.
- Comment-only edits in `crates/plugin-contract/src/abi/mod.rs` and
  `crates/plugin-contract/c/layout_static_asserts.c`, correcting text that still
  described the exact-minor rule. Both are the rule this ticket changes.
- `crates/plugin-runtime/src/lib.rs` (one doc comment) and
  `crates/plugin-runtime/tests/registry.rs` (the descriptor-version
  expectation). Neither changes loader behavior; the diff was checked line by
  line and contains no logic outside the test expectation.
- Nothing was removed from the layout, the header's constants, the checked-in
  wire contract, or the harness's existing check set apart from the one
  tautological major check named above.

### Ticket doc update needed?

Yes. The ticket doc has no `## Implementation notes` section (it ends at
`## Blocked by`), so the land-worker must create the heading, then record:

- the landing paragraph (merge commit, landing branch, working branch, as
  `plugin-contract-v1`'s note does);
- the fixture's capability list is eight, with `tool.newer` at version 2, and
  `plugin-runtime/tests/registry.rs` moved its expectation for that one name;
- the harness baseline is 73 checks, up from 62, with `symbol.wrong_major_absent`
  replaced by the four `symbol.other_major_*` checks;
- the new `crates/plugin-fixture/c/other-major-plugin.c` and
  `target/c-plugin/libother-major.so`, and the `docs/testing.md` text covering
  them;
- the header change is comment only and the layout is untouched;
- the three `table.*_not_read` checks are Linux only and skip elsewhere.

### User attention needed?

Yes, on two settled facts rather than on a decision:

- The ABI's acceptance rule for `abi_minor` changed from exact to `found >=
  known` and the fixture's public capability list grew from seven to eight. Both
  are the approved interface contract, but they are the surfaces the rest of the
  plugin line reads, and `plugin-runtime` now inherits the relaxed rule.
- Two platform and tooling caveats worth knowing: the strongest prefix-seam
  evidence (a faulting page) is Linux only, and the harness baseline moved to 73
  checks.

No spec change and no further operator decision is required before landing.
