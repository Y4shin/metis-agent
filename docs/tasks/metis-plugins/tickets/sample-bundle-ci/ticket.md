---
type: ticket
subtype: feature
title: The sample plugin loads end to end in CI
status: stable
workflow_state: ready
blocked_by: [callback-sink, teardown-and-release]
size: m
---

## What to build

The sample plugin gate. A packaging step writes a signed bundle containing the C
fixture's library and a hand-written manifest, signing it with a test key, and an
integration test loads that bundle through the same entry the daemon uses,
dispatches a capability, observes an event through the sink, and releases the
instance cleanly with the ledger asserting the order. The gate runs as a CI step
that fails the pipeline. The daemon's compiled-in root set stays empty, so no
shipped build trusts the test key.

## Acceptance criteria

- [ ] The packaging step produces a signed bundle from the C fixture's library and
      a hand-written manifest, and it fails loudly when the manifest and the
      library disagree.
- [ ] The integration test loads the bundle through the public load entry,
      dispatches a capability, receives a response, observes at least one event,
      and releases the instance with the ledger asserting the teardown order.
- [ ] The same test fails when one byte of the bundle is altered and when the
      chain is signed by a key that is not a configured root.
- [ ] The test key is used only by the tests, and the daemon's default root set is
      empty.
- [ ] The gate runs in CI as a step that fails the pipeline, alongside the
      existing contract and ABI checks.

## Blocked by

- `callback-sink`
- `teardown-and-release`

## Implementation notes

The gate landed as `crates/plugin-runtime/tests/sample_bundle.rs` (five cases)
over the three public entries `load_bundle`, `Serving::registry().call` and
`Serving::release`, with the packaging builder in
`crates/plugin-runtime/tests/support/sample.rs`. The builder compiles the C
fixture through `scripts/build-c-plugin.sh` once per process behind a mutex,
assembles the archive the contract fixes (`manifest.toml` at the root,
`lib/<os>-<arch>/libcplugin.so` at mode `0600`, no directory entry), signs it
with the shared test signer's fixed key, and writes it to a temporary file that
is removed on drop. `.github/workflows/ci.yml` gained a `Sample bundle gate` step
after `Plugin ABI header regeneration check`, and `docs/testing.md` records the
same command as protocol number 12. Working branch `ticket/sample-bundle-ci`
(base `bc61feb`, tip `cb03433`, three `wip:` commits) merged into
`task/sample-bundle-ci` as `1485188`, six files, +724/-10.

Four divergences a reader needs. First, the packaging step is a test-support
builder, not `scripts/pack-sample-bundle.sh`: the test key lives in the test
signer, so a shell script would need a second signer or a cargo helper binary,
and a committed bundle would go stale against the freshly compiled library.
This is the "equivalent test-support builder" the arch spec allows. Second, the
manifest-versus-library disagreement is refused by the load entry's landed
capability cross-check rather than inside the packer, because the runtime
exposes no capability-declaration read without an initialised instance; both
directions are asserted as `LoadError::CapabilityMismatch` naming the capability
that differed. Third, `crates/plugin-fixture/c/c-plugin.c` gained one guarded
`emit_event` call plus a static helper, because the fixture emitted no event at
all and criterion 2 was otherwise unreachable; the ABI harness stays at 82 checks
PASS with its allocation ledger at 6/6/0 and the C file still compiles clean
under gcc and clang. A reviewer who rejects that hunk should read criterion 2 as
satisfied by the service request the `c.timer` capability raises and revert it.
Fourth, the sample bundle's name is the dotless `cplugin`, the fixture's own
name, with the file named by the shared `BundleFile` helper, so `start_plugins`'
file-stem naming rule is not exercised here and the dotted-name rule stays covered
by `tests/registry.rs`. No runtime API, dependency, contract file or generated
artifact changed.

Only the gate case was genuinely red first: it ran the whole path and failed on
the missing event. The other four cases exercise landed behaviour and passed on
their first run; they are the criteria's coverage rather than new production
code. Verification recorded `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` clean, `cargo test -p plugin-runtime
--test sample_bundle` at 5 passed, and the full workspace suite at 368 passed
across 56 targets. The gate was re-run on the landing branch at `1485188` after
the merge and reports the same 5 passed; `git rev-parse --verify
refs/heads/task/sample-bundle-ci` resolves.

Residuals carried forward. The gate is Linux-shaped: the builder hard-codes
`libcplugin.so`, the same assumption the ABI harness already makes, and the
`windows` CI job does not compile `plugin-runtime`'s tests, so the gate runs on
Linux only. The C plugin's ledger is process-wide static state shared by the two
gate cases that load it, and nothing in the gate asserts its numbers.
`LoadError::Registration` stays untested, as the arch spec already records. The
SDK CLI remains the only producer-facing packer, since no command in the tree
packages the sample bundle on its own.

The effort's `docs/tasks/state.yaml` pointer and the unrelated
`docs/tasks/metis-agent-core/` changes in this shared worktree are left
uncommitted: this landing commit stages only this ticket's doc directory and its
deviation report. The ticket's `workflow_state` and the archive stay with
`finalize-task`.
