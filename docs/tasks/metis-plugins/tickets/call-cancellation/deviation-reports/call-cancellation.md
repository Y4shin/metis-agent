---
type: deviation report
title: Deviation report for call-cancellation
status: stable
---

## Deviation report: call-cancellation

Compared: the arch spec's `call-cancellation` section, the ticket doc and its six
acceptance criteria, and the implementation on `ticket/call-cancellation`
(`git diff task/call-cancellation..ticket/call-cancellation`: eight `wip:`
commits, tip `385e538`, over the landing branch tip `c3e361e`). The changed set
is `crates/plugin-runtime/src/lib.rs` (+163/-4), the new
`crates/plugin-runtime/tests/cancellation.rs` (422 lines, six cases) and
`docs/testing.md` (+19). Nothing else changed, so the ABI, the status space and
`docs/plugin-contract.md` are untouched.

Every claim below was read from the diff and the source, not from the TDD
worker's summary; where the two disagree the source wins and it is said so.

### API surface changes

**1. The deadline type is `std::time::Instant`, and the plan named none.**

- **Planned:** `Instance::cancel_all(deadline) -> CancelReport`, with the
  deadline untyped, and the rule that the caller owns it and the runtime holds no
  timer and no default.
- **Actual:** `pub fn cancel_all(&self, deadline: Instant) -> CancelReport`,
  where `Instant` is `std::time::Instant`. The deadline is named back in the
  report rather than armed, and the function is synchronous: there is no
  `async fn`, no future and no blocking wait anywhere in the path.
- **Impact:** `teardown-and-release` plans `Serving::release(deadline)` and must
  take the same type, or a coherence pass is needed. `startup-policy` and the
  daemon must compute `Instant::now() + duration`; `Instant` is a monotonic clock
  reading, so nothing outside the process can carry it and no duration belongs in
  it. The ABI harness already uses `Instant`, so the crate now has one deadline
  representation rather than two.

**2. Two new public types, with an ordering the plan did not state.**

- **Planned:** `CancelReport` as the return of the instance-wide loop, "and
  reports what it did".
- **Actual:** at the crate root,
  `CancelReport { deadline: Instant, cancelled: Vec<u64>, refused: Vec<RefusedCancel> }`
  and `RefusedCancel { request_id: u64, status: MetisStatus }`. `cancelled` is
  sorted ascending by request id, so one state yields one report; `refused`
  carries the frozen status each id answered, which today can only be `NotFound`
  when a call leaves the in-flight set between the loop's snapshot and the mark.
- **Impact:** `teardown-and-release` and `startup-policy` read the report, and
  the ascending order is part of the surface now. A dependent must not assume the
  report is exhaustive: it cancels a snapshot of the in-flight set, so a call
  started after the pass is not in it, and the caller's loop is what owns that.

**3. `Instance::cancel` and `Instance::complete_service` are now gated on the
instance's own in-flight set.**

- **Planned:** "Cancelling an unknown or finished id answers the frozen not-found
  status without calling the plugin" and "no completion is ever delivered into a
  released context", both stated in prose.
- **Actual:** both methods answer `Err(CallError::Status(MetisStatus::NotFound))`
  before entering the plugin when the id is not in this instance's in-flight set.
  `cancel` keeps a second cancel of a live id accepted, and the token-marking
  step is now the private `mark_cancel`, whose error type is the frozen
  `MetisStatus` alone.
- **Impact:** this is a behaviour change, not only a refusal added. The plugin's
  call registry is process-wide, so before this change an id held by *another*
  instance reached the plugin and could mark that instance's call; the ticket's
  own red run is exactly that case, and it is now refused by the host. Three
  dependents must follow it: `callback-sink` answers services through
  `complete_service` and will get `NotFound` for an already-released call, which
  it must treat as the expected refusal rather than an error to report;
  `teardown-and-release` cancels or drains in-flight calls and will see
  `NotFound` for ids that finished during its pass; and the daemon's own cancel
  surface answers `NotFound` for an id it does not hold, which the effort map's
  cancellation decision already requires.

**4. A new crate-private host entry, `Host::cancel_services`.**

- **Planned:** the drop rule on `PluginCall`: "release every buffer the host
  still holds for the call and cancel its outstanding service requests".
