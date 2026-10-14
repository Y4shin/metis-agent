---
type: deviation report
title: Deviation report for plugin-contract-v1
status: stable
---

## Deviation report: plugin-contract-v1

Compared: the `plugin-contract-v1` section of
`docs/tasks/metis-foundations/arch-spec.md` as approved, the ticket's acceptance
criteria, and the seven commits on `ticket/plugin-contract-v1` on top of
`c1650a5`. No `task/plugin-contract-v1` landing branch existed at review time, so
the diff base is the starting branch.

Reported check counts (`[result] PASS (62 checks)` and the unit and integration
suites) are as the implementation reported them. The slice-verifier owns
re-running them; this report compares surfaces, not green-ness.

### API surface changes

**1. Descriptor schema fields are `Schema`, not schema text.**
- **Planned:** inline `request_schema` and `response_schema` on
  `CapabilityDescriptor`, described as the schema JSON an author's types imply.
- **Actual:** `request_schema: Schema` and `response_schema: Schema`
  (`crates/plugin-contract/src/authoring/capability.rs:41`), with `schemars`
  re-exported from `authoring` so an author can derive them.
- **Impact:** the JSON payload shape is unchanged, because both are JSON objects
  on the wire. A Rust consumer names `Schema` rather than `String`.
  `plugin-compatibility` and `metis-plugin-sdk` are unaffected at the payload
  level.

**2. The C-side presence rule is slot constants, not a function.**
- **Planned:** the size-based presence predicates for both tables in the
  cbindgen `[export] include` list, with the computed byte thresholds moved out
  of the `exclude` list.
- **Actual:** the Rust predicates exist as `const fn`
  (`crates/plugin-contract/src/abi/mod.rs:513`-`523`), and the header carries
  per-entry slot constants (`METIS_PLUGIN_TABLE_V1_*_SLOT`,
  `METIS_HOST_API_V1_*_SLOT` and the two `*_SLOTS` counts) that the C presence
  rule is written from. The three computed byte thresholds stay in the `exclude`
  list because cbindgen renders neither a `const fn` nor a `size`-computed
  constant.
- **Impact:** none on `plugin-compatibility`, which consumes the Rust
  predicates. The C implementer is updated, and `c/layout_static_asserts.c`
  asserts the rule in the documented form.

**3. `metis_read_table` and `abi/version.rs` are new surface.**
- **Planned:** the version errors in `crates/plugin-contract/src/abi`, with
  `metis_read_prefix` named as the existing abstraction to build on.
- **Actual:** a new `abi/version.rs` with `MetisAbiError { Unreadable,
  MajorMismatch { expected, found }, MinorMismatch { known, found },
  EntryMissing { entry } }` and `metis_read_table`, which reads the prefix,
  refuses another major, refuses a minor it does not know in either direction,
  then names the first uncovered entry.
- **Impact:** additive, and it sharpens `plugin-compatibility`'s job: the variant
  it must relax to additive tolerance is named `MinorMismatch`.

**4. `Accounting` lost its public `suppressed` field.**
- **Planned:** reuse `Accounting` and `accounting` for the ledger.
- **Actual:** `Accounting { allocs, frees, live, violations }`.
- **Impact:** breaks any code constructing or destructuring the struct. Only the
  fixture and the harness do, and both were updated. The rule it counted died
  with the completion callback.

**5. `MetisFutureHandle` is `Copy`.**
- **Planned:** a two-field handle, no derives specified.
- **Actual:** `#[derive(Clone, Copy)]`, documented as what a two-field C struct
  is, with the exactly-once drop rule unchanged.
- **Impact:** additive; no consumer has to change.

**6. `plugin-runtime` grew a named surface and a second declaration type.**
- **Planned:** the host driver and registry with the seven named behaviours, and,
  per the `plugin-compatibility` section, inheriting the ABI version error rather
  than inventing its own.
- **Actual:** `Loaded`, `Host`, `Instance`, `PluginCall`, `PendingService`,
  `CallError`, and `registry::{Capability, Entry, Registry, RegistryError,
  RouteError, parse_capabilities, validate_request, validate_response}`. It does
  inherit the version error (`CallError::Version(MetisAbiError)` with
  `From<MetisAbiError>`), which satisfies the plan. It parses the capability
  declaration into its own `registry::Capability` rather than enabling
  `plugin-contract`'s `authoring` feature, so the declaration exists twice, tied
  by a test asserting the two agree.
- **Impact:** `metis-plugins` consumes these names. Two consequences for the
  plan: open item O8, the error taxonomy, is now partly realised without being
  decided, because `CallError`, `RouteError` and `RegistryError` exist alongside
  the frozen status space; and the duplicated declaration type is the same
  tension as the question of one shared declaration type, which
  `metis-plugin-sdk` will have to answer when it re-exports an author-facing
  descriptor.

