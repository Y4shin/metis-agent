---
type: deviation report
title: Deviation report for plugin-abi-core
status: stable
---

## Deviation report: plugin-abi-core

Reviewed: `task/plugin-abi-core..ticket/plugin-abi-core` (22 files, +3370/-5) and
the working tree at `5e122d2`, read against the `plugin-abi-core` section of
`docs/tasks/metis-foundations/arch-spec.md` and the ticket's seven acceptance
criteria.

### API surface changes

**The exported ABI surface matches the plan, name for name.**

- **Planned:** `crate::abi` with `METIS_ABI_MAJOR`/`METIS_ABI_MINOR`,
  `METIS_PLUGIN_ENTRY_V1`, `MetisStatus` 0 through 7 including `Pending`,
  `MetisBuffer` plus the owner tags, `MetisTablePrefix` plus
  `metis_read_prefix`, eleven function-pointer aliases, `MetisPluginTableV1`
  (`size` first, seven pointer slots, 72 bytes on 64-bit), `MetisHostApiV1`
  (`size`, `user_data`, four callbacks), and the argument-free
  `MetisPluginEntryV1Fn`.
- **Actual:** exactly that, in `crates/plugin-contract/src/abi/mod.rs` and
  `src/abi/layout.rs`. `metis_read_prefix` is exercised by the harness against
  both the real table and a deliberately short one.
- **Impact:** none on dependents. `plugin-compatibility` and the metis-plugins
  host can read the prefix and the table exactly as the interface contract
  promised.

The remaining deviations are in tooling and test support, not in the ABI.

**Deviation 1: the nixpkgs attribute is `rust-cbindgen`, not `cbindgen`.**

- **Planned:** `devenv.nix` adds `pkgs.cbindgen`.
- **Actual:** `pkgs.rust-cbindgen`, which installs the `cbindgen` binary at
  0.29.4, the version the mechanics spike measured. The spec's attribute does
  not exist in the pinned nixpkgs.
- **Impact:** none on dependents. The arch spec's export list is wrong and
  should be corrected so a later chain does not copy the bad attribute.
  `docs/dev-env.md` records the real name.

**Deviation 2: `cbindgen.toml` needs an `exclude` list for the payload
constants.**

- **Planned:** `[export] include` lists every ABI type, and `crate::payload`
  sits outside the cbindgen surface.
- **Actual:** cbindgen emits every public constant of the crate regardless of
  the include list, so the first header contained `METIS_PAYLOAD_*` and
  `METIS_CODEC_*`. The six constants are now excluded by name, and the header
  contains exactly `crate::abi`.
- **Impact:** low but load-bearing for the next ticket. `plugin-authoring-trait`
  adds public items to the same crate, and any new public constant will land in
  the header unless it is excluded too. Worth recording on the ticket.

**Deviation 3: the fixture crate is `cdylib` plus `rlib`.**

- **Planned:** the fixture crate is a `cdylib` only.
- **Actual:** `crate-type = ["cdylib", "rlib"]`, so the sibling harness binary
  can link the fixture's payload shapes instead of restating the JSON. The
  loaded artifact is unchanged and still exports exactly one `metis_*` symbol.
- **Impact:** none. Nothing outside `crates/plugin-fixture` depends on it.

**Deviation 4: the fixture has four capability names, not five paths.**

- **Planned:** `tool.echo`, a pending capability, a cancel path, a
  late-completion-after-cancel path, and a panic path.
- **Actual:** `tool.echo` (inline), `tool.pending` (pending, `progress`, then
  `complete`), `tool.cancellable` (suppresses its completion after a cancel),
  `tool.panic`. The late path is `tool.pending` itself, which never honours a
  cancel, so its completion arrives after the host's successful cancel and the
  host drops it. No `tool.late` name exists.
- **Impact:** none. The ownership seam is still covered: the harness has
  `cancel.host_dropped_late_completion` and `cancel.dropped_payload_still_released`.

**Deviation 5: the host-callback panic probe lives in the harness, not the
fixture.**

- **Planned:** seam 5, "a panic inside a host callback becomes `Internal`".
- **Actual:** the harness panics on purpose inside its own `complete` callback
  for one designated request, catches it there, and records that request as
  `internal`. A panic inside a Rust `extern "C"` callback cannot be caught by
  the plugin that called it, because unwinding out of an `extern "C"` frame
  aborts, so only the callback's own frame can convert it.
- **Impact:** this is a constraint the host implementer must honour.
  `plugin-runtime` has to wrap every callback it installs the same way, exactly
  as D7's amendment says.

**Deviation 6: one check was written and then removed before landing.**

- **Planned:** not required by the ticket or the spec.
- **Actual:** a per-capability version gate check was written, then removed
  because the fixture advertises no version a host would refuse, so the check
  could not fail. The harness notes that per-capability gating belongs to
  `plugin-compatibility`.
- **Impact:** none. It matches the spec's interface contract, which assigns
  per-capability gating to `plugin-compatibility`.

