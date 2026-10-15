---
type: ticket
subtype: feature
title: One release entry, quarantine and shutdown reporting
status: deprecated
workflow_state: done
blocked_by:
- async-initialisation
- call-cancellation
size: m
---

## What to build

One asynchronous release entry: unregister the instance, cancel or drain in-flight
calls to the caller's deadline, call the plugin's shutdown entry, drop every
future, destroy the instance, unload the library, and report the ordering-ledger
entries it added. An instance that cannot be released stays mapped and is
reported, which covers a shutdown that returns a non-ok status, a quarantined
instance whose abandoned poll never returns, and an unload refused for any reason.
Disabling and enabling are registry state only and touch nothing else. The
daemon's shutdown deadline ends the wait and produces the report. The
drop-after-unload fault is proved in a subprocess, and no outstanding-clone gate is
added.

## Acceptance criteria

- [ ] The release entry performs the documented order and the ledger proves it,
      including the case where in-flight calls had to be cancelled first.
- [ ] A shutdown that returns a non-ok status, and an unload refused for another
      reason, both leave the library mapped and appear in the report.
- [ ] A quarantined instance keeps its library resident and is reported at
      shutdown.
- [ ] Disabling refuses new calls without touching in-flight calls or the library,
      and enabling accepts calls again with no reload.
- [ ] The daemon's shutdown deadline ends the wait, and the report names every
      instance that could not be released.
- [ ] The drop-after-unload fault is reproduced in a subprocess, and the release
      entry still has no outstanding-clone gate.

## Blocked by

- `async-initialisation`
- `call-cancellation`

## Implementation notes

Landed on `task/teardown-and-release` as the merge commit `2e28431`
("ticket(teardown-and-release): One release entry, quarantine and shutdown
reporting"), parents `ffd7ae2` (the landing branch tip, the `startup-policy`
finalize commit) and `f784b74` (the working branch `ticket/teardown-and-release`,
ten `wip:` commits from `099dda7` to `f784b74`). The merge was clean with no
conflicts and the landing tree is byte-identical to the verified `f784b74`
(`git diff f784b74 HEAD` is empty), so every source file landed exactly as
verified. The branch `ticket/teardown-and-release` was deleted after the merge.

The ticket diff over `ffd7ae2` is six files, 1309 insertions and 11 deletions:
the new `crates/plugin-runtime/tests/teardown.rs` (842 lines, 8 cases, one of
them the ignored child the subprocess fault test spawns),
`crates/plugin-runtime/src/load.rs` (+289/-1, `Serving::release`,
`ReleaseReport`, `ReleaseRefusal` and `Serving::registry_mut`),
`crates/plugin-runtime/src/startup.rs` (+41/-4, `StartupStop::serving()` and
`into_serving()`, the release of the started servings and the narrowed derives),
`crates/plugin-runtime/tests/startup.rs` (+80/-1, the one stop-handover case),
`crates/plugin-fixture/src/lib.rs` (+28/-4, the `REFUSING_SHUTDOWN` reserved
instance name) and `docs/testing.md` (+29/-1, the teardown command bullet and the
sentence the stop handover adds to the startup bullet). This landing commit also
adds the note below and the ticket's deviation report at
`docs/tasks/metis-plugins/tickets/teardown-and-release/deviation-reports/teardown-and-release.md`,
as every previous landing in this effort did.

