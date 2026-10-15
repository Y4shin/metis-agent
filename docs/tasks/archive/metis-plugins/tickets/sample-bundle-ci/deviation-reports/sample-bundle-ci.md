---
type: deviation report
title: Deviation report for sample-bundle-ci
status: stable
---

## Deviation report: sample-bundle-ci

Compared: the `sample-bundle-ci` section of `docs/tasks/metis-plugins/arch-spec.md`, the ticket doc and its five acceptance criteria, and the implementation on `ticket/sample-bundle-ci` (`git diff task/sample-bundle-ci..ticket/sample-bundle-ci`, three `wip:` commits, tip `cb03433`, base `bc61feb`). Six files changed, +724/-10, and no file under `crates/plugin-runtime/src/` is touched.

### API surface changes

**1. The packaging step is a test-support builder, not `scripts/pack-sample-bundle.sh`.**

- **Planned:** "A packaging step (`scripts/pack-sample-bundle.sh` or an equivalent test-support builder) writes a signed bundle holding the C fixture's library and a hand-written manifest, signed with the test key."
- **Actual:** `crates/plugin-runtime/tests/support/sample.rs`, a test module. It builds the C fixture with the script the ABI harness runs, assembles the two entries through the shared archive writer, and signs them with the shared test signer's fixed key. The bundle is written to a temporary file that is removed when its guard drops rather than committed as an artifact.
- **Impact:** none on the pipeline, and the choice is defensible (the signing key lives in the test signer, so a shell script would need a second signer or a helper binary). The consequence to record is that there is no producer-facing packaging command in the tree: the SDK CLI remains the only packer for a real author, and a future consumer that wants the sample bundle on disk must call the test support module or write its own packer.

**2. The C fixture gained one event emission.**

- **Planned:** the ticket names the C fixture as the sample plugin and the arch spec expects the gate to read "the events its own sink recorded". Nothing in the plan said the fixture had to change.
- **Actual:** `crates/plugin-fixture/c/c-plugin.c` gains one static helper `emit_ready_event` and one call from the ready capability's poll, so `c.ready` emits the topic `cplugin.ready` with the body `{"ready":true}`. The call is guarded by the same presence-by-size rule the file already applies to `service_request`, and the payload is released through the plugin's own `release_buffer` entry.
- **Impact:** criterion 2 was unreachable without it, because the C fixture called `service_request` and `log` and never `emit_event`. The ABI harness is unaffected: `[result] PASS (82 checks)`, with `cplugin.ledger_zero_live` reporting allocs 6, frees 6, live 0, and the file still compiles under gcc and clang with `-Werror`. This is the one change outside the file set the ticket named, and the one a reviewer could reasonably reject; if it is rejected, criterion 2 should be read against the service request the `c.timer` capability already raises, the hunk reverted, and the gate case made to dispatch `c.timer` instead of `c.ready`.

**3. No runtime source file changed at all.**

- **Planned:** "The end-to-end gate, not new runtime surface."
- **Actual:** confirmed. The diff carries no `crates/plugin-runtime/src/` file, no `Cargo.toml`, no `Cargo.lock`, no contract text, no generated header and no proto. The gate calls only landed public entries: `load_bundle`, `Host::with_sink`, `Serving::registry()` and `Serving::release`.
- **Impact:** every dependent surface is consumed exactly as landed, so the composed-path claim is a claim about the landed code rather than about new code. The one new public-ish name is `sample::NAME` and the `sample::` constants, which are test-support items and reach no consumer outside the test binary.

**4. The test seam widened by one public entry.**

- **Planned:** "The single end-to-end seam, which is the public load entry plus the sink plus the release entry."
- **Actual:** four of the five cases read that seam. The fifth, `the_test_key_is_not_a_compiled_in_root`, also calls `plugin_runtime::bundle::verify_bundle` directly, because the claim it makes is about the compiled-in root set, which no load path can observe without a bundle it accepts.
- **Impact:** none. `verify_bundle` is a landed public entry from `bundle-verification`, not a raw table or a private detail, and the load-based cases already assert the same trust refusal end to end.

**5. The gate pins exact literals for the fixture's response, event and ABI demand.**

- **Planned:** the plan fixed the entries and the ledger assertion, not the fixture's payloads.
- **Actual:** the manifest declares `min_abi_minor = 2`, the response is asserted against `{"echo":"c.ready","bytes":5}`, the event against `{"ready":true}`, and the release ledger against four substring positions (`unregistered < quiesced < destroyed < library unloaded`).
- **Impact:** a later change to the ready capability's response, to that event's body, or to the ABI minor breaks this gate, which is what an end-to-end gate is for. The ledger assertion is substring-based, so a reworded ledger line breaks the gate rather than the check silently weakening; that is worth knowing at the reconcile.

**6. The gate is Linux-shaped and CI runs it on Linux only.**

- **Planned:** "The gate runs in CI as a step that fails the pipeline, alongside the existing contract and ABI checks."
- **Actual:** `.github/workflows/ci.yml` gains a `Sample bundle gate` step in the `check` job, directly after the ABI header regeneration check and before the `no-panic` step, running `devenv shell -- cargo test -p plugin-runtime --test sample_bundle`, which fails the job on any failure. The packaging helper hard-codes `libcplugin.so`, and the `windows` job compiles `-p api-contracts -p daemon --all-targets`, which does not build `plugin-runtime`'s tests.
- **Impact:** the composed bundle path is proven on Linux. The Windows plugin line stays compile-only, which is the state the MSVC ticket left it in, and the `.so` assumption is inherited from the ABI harness rather than introduced here.

