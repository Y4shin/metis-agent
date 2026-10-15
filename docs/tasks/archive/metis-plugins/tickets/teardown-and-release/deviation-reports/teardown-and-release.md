---
type: deviation report
title: Deviation report for teardown-and-release
status: stable
---

## Deviation report: teardown-and-release

Compared: the arch spec's `### teardown-and-release` section, the ticket doc's
six acceptance criteria, and the implementation on `ticket/teardown-and-release`
(`git diff task/teardown-and-release..ticket/teardown-and-release`; the landing
branch is `ffd7ae2`). Six files changed, +1309/-11: two source files in
`plugin-runtime`, one in `plugin-fixture`, one new test file with eight cases,
one startup test case, and `docs/testing.md`.

### API surface changes

**1. `Serving::release` is consuming and asynchronous.**
- **Planned:** `Serving::release(deadline: std::time::Instant) -> ReleaseReport`.
- **Actual:** `pub async fn release(self, deadline: Instant) -> ReleaseReport`.
  It consumes the `Serving`, and the await is what lets it drain: the runtime
  hands each `PluginCall` back to the caller and keeps no handle on it, so
  "drop every future" is the caller's own drop and the release can only wait for
  it. The documented order holds and is on the ledger: unregister, `cancel_all`,
  drain to the deadline, the quarantine and in-flight gates, `shutdown`,
  `unload`, then drop the library reference the release kept.
- **Impact:** `sample-bundle-ci` and the daemon call it as an awaited method on
  an owned `Serving`, and their own drop of their calls is what completes the
  drain. A caller that never drops a call gets `ReleaseRefusal::FuturesInFlight`
  at its deadline rather than a hang.

**2. Two new public types carry the report, `ReleaseReport` and `ReleaseRefusal`.**
- **Planned:** the arch spec fixed the signature and described the report's
  content in prose (name, the ledger entries added, whether it was released).
- **Actual:** `ReleaseReport` with `name()`, `deadline()`, `cancelled() ->
  &CancelReport`, `released()`, `refusal() -> Option<&ReleaseRefusal>`,
  `resident() -> Option<&Arc<Loaded>>` and `ledger() -> &[String]`, plus
  `Display`; `ReleaseRefusal` with `Quarantined`, `FuturesInFlight(Vec<u64>)`,
  `Shutdown(CallError)`, `Unload(CallError)`, each with `step()`, `Display` and
  `Error`. `resident()` is what keeps an unreleased instance's library mapped
  after `release` has consumed the `Serving`.
- **Impact:** additive; a dependent reads the report instead of the ledger text
  where it can, and `resident()` is the only remaining handle to a library that
  could not be released.

**3. `Serving::registry_mut` was added, because the planned path could not work.**
- **Planned:** "Disabling and enabling are registry state only, reached through
  each `Serving::registry()`."
- **Actual:** `registry()` is unchanged and still `&self`, and the new
  `registry_mut(&mut self) -> &mut Registry` is the path to disable and enable,
  because `Registry::enable` takes `&mut self`. Criterion 4 was unreachable
  without it.
- **Impact:** additive, and it is the surface the daemon toggles a plugin with.
  `StartupReport::call` keeps using the immutable accessor.

**4. `StartupStop` gained the started plugins, and its derives narrowed.**
- **Planned:** the arch spec assigns this ticket the level-4 gap (a stopped pass
  dropped the `Serving` values it had started, without teardown) and leaves the
  shape open: "whether it adds a release-on-stop path or requires the caller to
  release what it holds".
- **Actual:** `StartupStop::serving() -> &[Serving]` and `into_serving() ->
  Vec<Serving>` hand the started plugins back in start order, with nothing
  unloaded on that path; the derive set narrowed from `Debug, Clone, PartialEq,
  Eq` to `Debug`, with a manual `Debug` for `Serving`.
- **Impact:** a landed type's public surface changed shape, so anything that
  cloned or compared a `StartupStop` breaks (nothing in tree does; the startup
  tests were updated). A caller that handles a stop must now release what it is
  handed. Release-on-stop with an immediate deadline was rejected, and the reason
  is the map's cancellation decision: the deadline is the caller's and the
  runtime holds no timer and no default.

**5. `plugin-fixture` gained one reserved instance name.**
- **Planned:** nothing; the plan named no fixture change.
- **Actual:** `REFUSING_SHUTDOWN` (`fixture-refusing-shutdown`) plus one `Fixture`
  field, whose teardown raises the panic the adapter contains. The authoring
  `stopped` hook is infallible, so a panic is the only route to a shutdown entry
  that answers a status other than `ok`.