Verification ran against `f784b74` and, after the merge, again on the landing
tree, which is that same tree: `cargo test -p plugin-runtime --test teardown` ->
8 passed, 0 failed, 1 ignored (the ignored case is the faulting child, which the
`a_copied_handle_dropped_after_unload_faults_the_child` case runs in a subprocess
of its own test binary and reads as the signal the child died with). The
verifier's full record on `f784b74` is `cargo fmt --check` clean, `cargo clippy
--all-targets -- -D warnings` clean (only the pre-existing `_FORTIFY_SOURCE`
note from `plugin-contract`'s C build script), `cargo test -p plugin-runtime
--test startup` 22 passed and 0 failed, and the whole workspace `cargo test`
green. All six acceptance criteria hold, read against the code and those runs;
the boxes above stay unchecked, as in every previous landing in this effort.

Durable notes for the tickets that follow. `Serving::release(deadline)` is the
one teardown entry, and it is an `async fn` that consumes the `Serving`: the
runtime hands each `PluginCall` back to the caller and keeps no handle on it, so
"drop every future" is the caller's own drop and the release cancels the calls it
finds and waits (a self-waking `yield_now` loop with the deadline read in its
condition) until that drop happens or the deadline passes. The runtime arms no
timer and holds no default deadline, which is the level-4 cancellation decision;
the deadline is the same `std::time::Instant` that `call-cancellation` landed.
A caller that never drops its call gets `ReleaseRefusal::FuturesInFlight` at its
deadline rather than a hang. Because the runtime owns no deadline, a stop cannot
release for the caller, so a stopped start-up pass hands the plugins it started
back through `StartupStop::serving()` and `into_serving()` and unloads nothing
automatically; a release-on-stop with an immediate deadline was considered and
rejected for that reason. `StartupStop`'s derive narrowing (`Clone`,
`PartialEq` and `Eq` dropped, `Debug` kept through a manual `Debug` for
`Serving`) is a public-surface change for the reconcile pass to pick up.
`Serving::registry_mut()` is the additive path to disable and enable, which the
landed immutable `registry()` cannot serve; `registry()` is unchanged.
`ReleaseRefusal::step()` names `unload` for a quarantined instance, because the
step a quarantine refuses is the unload, and `drain` for calls still in flight;
the arch spec named no step vocabulary, so this is the chosen wording.
`ReleaseReport::resident()` is the library reference the release keeps when the
instance could not be released, so that instance's library stays mapped after
the report is read. The fixture's `REFUSING_SHUTDOWN` name is the only route to a
shutdown entry that answers a status other than `ok`, because the authoring
`stopped` hook is infallible.

Residual risks carried forward. The refused-`destroy_instance` branch of
`Instance::unload` has no test, because no library in this tree refuses destroy
for a served instance (`tests/fixtures/refusing_destroy.rs` fails initialisation,
so it can never produce a `Serving`); the release reports it as
`ReleaseRefusal::Unload` and records its own line for it. `Instance::unload`
records the ledger line `instance destroyed` before it checks what the destroy
entry answered, so a refused destroy leaves a line that is not true; the branch
is pre-existing, no in-tree test reaches it, and a later pass should move that
line after the status check. `ReleaseReport::ledger()` is the host's ordering
from the index the release started at, and the ordering ledger carries no
instance attribution, so concurrent releases on one host could appear in one
report; every test in this ticket gives each release its own host, where the
slice is exact. Four of the eight cases had no failing first run, because the
release entry landed with its whole refusal set in criterion 1's slice: criteria
2 (both halves), 3, 5 and criterion 6's clone-gate half. Sensitivity was proved
by mutation instead, removing the drain loop fails criteria 1 and 5 and removing
the quarantine gate fails criteria 3 and 5, matching the arch spec's precedent
for `call-cancellation` criterion 1. The subprocess fault test asserts `SIGSEGV`
and is Unix-gated, so off Unix it prints `[skip]` and returns, following the pipe
test's convention; on this Linux host it ran for real, and the child's marker
line proves the fault happens after the unload. The fixture's panic route prints
one line through the fixture's own panic hook on the shut-down case, as the
fixture's deliberate capability panics already do.

`docs/tasks/state.yaml` carries this effort's orchestration task pointer (`map:
metis-plugins`, `task: teardown-and-release`) and is left uncommitted and out of
this landing commit, as in the earlier landings, because `finalize-task` owns it.
The working tree's `docs/tasks/metis-agent-core/` changes belong to another
effort sharing this worktree, so this landing commit stages only this ticket's
doc directory and its deviation report; those stay uncommitted and untouched.
The ticket's `workflow_state` and the archive stay with `finalize-task`.

### Knowledge harvest

The CI gate was re-run on the landing branch at `bc61feb`: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, `buf lint`, `buf
breaking`, `bash scripts/regenerate-contracts.sh --check`, `bash
scripts/build-c-plugin.sh`, `bash scripts/check-c-layout-asserts.sh`, `cargo
check -p plugin-contract --no-default-features`, `cargo run -p plugin-fixture
--bin abi-harness` (82 checks), `bash scripts/regenerate-abi-header.sh --check`
and `cargo test --profile no-panic -p plugin-runtime --features no-panic-check
--no-run` all pass. Protocol command 12 (`cargo test -p plugin-runtime --test
sample_bundle`) does not exist on this branch, because that test file lands with
`sample-bundle-ci` on top of it; running it here answers `no test target named
sample_bundle`, which is an ordering fact rather than a failure.

Durable lessons for a later consumer. The deadline is the caller's, so the
release is awaited by whoever owns the clock and the drain is the caller's own
drop of its `PluginCall` futures; a caller that keeps a call alive reads
`ReleaseRefusal::FuturesInFlight` at its deadline rather than hanging. The
report's `resident()` is the only handle that keeps an unreleased library
mapped, so a daemon that reports an unreleasable plugin must hold the report.
A start-up pass that stops hands its started `Serving` values back rather than
releasing them, because an immediate deadline inside the pass would decide a
deadline the runtime does not own.