- **Actual:** `pub(crate) fn cancel_services(&self, request_id: u64) ->
  Vec<PendingService>` removes the host's pending service records for that call
  and returns them, so the only caller (the call's own `Drop`) releases them.
  `PendingService` is unchanged; its `Clone` derive predates this ticket, and its
  `body` is a `Vec<u8>` copy the host owns, so releasing one drops a copy rather
  than a plugin-allocated buffer.
- **Impact:** no public surface is added, and the daemon cannot reach it. A
  reviewer who reads "every buffer the host still holds" more widely than the
  host-held service-request copies should challenge item 6 below.

**5. Two new ordering-ledger line kinds, with wording this ticket fixes.**

- **Planned:** the ledger carries the unacknowledged cancel.
- **Actual:** two lines, each only when it says something:
  `call <id> released <n> service request(s) the host still held` for `n > 0`,
  and `call <id> cancel unacknowledged: the plugin never resolved it as
  cancelled`. The second needs both a plugin-accepted cancel and no poll that
  observed the terminal `Cancelled` status, which is the new
  `PluginCall::acknowledged` flag; a drop order is release, then the
  unacknowledged line, then the plugin future's drop, then the reclaim line.
- **Impact:** `teardown-and-release` reports the ledger entries its own pass
  adds, so it will see both, and this ticket's tests are what fix their wording.
  A dependent that matches ledger text must match these strings.

**6. "Every buffer the host still holds" is read as the host-held
service-request copies.**

- **Planned:** the drop "releases every plugin buffer the host still holds for
  that call before the context is reclaimed".
- **Actual:** the release is the drop of the host's `PendingService` copies, one
  per outstanding service request, and nothing else is released because nothing
  else is held: per contract section 11 rule 1 every plugin-allocated buffer is
  copied and released at the callback that carried it, so a live call leaves no
  plugin buffer in the host's hands.
- **Impact:** the outcome claimed by the criterion holds either way, but the
  wording promises a wider release than any code can perform. This is a wording
  defect in the plan rather than an implementation deviation, and it is worth
  correcting in the contract or the arch spec so a later reader does not hunt for
  a missing release. No dependent is affected.

**7. Contract and spec documents: no edit, and the host-side rule has no home in
the contract.**

- **Planned:** the arch spec's Exports name no document edit; the ticket doc's
  criteria name none either.
- **Actual:** `docs/plugin-contract.md` is untouched. The contract's cancel text
  (section 10's flow and section 5's `cancel` row) says what the plugin does when
  it is asked, and it does not state that the host refuses an id it does not hold
  before entering the plugin, which is now a real rule of the host side.
- **Impact:** the rule is recorded in the effort map, the effort spec and the
  code, so nothing is lost, but the contract is the artifact a second
  implementation reads. `startup-policy` is the next ticket that edits that file,
  so one sentence there would close the gap; this is a documentation placement
  question, not a behaviour deviation.

### Abstraction usage

- Used/was specified: **yes**. The in-flight set, `Instance::complete_service`,
  the ordering ledger, the frozen `MetisStatus` space and the fixture's
  cancellation token are the landed abstractions, used as they were. The
  do-not list held in full: no timer, no runtime, no new status variant for
  cancellation, no quarantine path for a slow call (quarantine stays reserved for
  an abandoned poll and an unreleasable teardown), and no ABI or dependency
  change. The one new piece of host state, `Instance::accepted_cancels`, exists
  because only a poll observes the terminal status, which is the call's fact
  rather than the instance's.
- The seam is the one the spec named: `crates/plugin-runtime/tests/cancellation.rs`
  against the public instance and call API, with the ledger and the plugin's own
  callback logs as the assertions. `docs/testing.md` gained the command bullet
  for that file, as the file's protocol rule requires.
- Evidence worth reading with the report: two of the six cases are regression or
  shape assertions rather than RED-first cycles. Criterion 1's behaviour was
  already landed by `waker-lifecycle-fix`, so it passed on arrival, and criterion
  6 is a property of the code's shape (the test proves `cancel_all` coerces to
  `fn(&Instance, Instant) -> CancelReport`, which shows the form is synchronous
  and deadline-fed), so it cannot have a failing first run. The worker recorded
  both rather than claiming a red run it did not have.

### Out-of-scope changes

- `Host::cancel_services` and the internal `mark_cancel`, `accepted_cancels` and
  `acknowledged` are additions outside the ticket's named exports. The first is
  the drop rule's necessary host-side entry and is crate-private; the rest are
  private state. None widens the daemon-facing surface.
- `docs/testing.md` (+19) is in scope by the test protocol's own rule.
- No change to `Cargo.toml`, `Cargo.lock`, `docs/plugin-contract.md` or any ABI
  artifact.
- One evidence gap rather than a change: the TDD worker did not run `buf lint`,
  `buf breaking` or `bash scripts/regenerate-contracts.sh --check`, on the stated
  premise that they need the BSR credential. They have run and passed in this
  effort without a token, because `buf` resolves anonymously with a rate limit,
  so the ticket's evidence set is three commands narrower than CI's. Finalize's
  gate runs them, and this ticket changes no contract.

### Ticket doc update needed?