- **Impact:** a test fixture change only; a hand-written fault cdylib was
  rejected in favour of reusing the fixture, and the reserved name joins the
  existing `DEFECTIVE_INITIALISATION` convention.

**6. The drain yields rather than spins.**
- **Planned:** "cancel or drain in-flight calls to the caller's deadline".
- **Actual:** `yield_now()` is a `poll_fn` that wakes and returns `Pending` once
  and `Ready` on the next poll, and the deadline is read in the loop condition.
  A loop that awaited nothing would spin the caller's thread for the whole
  deadline, which the supervisor required the implementation to avoid.
- **Impact:** none on dependents; the release stays pollable on any executor,
  including the tests' single-threaded loop, and the crate still starts no
  runtime.

### Abstraction usage

- Used/was specified: **yes**. `Instance::unload`, `Instance::shutdown`,
  `Instance::cancel_all(deadline)` with its `CancelReport`,
  `instance.is_quarantined()`, the ordering ledger, the per-`Serving` registry,
  and `LoadError::UnloadRefused { plugin, step, status }` as the shape a refused
  `destroy_instance` already answers. The deadline is the same
  `std::time::Instant` that `call-cancellation` landed; no second deadline type
  exists.
- The do-not list held in full: no clone counter, no outstanding-clone gate (the
  one waker clone the fixture's `tool.deferred` keeps is outstanding and the
  release unloads anyway), no forced unmap, and no live teardown or
  reconfiguration. Nothing stages a file and no process was left running.

### Out-of-scope changes

- `crates/plugin-runtime/src/startup.rs` (45 lines) and `tests/startup.rs` (81
  lines): the level-4 stop gap. It is outside the ticket doc's stated work and
  inside the arch spec's stated obligation for this ticket, and the supervisor
  chose the hand-over shape (option b) explicitly.
- `crates/plugin-fixture/src/lib.rs`: 28 lines, the reserved shutdown-refusing
  name and its field.
- `docs/testing.md`: the new `cargo test -p plugin-runtime --test teardown`
  command bullet and one sentence on the startup bullet, as the protocol rule
  requires for a new test file.
- No `docs/plugin-contract.md` edit: the arch spec lists no doc edit for this
  ticket, and contract section 11 rule 7 already states the teardown order.
- One commit on the working branch was rewritten with `git reset --soft` so the
  `registry_mut` accessor lands with the criterion-4 test that needs it. No
  landed commit was touched, and the working branch is the only branch affected.
- `docs/tasks/state.yaml` (the orchestrator's pointer) is modified and unstaged,
  as every earlier landing in this effort recorded.

### Findings for a later pass, not blockers

- `crates/plugin-runtime/src/lib.rs:1173` records the ledger line `instance
  destroyed` before it checks what `destroy_instance` answered, so a refused
  destroy leaves a line that is not true before the error is returned. It is
  pre-existing, no in-tree test reaches it, and the release's own refusal line is
  the honest record. Moving the line after the status check is a one-line fix for
  a later pass.
- The refused-`destroy_instance` branch of `Instance::unload` has no test for a
  served instance: the patched library in `tests/fixtures/` fails
  initialisation, so it cannot produce a `Serving`.
- The ordering ledger carries no instance attribution, so a report's ledger slice
  could include concurrent activity on the same host. Every test here gives each
  release its own host, where the slice is exact.
- The subprocess fault test asserts `SIGSEGV` and is Unix-gated: off Unix it
  prints `[skip]` and returns, following the pipe test's convention.
- Four of the nine cases had no failing first run, because the whole refusal set
  landed with criterion 1; sensitivity was proved by mutation instead (removing
  the drain loop fails criteria 1 and 5, removing the quarantine gate fails 3 and
  5). Recorded rather than claimed as RED-first, matching the `call-cancellation`
  precedent.

### Ticket doc update needed?

Yes. The land step's note should append to `## Implementation notes`: the
deadline-type continuity with `call-cancellation`, the stop hand-over with the
rejected release-on-stop and the caller-owned-deadline reason, `StartupStop`'s
derive narrowing as a public-surface change for the reconcile pass, the
`registry_mut` addition with the reason the planned accessor could not serve
criterion 4, and the four non-RED-first cases.

### User attention needed?

Yes, one sentence for the record. The release entry is complete and verified, and
this ticket also closed the level-4 stop gap the arch spec assigned it: a stopped
pass now hands its started plugins back for the caller to release instead of
dropping them, which narrowed a landed type's derives and added two public types
plus a mutable registry accessor, so `sample-bundle-ci` and the daemon consume a
wider surface than the ticket doc listed.