**7. The bundle file is not named after its plugin.**

- **Planned:** the arch spec says the sample file is named after its plugin, as a condition of a gate that also drives `start_plugins`.
- **Actual:** the bundle is written through the shared `BundleFile` helper, whose name is `metis-load-test-<pid>-<nanos>-<n>.metisbnd`, and the manifest name is `cplugin`. The gate does not drive `start_plugins`, so the stem rule is not exercised.
- **Impact:** the conditional in the plan does not apply to what landed. The note to carry is that a later ticket reusing this helper for a `start_plugins`-driven gate would register the plugin under `metis-load-test-...` and must rename the file or the helper first.

### Abstraction usage

- Used/was specified: **yes**. The gate reuses the shared test signer (`signer::signed_bundle`, `standard_specs`), the archive writer and its helpers (`bundle::archive`, `host_target()`, `library_path`, `MANIFEST`, `BundleFile`), the C fixture's build script, and the runtime's support helpers (`block_on`, `repository_root`). No second packaging format, no second loader, no SDK, and no third derivation of `<os>-<arch>`: the sample module calls the shared `bundle::host_target()`. The hand-written manifest is a literal in the support module, not a serialisation of any type in the tree, which is the point of using the C fixture as the sample plugin, and the capability list is a literal too, so the manifest-versus-library disagreement is a real disagreement rather than a list read out of the library.
- The do-not list held: `docs/plugin-contract.md`, the generated ABI header, `buf.yaml`, the proto tree and `Cargo.toml` are untouched, and the C fixture is still packaged with a hand-written manifest rather than a generator.
- Coverage honesty: the gate case was genuinely RED first (it ran the whole path and failed on the missing event, which confirmed the packer, manifest cross-check, routing and release ledger before the fixture change), while the manifest-disagreement, altered-byte, unrooted-key and empty-root-set cases passed on their first run because they exercise landed behaviour. They are the criteria's coverage rather than new production code, and the tdd result records that rather than claiming five RED cycles.
- One pre-existing duplication this ticket did not add to: a test-side copy of the runtime's private `<os>-<arch>` derivation still exists in `crates/plugin-runtime/tests/support/bundle.rs` and `crates/plugin-runtime/tests/manifest.rs`, while `crates/plugin-runtime/src/load.rs` derives the same string privately. The sample module reuses the test-side copy rather than adding a fourth spelling.

### Out-of-scope changes

- `crates/plugin-fixture/c/c-plugin.c`: the event emission described in item 2, plus its doc comment. Outside the ticket's named file set, forced by criterion 2, and the only product-code change in the diff.
- `crates/plugin-runtime/tests/support/mod.rs`: one line, `pub mod sample;`.
- `crates/plugin-runtime/tests/support/sample.rs` and `crates/plugin-runtime/tests/sample_bundle.rs`: the packaging step and the gate, both new.
- `.github/workflows/ci.yml`: the new CI step, in scope per criterion 5.
- `docs/testing.md`: the command as number 12, a run-commands block in the `Plugin host` section describing what one run proves, and the CI paragraph updated to name the sample gate before the `no-panic` check. The paragraph's "three contract commands and the five plugin ABI commands" count is still accurate, because the sample gate is a sixth step rather than one of the five.
- Nothing else. `Cargo.toml`, `Cargo.lock`, `docs/plugin-contract.md`, the generated header and the proto tree are untouched, and no dependency was added.
- Present in the worktree but not this ticket's: `docs/tasks/metis-agent-core/map.md` (modified), `docs/tasks/metis-agent-core/tasks/` (untracked) and `docs/tasks/state.yaml` (the orchestrator's pointer). None of them is committed on this branch, and nothing is staged.

### Ticket doc update needed?

Yes. The ticket doc has no `## Implementation notes` section yet, so the landing note should create it and record:

- The four divergences above that a reader needs: the test-support packer instead of a script; the C fixture's event emission and the alternative reading of criterion 2 if that hunk is rejected; the manifest-versus-library disagreement asserted through the load entry rather than inside the packer (the runtime exposes no capability-declaration read without an initialised instance); and the dotless `cplugin` name with a bundle file named by the shared helper rather than by the plugin.
- The mechanics: base `bc61feb` (`teardown-and-release`'s landing), three `wip:` commits, tip `cb03433`, six files, +724/-10, nothing staged.
- The gate evidence: `cargo test -p plugin-runtime --test sample_bundle` reports `5 passed; 0 failed` at `cb03433`, which this report reproduced; the tdd result also records the same command stable over four runs and after `rm -rf target/c-plugin`, and `cargo run -p plugin-fixture --bin abi-harness` at `82 checks PASS` with the fixture's allocation ledger at 6/6/0.
- The residuals the land note should carry: the gate is Linux-shaped and CI runs it on Linux only; the packaging helper hard-codes `libcplugin.so`, which the ABI harness already does; the C plugin's ledger is process-wide static state shared by the two gate cases that load it in parallel, and nothing in the gate asserts its numbers; and `LoadError::Registration` stays untested.

At the reconcile, the effort's arch spec should also record that `start_plugins`' file-stem naming rule is not exercised by this gate, and that this ticket added no `scripts/` command.

### User attention needed?

Yes, one sentence. Acceptance criterion 2 required a change to the C fixture, which the ticket's file set did not name: the fixture emitted no event at all, so "the gate observes an event through the sink" was unreachable until `c.ready` gained one. The change is one guarded call plus a helper, the ABI harness and its allocation ledger are unaffected, and the alternative reading is recorded in item 2 if the change is unwanted. Everything else in this diff is test support, a CI step or documentation, with no consumer outside this effort.