Yes, and the landing step should append to `## Implementation notes`: the
`Instant` deadline and the two report types with their ascending order; the two
new host-side gates on `cancel` and `complete_service` and the three dependents
they affect; the crate-private `cancel_services`; the two ledger line strings;
the reading of "every buffer the host still holds" as the service-request copies;
and the three CI commands the worker did not run. The ticket's checkboxes and
`workflow_state` stay untouched: landing and finalize own them.

### User attention needed?

No approval is needed, and no scope was dropped. The API surface differs in
three ways a dependent ticket must follow deliberately: the `Instant` deadline
that `teardown-and-release` should share, the `NotFound` that
`Instance::complete_service` and `Instance::cancel` now answer for an id the
instance does not hold, and the two new report types at the crate root. One
documentation placement question is left to the reconciler or to
`startup-policy`: the host-side rule that an id the host does not hold is
answered `NotFound` without calling the plugin has no sentence in
`docs/plugin-contract.md`.

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "Deviation report written to docs/tasks/metis-plugins/tickets/call-cancellation/deviation-reports/call-cancellation.md comparing the arch spec's call-cancellation section, the ticket's six criteria and the implementation on ticket/call-cancellation (tip 385e538), with seven API-surface items, the abstraction verdict, the out-of-scope set, the ticket-note ask and the user-attention answer, plus five residual risks."
    }
  ],
  "changedFiles": [
    "docs/tasks/metis-plugins/tickets/call-cancellation/deviation-reports/call-cancellation.md"
  ],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "git diff --stat task/call-cancellation..ticket/call-cancellation",
      "result": "passed",
      "summary": "three files: lib.rs +163/-4, tests/cancellation.rs new (422), docs/testing.md +19"
    },
    {
      "command": "git diff task/call-cancellation..ticket/call-cancellation -- crates/plugin-runtime/src/lib.rs",
      "result": "passed",
      "summary": "read every hunk: CancelReport, RefusedCancel, accepted_cancels, cancel_services, mark_cancel, cancel_all, both gates, acknowledged, the Cancelled poll arm, the drop order"
    },
    {
      "command": "git diff task/call-cancellation..ticket/call-cancellation -- docs/testing.md",
      "result": "passed",
      "summary": "the new command bullet matches the six test cases"
    },
    {
      "command": "grep -n 'pub struct PendingService' -A 20 crates/plugin-runtime/src/lib.rs",
      "result": "passed",
      "summary": "PendingService holds a String and a Vec<u8> copy plus the id, so releasing one drops a copy; its Clone derive predates this ticket"
    },
    {
      "command": "grep -n -i cancel docs/plugin-contract.md",
      "result": "passed",
      "summary": "the contract states the plugin's side of cancel and does not state the host-side in-flight gate, supporting item 7"
    },
    {
      "command": "sed -n '365,422p' crates/plugin-runtime/tests/cancellation.rs",
      "result": "passed",
      "summary": "criterion 6's case is a compile-time shape proof plus runtime evidence that nothing was polled again and the in-flight set still holds the call"
    }
  ],
  "validationOutput": [
    "git diff --stat task/call-cancellation..ticket/call-cancellation: 3 files changed, 600 insertions(+), 4 deletions(-)",
    "8 wip commits, tip 385e538, landing branch task/call-cancellation at c3e361e",
    "new public items at the crate root: CancelReport { deadline, cancelled, refused }, RefusedCancel { request_id, status }, Instance::cancel_all; Instance::cancel and Instance::complete_service each return CallError::Status(MetisStatus::NotFound) before entering the plugin"
  ],
  "residualRisks": [
    "The Instant deadline is not yet shared with teardown-and-release, whose Serving::release(deadline) must take the same type or a coherence pass is needed.",
    "cancel_all cancels a snapshot of the in-flight set, so a call started after the pass is not cancelled and the report is not exhaustive.",
    "An id reused across instances is now refused by the instance that does not hold it, which changes what a caller sees when it cancels an id it does not own.",
    "The two new ledger line strings are matched by this ticket's tests and by teardown-and-release's report step, so a wording change is a cross-ticket change.",
    "The host-side NotFound rule has no sentence in docs/plugin-contract.md, the artifact a second implementation reads."
  ],
  "noStagedFiles": true,
  "diffSummary": "Implementation under review: Instance::cancel and Instance::complete_service gated on the in-flight set, Instance::cancel_all(deadline: Instant) returning CancelReport and RefusedCancel, a crate-private Host::cancel_services used by PluginCall::drop, two new ledger lines, and a six-case cancellation test file plus its testing.md bullet.",
  "reviewFindings": [
    "no blockers",
    "note: criterion 1 and criterion 6 had no failing first run; the worker recorded this rather than claiming a red cycle",
    "note: three CI commands (buf lint, buf breaking, regenerate-contracts --check) were skipped by the worker on a credential premise that does not hold in this repo; they pass anonymously here"
  ]
}
```