**Interface-contract note: the presence predicate is not exported yet.**

- The spec's interface contract for `plugin-compatibility` names "a `size`-based
  presence predicate". `metis_read_prefix` exists; a helper that answers "does
  this table carry the field at offset N" does not. `plugin-compatibility` adds
  it in-crate, which is additive and changes no layout.

### Abstraction usage

- Used/was specified: **yes.** Every type is ported from
  `spikes/plugin-abi-mechanics/crates/abi/` rather than invented: the table and
  host layouts, the `[export] include` list, the `cc` build script, the Rust
  const assertions, the C translation unit, and the header `--check` diff. The
  fixture and harness follow `crates/plugin` and `crates/host`, including the
  PASS/FAIL line pattern and the `env!("CARGO")` rebuild. No spike crate is
  depended on, no ABI is reimplemented in the harness, and no `build.rs`
  generation of product code was added.

### Out-of-scope changes

- `docs/dev-env.md` and `docs/testing.md` gained the new commands and the
  `cbindgen` note. Both follow the precedent `contract-pipeline` set, and
  `docs/testing.md` owns the run commands.
- `.github/workflows/ci.yml` gained the harness step and the header drift step,
  as the spec's CI bullet requires.
- `Cargo.toml` gained the `plugin-fixture` member and the `cc`, `libloading`,
  `serde` (with `derive`), and `serde_json` workspace entries. The `derive`
  feature is one word beyond the spec's `serde = "1"`.
- `Cargo.lock` grew with that dependency set.
- Additions only. No removals beyond replaced documentation lines, no spike or
  research file touched, no hand edit to the generated header, no `authoring`
  feature gate (correctly deferred to `plugin-authoring-trait`).

### Ticket doc update needed?

**Yes.** Append to `## Implementation notes` (do not alter the acceptance
criteria):

1. `devenv.nix` uses `pkgs.rust-cbindgen`, because `pkgs.cbindgen` does not
   exist in the pinned nixpkgs; the installed binary is `cbindgen` 0.29.4.
2. `cbindgen.toml` excludes the six `crate::payload` constants by name, because
   cbindgen emits public constants regardless of `[export] include`. Any new
   public constant in `plugin-contract` must be excluded too.
3. The fixture crate is `cdylib` plus `rlib`; the `rlib` exists only so the
   sibling harness binary shares the payload shapes. The cdylib still exports
   exactly one `metis_*` symbol.
4. The fixture advertises four capabilities; the late-completion-after-cancel
   path is `tool.pending`, which never honours a cancel.
5. The panic-in-a-host-callback probe is harness-owned, because unwinding out of
   an `extern "C"` callback frame aborts. Hosts must wrap every callback.
6. A per-capability version gate check was written and removed because it could
   not fail; per-capability gating is `plugin-compatibility`'s subject.
7. At landing the harness reported 41 checks, `plugin_ledger_balanced`
   `allocs=10 frees=10 live=0`, `ledger_agrees_with_harness` 10 plugin
   allocations against 10 harness releases, and `no_foreign_or_rejected_releases`.
8. Residual risks: the C side assumes function pointers and object pointers
   share size and alignment on supported targets (asserted, so an exotic target
   fails loudly); Windows `cdylib` loading, CRT, and allocator boundaries are
   untouched; the harness needs `nm` from the devenv shell for the symbol check.

### User attention needed?

**Yes, but no decision blocks progress.** There is no scope change and no
API-surface divergence; the ABI matches the spec exactly.

1. The arch spec names `pkgs.cbindgen`, which does not exist. Fold the real
   attribute name and the cbindgen constant-emission rule into the effort's
   arch spec before `plugin-authoring-trait` extends `plugin-contract`, so the
   next chain does not repeat the guess or leak constants into the header.
2. The host-callback panic rule is a real obligation on `plugin-runtime`, not
   just a fixture detail, and is worth carrying into the metis-plugins effort's
   interface notes.

### Evidence checked

- `crate::abi` export list read directly from `src/abi/mod.rs`; field order and
  the `size`-first rule confirmed in the struct definitions.
- `src/abi/layout.rs` writes all 39 assertions through `size_of::<usize>()`,
  `size_of::<*const c_void>()`, and `offset_of!`, including the append identity
  `offset_of!(shutdown) + PTR == size_of::<MetisPluginTableV1>()`.
- `build.rs` compiles `c/layout_static_asserts.c` with `-std=c11` and re-runs on
  either file changing; `tests/c_layout.rs` cross-checks the C-measured values.
- The harness rebuilds with `env!("CARGO") build --manifest-path ... --lib`
  before `Library::new`, resolves the library beside `current_exe()`, prints a
  line per check, and returns `checks.report()` as its exit code.
- `scripts/regenerate-abi-header.sh --check` regenerates to a scratch file and
  diffs; the committed header is the generator's output, not hand-edited.
- CI gained the harness and header steps; the working tree carries no staged
  files and only the orchestrator's `docs/tasks/state.yaml` modification.