### Abstraction usage

Used as specified: yes.

- Reused rather than reimplemented: the payload envelope
  (`crates/plugin-contract/src/payload.rs`, a 12-line change), size-based
  presence built on `metis_read_prefix`, `call.rs` extended for the service route
  (`Call::service_request`) rather than a second route type, `alloc.rs`
  accounting, the fixture's `payload.rs`, the harness conventions,
  `scripts/regenerate-abi-header.sh`, `libloading`, and the devenv shell.
- Forbidden paths avoided: no `async-ffi` (it appears only in the throwaway
  `spikes/plugin-registry`), no plugin-side executor (the file is deleted), no
  async runtime in `plugin-contract` (its dependencies stay `serde`,
  `serde_json` and `schemars`, all behind `authoring`), no second codec layer,
  and the Protobuf pipeline is untouched.
- One wording correction for the spec: it said the harness prints "one PASS or
  FAIL line per check". The repo's actual convention, which predates this ticket,
  is `[ok]` / `[fail]` per check plus a `[result] PASS` / `FAIL` summary. The
  implementation kept the real convention; the spec text was the imprecise part.

### Out-of-scope changes

- New workspace dependency edges, all anticipated: `schemars` in the workspace
  and behind `authoring`, plus a direct `schemars` in `plugin-fixture` for its
  derives; `jsonschema` with `default-features = false`; direct `serde`,
  `serde_json` and `libloading` in `plugin-runtime`; and a test-only
  `plugin-runtime` dev-dependency on `plugin-fixture`.
- New artifacts inside the ticket's stated scope:
  `docs/adr/0002-plugin-contract-futures-only.md`,
  `crates/plugin-fixture/c/c-plugin.c` (482 lines),
  `scripts/build-c-plugin.sh`, two CI steps (the C build, and
  `cargo check -p plugin-contract --no-default-features`), and the
  `docs/testing.md` commands.
- The C implementer is compiled by a script plus a CI step rather than a
  `build.rs`, because it is a second cdylib for the harness rather than something
  the Rust build links.
- `docs/tasks/metis-foundations/arch-spec.md` was rewritten by the TDD worker
  (218 insertions, 134 deletions), including folding in its own two divergences.
  The ticket's acceptance criteria required that reconcile, so it is legitimate,
  but the pipeline's normal owner of the arch spec is the spec-reconciler at
  end-of-effort. The end-of-effort reconcile should not re-apply what is already
  there.
- Defect found while diffing, cosmetic:
  `crates/plugin-contract/src/authoring/traits.rs:134` doc-links
  ``Call::service``, which does not exist; the method is `Call::service_request`
  (correctly linked at `call.rs:298` and `authoring/mod.rs:23`). No rustdoc
  broken-link deny is configured, so nothing fails today.

### Ticket doc update needed?

Yes. The ticket doc is unchanged by the chain (only `arch-spec.md` changed under
`docs/tasks/`), and its `## Implementation notes` still holds only the pre
implementation pointers. Append:

- the landing facts at land time: the merge commit on `task/plugin-contract-v1`,
  its parents, and the deleted working branch;
- the check counts and exact commands, including the harness
  `[result] PASS (62 checks)` and the C build under gcc and clang;
- the divergences above, at least 1, 2, 4 and 6 plus the C-build mechanism,
  because a reader of the ticket alone would otherwise expect a `String` schema
  field and a callable C presence predicate;
- the residuals the implementation reported: the two structural-only hazards,
  the missed-cancel path, the version gate not driven across the ABI, the fixture
  rebuild serialising integration tests, callbacks logged rather than streamed,
  and the C plugin's single-instance globals;
- pointers to the ADR, the C plugin, the two CI steps, and this report.

### User attention needed?

Yes, but nothing here blocks landing and no scope was dropped. Three things want a
decision or a note rather than a rework:

1. `plugin-runtime` carries its own capability declaration type alongside
   `plugin-contract`'s `CapabilityDescriptor`, tied by a test. That is the "one
   shared declaration type" question, in the same family as the open item about
   compile-time agreement of host and plugin payload types, and
   `metis-plugin-sdk` will meet it when it re-exports an author-facing
   descriptor.
2. `CallError`, `RouteError` and `RegistryError` realise part of the error
   taxonomy against the frozen status space without that item being decided. It
   should be reviewed deliberately rather than inherited by accident.
3. The arch spec was reconciled by a worker rather than by the spec-reconciler,
   which is a pipeline observation for the end-of-effort stage.

The two divergences the arch spec already records, the presence constants in the
header and the runtime's own declaration type, are documented there, so that
reconcile has landed rather than being owed.
