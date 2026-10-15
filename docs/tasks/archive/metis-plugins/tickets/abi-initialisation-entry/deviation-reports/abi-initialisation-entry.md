---
type: deviation report
title: Deviation report for abi-initialisation-entry
status: stable
---

## Deviation report: abi-initialisation-entry

Compared: `docs/tasks/metis-plugins/arch-spec.md` section
`abi-initialisation-entry`, the ticket doc and its six acceptance criteria, the
effort spec's initialisation decisions, and the implementation on
`ticket/abi-initialisation-entry` (`git diff task/abi-initialisation-entry..ticket/abi-initialisation-entry`,
that is `ae96284..c6666ff`: 25 files, 1733 insertions, 351 deletions, across four
`wip:` commits). The implementation pointer was the tdd result of run
`047ce8dc`. Read directly, not taken on trust: the ABI table, the regenerated
header, the C static asserts, the contract text, the authoring trait and adapter,
the fixture, the C plugin, the harness's waker lifecycle, and the ticket doc.

### API surface changes

**1. The runtime gained a public initialisation hand-off, which the spec had
assigned to `async-initialisation`.**

- **Planned:** "the host-side driver for that future lands in
  `async-initialisation`, not here", and this ticket's exports are the amended
  table in `plugin-contract`.
- **Actual:** `plugin-runtime` replaced
  `Loaded::create_instance(&host) -> Result<Arc<Instance>, CallError>` with
  `Loaded::init_plugin(&host, body: &[u8]) -> Result<PluginInit, CallError>`,
  and `PluginInit` is a public `Future<Output = Result<Arc<Instance>, CallError>>`
  that owns the plugin's init future, reads the capability declaration once the
  future resolves, and destroys the instance pointer on drop when it did not
  resolve `Ok`. `CallError::Create(MetisStatus)` was renamed
  `CallError::Init(MetisStatus)`. Two private details came with it: the ordering
  ledger marker `initialisation started from <path>`, and
  `INITIALISATION_REQUEST_ID = 0`, the id a late wake from an initialisation is
  recorded against.
- **Impact:** the table change cannot leave the runtime compiling against a
  deleted slot, so the work is necessary, but the surface is wider than the plan
  and public. `async-initialisation` should consume `Loaded::init_plugin` and
  `PluginInit` rather than reach the raw table, and should confirm at its own
  seam that the failure ordering is the one it wants (drop the future, then
  `destroy_instance`). `manifest-and-library` resolves the entry symbol and reads
  the table, so it sees the renamed slot constant too.

**2. The authoring trait broke: `Default` is gone, `init` is in, `started` is
out.**

- **Planned:** the arch spec's section for this ticket lists the `authoring`
  feature gate and `plugin_adapter` as things to use, and names no change to the
  authoring trait. The effort spec does require the shape (an asynchronous
  constructor, no `Default`), but it is the section that assigns it, not this
  ticket's export list.
- **Actual:** `pub trait Plugin: Send + Sync + 'static` with
  `fn init(host: Host, initialisation: Initialisation) -> impl Future<Output =
  Result<Self, PluginError>> + Send where Self: Sized`; `fn started` removed;
  `fn stopped` kept; two new public types, `Initialisation` and `HostContext`,
  with `Initialisation::{new, body, decode, config, host_context}` and the two
  field constants. The adapter installs `init_plugin: init_plugin::<P>` in slot 0
  and calls `P::init(host, initialisation).await`.
- **Impact:** this is a break of a published authoring surface that the
  `metis-plugin-sdk` effort consumes, and no ticket in this effort's graph owns
  the migration. The old `Plugin` shape is not reachable behind a flag, so the
  SDK inherits the new shape wholesale: `Default` construction and `started` are
  gone, and every author must write `init`.

**3. The initialisation payload types sit behind the plugin-side feature gate.**

- **Planned:** "The configuration and the host context are the runtime's own
  value types, built by the daemon and delivered in one payload", and the
  dependency posture keeps `plugin-contract`'s `authoring` feature off for
  `plugin-runtime` so the host sees the primitives and the envelope alone.
- **Actual:** `HostContext` and `Initialisation` live in
  `plugin_contract::authoring::traits`, and `pub mod authoring` is behind
  `#[cfg(feature = "authoring")]`. `plugin-runtime` does not enable that feature
  and its manifest comment still says it stays off.
- **Impact:** the next ticket cannot use those types to build the payload
  without either turning the `authoring` feature on for the runtime (which the
  posture forbids, because the feature pulls `schemars`) or writing the body
  itself, keeping the names `config` and `host_context` and the `HostContext`
  field set in step by hand. `serde_json` is already a runtime dependency, so
  the second path works, but the duplication is the risk and the decision should
  be taken deliberately by `async-initialisation`.

**4. New public ABI items, additive and expected by the presence rule.**

- **Planned:** the slot becomes `init_plugin`, the counters entry does not
  exist, the minor is bumped, the symbol and major do not move.
- **Actual:** exactly that, plus
  `METIS_PLUGIN_TABLE_V1_INIT_PLUGIN_SLOT`,
  `METIS_PLUGIN_TABLE_V1_INIT_PLUGIN_THRESHOLD`,
  `metis_plugin_table_has_init_plugin(size)` and the `MetisInitPluginFn` type
  alias. `METIS_ABI_MAJOR` is 1 and `metis_plugin_entry_v1` is unchanged;
  `METIS_ABI_MINOR` is 2. Verified in the tree, not only in the report: the
  regenerated header carries `METIS_ABI_MAJOR 1`, `METIS_ABI_MINOR 2`,
  `METIS_PLUGIN_TABLE_V1_INIT_PLUGIN_SLOT 0` and `MetisInitPluginFn
  init_plugin;`. No `create_instance` or counters entry survives in the crates:
  the only remaining occurrences of either name are the doc comment in
  `src/abi/mod.rs` that records the replacement, and the two tests in
  `tests/abi.rs` that assert the generated header and the contract table no
  longer carry them.
