---
type: ticket
subtype: bug
title: Fix the waker lifecycle defects the hazard prototype measured
status: stable
workflow_state: done
blocked_by: []
size: m
---

## What to build

The authoring bridge's waker lifecycle is correct, so a plugin authored with it
can be polled, cancelled and dropped without leaking or over-releasing a call's
shared state. Three measured defects go: the value handed to the plugin's poll
leaks one reference per call because it is never released, the wake-after-drop
record cannot name the call it belongs to, and a second wake on one consumed
clone over-releases the call state. A wake after the future was dropped is
recorded as a counter rather than an ordering-ledger entry, and the contract says
so.

## Acceptance criteria

- [ ] A call whose future is dropped releases every reference it took, and the
      fixture's reference counts balance after a full call, a cancelled call and
      an abandoned call.
- [ ] A wake delivered after the future was dropped is recorded and names the
      request id it belongs to.
- [ ] A second wake on one consumed clone does not release the call state twice.
- [ ] The contract states that a wake after drop is a counted violation rather
      than an ordering-ledger entry.
- [ ] The fixture, the ABI harness and the workspace test suite pass.

## Blocked by

- None (can start immediately).

## Implementation notes

Landed on `task/waker-lifecycle-fix` as the merge commit `dd7e1bf`
("ticket(waker-lifecycle-fix): Fix the waker lifecycle defects the hazard
prototype measured"), parents `78d65f0` and `cefc602`. The landing branch did not
exist when this ticket was landed, so it was created at `main`'s tip (`78d65f0`,
"docs(metis-plugins): add and approve the effort architecture spec"), following
the convention recorded when `event-stream` landed. `cefc602` is the working
branch `ticket/waker-lifecycle-fix`, five `wip:` commits, one per acceptance
criterion. The merge was clean with no conflicts, and the landing tree is
byte-identical to the verified `cefc602` (`git diff cefc602 HEAD` is empty), so
every source file landed exactly as verified. The branch
`ticket/waker-lifecycle-fix` was deleted after the merge.

The verifier ran against `cefc602`: `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` clean, `cargo test` 46 suites and
223 tests passed with no failures, `cargo run -p plugin-fixture --bin
abi-harness` PASS (73 checks), `bash scripts/build-c-plugin.sh` and
`bash scripts/regenerate-abi-header.sh --check` clean, and
`cargo check -p plugin-contract --no-default-features` clean. All five
acceptance criteria were checked against the code and hold.

The worker recorded these divergences from the arch spec, and the verifier
agreed they are not blockers:

- The host release for each poll is `PluginCall::poll`'s own
  `release_waker`, so `crates/plugin-contract/src/authoring/{call.rs,future.rs}`
  needed no change, contrary to the spec's file list. The spec's interface
  contract requires a host-side release, and a plugin-side release would break
  the ABI harness's own `waker_clones() == waker_drops()` invariant.
- `Host::waker_counts()`'s first element changed meaning from "clones the plugin
  made" to "every owning waker reference the host handed out". The tuple shape
  is unchanged, and the criterion was not observable under the old meaning,
  which masked the leak. `Host::wakes_after_drop()` is new, and
  `docs/tasks/metis-plugins/tasks/abi-hazard-fixtures/findings.md` still
  describes the old meaning at line 74 and needs a later reconciler pass.
- A `MetisWaker` now carries a host-side token in its `data` field instead of a
  raw `Arc<CallState>` pointer. The ABI is unchanged (opaque field, no layout
  change, no minor bump); the new host state is a process-wide live-waker table.
- The ABI doc comment in `crates/plugin-contract/src/abi/mod.rs` and the
  regenerated header carry the corrected lifetime claim, alongside the contract
  document that criterion 4 names.
- Two fixture capabilities were added (`tool.deferred`, `tool.wake_deferred`) and
  the harness's expected capability list grew from 8 to 10.
- The double wake is provoked by a `#[cfg(test)]` unit test in
  `crates/plugin-runtime/src/lib.rs` rather than through the authoring surface,
  which cannot express it.

`docs/tasks/state.yaml` was already modified in the working tree before this
ticket started (`task: null` to `task: waker-lifecycle-fix`, the workflow
runner's own file). It was left unstaged and untouched by this landing, so the
landing commit carries only this implementation note.

### Knowledge harvest

The CI gate was re-run on the landing branch at `b30efde`, after the landing
note above, and passes: `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, `cargo test`, `buf lint`, `buf breaking`, `bash
scripts/regenerate-contracts.sh --check`, `bash scripts/build-c-plugin.sh`,
`cargo check -p plugin-contract --no-default-features`, `cargo run -p
plugin-fixture --bin abi-harness` (PASS, 73 checks), and `bash
scripts/regenerate-abi-header.sh --check`.

Durable lessons folded into the project docs: `docs/testing.md` already
describes the waker accounting and the two new fixture capabilities, and
`docs/tasks/metis-plugins/tasks/abi-hazard-fixtures/findings.md` now carries a
post-fix note that the first element of `Host::waker_counts()` counts every
owning reference the host handed out, so its prototype numbers stay readable.
The one obligation this fix left standing is for the second host
implementation: the ABI harness still does not release the owning waker
reference its poll is given, and its `waker_wake` still prints an ordering line
for a dead slot. `abi-initialisation-entry` rewrites that harness, so the
obligation is recorded in the effort's arch spec as carried work and belongs
with that ticket.