- **Impact:** additive, and the dependent tickets (`manifest-and-library`,
  `async-initialisation`) read the new constant and predicate through the same
  presence rule as before.

**5. The contract gained a fixed initialisation payload, section 18.**

- **Planned:** one envelope payload carrying the configuration and the host
  context, with the plugin's setup failure falling into two distinguishable
  classes.
- **Actual:** the body is one JSON object with `config` and `host_context`; the
  host context fields this build carries are `instance_name`, `data_dir`
  (optional) and `capability_version`, and it is declared permissive so a host
  may add a field; a configuration the plugin cannot read is
  `invalid_argument`, every other terminal status is the plugin's own defect;
  and on either the host drops the future and then calls `destroy_instance`.
  Section 11 rule 1 now states the init payload is host-owned and borrowed for
  the entry, and section 5 states the table is eight entries with no counter.
- **Impact:** this is the interface the C author and the SDK both write against,
  and `manifest-and-library` must supply the configuration schema this payload
  carries.

**6. Removals, in the same commit as their replacements.** `Plugin::started`,
`MetisCreateInstanceFn`, `METIS_PLUGIN_TABLE_V1_CREATE_INSTANCE_SLOT`,
`Loaded::create_instance`, `CallError::Create`, and the `cbindgen.toml` include
entry for the old alias. Nothing was kept behind a compatibility flag.

### Abstraction usage

- Used/was specified: **yes**. The size-prefixed append-only table rule and
  `MetisAbiError`'s version gate are unchanged; the `authoring` feature gate and
  `plugin_adapter` carry the Rust side of the entry; the payload envelope from
  `payload.rs` carries the init body; the header regeneration script, the
  `no_plugin_thread` and `feature_gate` tests, and the C plugin build are the
  gates they were.
- The "do not reimplement" list held in full: one table type and no per-version
  entry wrapper; no async runtime and no executor anywhere (the entry returns a
  table-defined future, the host polls it, and `no_plugin_thread` now measures
  the thread count across initialisation too); no serde type across the
  boundary, only the JSON envelope; the counters entry is deleted rather than
  flagged off; and the MSVC `_Static_assert` message form is untouched, so
  `c-static-assert-msvc` is still the ticket that owes that fix.
- No dependency changed at all: `Cargo.toml` and `Cargo.lock` are not in the
  diff.

### Out-of-scope changes

- Two runtime test files were touched, mechanically and not to force green:
  `crates/plugin-runtime/tests/registry.rs` (expected ABI `(1, 1)` to `(1, 2)`,
  and one fixture lifecycle log line) and `crates/plugin-runtime/tests/waker.rs`
  (an absolute waker count became a delta). The arch spec's named seams for this
  ticket are the ABI harness, the contract tests and the C plugin build, so
  these are outside the agreed seam list, as the tdd result also records.
- `crates/plugin-fixture/src/payload.rs` gained `FixtureConfig`, the fixture's
  own configuration declaration, and re-exports the two new authoring types.
  Reasonable, but it is a new fixture type the plan did not name, and it is what
  makes the C plugin's configuration declaration checkable in Rust too.
- The carried risk from the previous ticket is **closed**, and this is planned
  rather than out of scope: the ABI harness now releases the one owning waker
  reference it hands each poll as soon as that poll returns, at all three poll
  sites, asserts `waker_handed_out == waker_released`, and records a wake after
  a drop as a request id in a list rather than a count. The harness grew from 73
  to 82 checks (reported by the tdd result; I read the release path and the
  accounting, not the run).
- Residual, recorded rather than filled: the runtime failure path (an
  initialisation that never resolves `Ok`, so the future is dropped and the
  pointer destroyed) has no test at the runtime seam. It is covered at the raw
  ABI seams, in `authoring.rs` and by the harness against the C plugin. The
  runtime seam belongs to `async-initialisation`.

### Ticket doc update needed?

Yes. The ticket doc has no `## Implementation notes` section yet, so the landing
step should create it and record: the four divergences above (the runtime
hand-off and the `CallError::Init` rename, the authoring trait break, the payload
types behind the `authoring` gate, the fixture's `FixtureConfig`), the mechanics
(25 files against `ae96284`, four `wip:` commits, tip `c6666ff`, `state.yaml`
left uncommitted per the `475f1c0` precedent), the gate results the tdd result
reports (`cargo test` 293 passed with 8 ignored, harness `82 checks PASS`, header
drift check, C plugin build under gcc and clang, `no-default-features` check),
the closed waker obligation from the previous ticket, and the runtime-seam
residual. The ticket's checkboxes and `workflow_state` stay untouched: the
landing and finalize steps own them.

### User attention needed?

Yes, three sentences, because two of them outlive this effort.

1. The authoring trait is a published surface the `metis-plugin-sdk` effort
   consumes, and it broke here (`Default` removed, `started` removed, `init`
   added) with no ticket in this effort owning the migration. The SDK effort
   needs to absorb that break.
2. The initialisation payload's value types live behind `plugin-contract`'s
   `authoring` feature, which `plugin-runtime` deliberately keeps off, so
   `async-initialisation` must either build the `{"config", "host_context"}`
   body itself or the dependency posture has to change. That decision is worth
   taking deliberately rather than by accident.
3. `plugin-runtime` gained a public hand-off (`Loaded::init_plugin`,
   `PluginInit`) that the approved spec had assigned to `async-initialisation`;
   the arch spec should be reconciled to say whether the hand-off stays where it
   landed or moves.
