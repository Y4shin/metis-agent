---
type: ticket
subtype: feature
title: Call cancellation, its unknown ids and its drop rule
status: stable
workflow_state: ready
blocked_by: [async-initialisation]
size: m
---

## What to build

Cancellation as it was decided. The caller owns the deadline and the drop, so the
runtime holds no timer and no default deadline. Cancelling a live call marks the
plugin's token, and the plugin resolves the call as cancelled. Cancelling a
request id the host does not hold answers the frozen not-found status and never
calls the plugin. An unacknowledged cancel is recorded on the ordering ledger and
the instance keeps serving. The instance-wide form is a host-side loop over the
in-flight set. Dropping a call releases every buffer the host still holds for it
and cancels its outstanding service requests, so no completion is ever delivered
into a released context.

## Acceptance criteria

- [ ] A cancellable fixture call resolves as cancelled and reaches its caller as
      the frozen cancelled status.
- [ ] Cancelling an unknown or already finished id answers not-found without the
      plugin being called, and a second cancel of a live id is accepted.
- [ ] A call dropped before the plugin acknowledged is recorded on the ledger, the
      instance is not quarantined, and the next call to it succeeds.
- [ ] Dropping a call releases its buffers and cancels its service requests, and a
      completion offered afterwards for that id is refused rather than delivered.
- [ ] The instance-wide loop cancels every call on one instance and reports what
      it did.
- [ ] The runtime exposes no way to wait for a cancellation on its own; the caller
      drives both the deadline and the drop.

## Blocked by

- `async-initialisation`

## Implementation notes

Landed on `task/call-cancellation` as the merge commit `ea61c72`
("ticket(call-cancellation): Call cancellation, its unknown ids and its drop
rule"), parents `c3e361e` (the landing branch tip) and `385e538` (the working
branch `ticket/call-cancellation`, eight `wip:` commits, one per acceptance
criterion). The merge was clean with no conflicts and the landing tree is
byte-identical to the verified `385e538` (`git diff 385e538 HEAD` is empty), so
every source file landed exactly as verified. The branch
`ticket/call-cancellation` was deleted after the merge.

The ticket diff over `c3e361e` is `crates/plugin-runtime/src/lib.rs` (+163/-5),
the new `crates/plugin-runtime/tests/cancellation.rs` (422 lines, six cases, one
per acceptance criterion) and `docs/testing.md` (+19, the command bullet for the
new test file). The ABI, the status space and `docs/plugin-contract.md` are
untouched.

The verifier ran against `385e538`: `cargo fmt --check` clean, `cargo clippy
--all-targets -- -D warnings` clean, `cargo test -p plugin-runtime --test
cancellation` 6 passed, and the full `cargo test` 323 passed across 52 test
binaries with no failures. All six acceptance criteria were checked against the
code and hold. `buf lint`, `buf breaking` and `regenerate-contracts.sh --check`
were not run: they need the BSR credential and this ticket changes no contract.

The worker recorded these divergences from the arch spec, and the deviation
report and the verifier both agree they are not blockers:

- `cancel_all` takes `std::time::Instant`, a deadline type the plan left untyped.
  `teardown-and-release` plans `Serving::release(deadline)` and must consume the
  same type or a coherence pass is needed.
- `CancelReport { deadline, cancelled, refused }` and
  `RefusedCancel { request_id, status }` are new public types; the plan named
  only `CancelReport`. `cancelled` is sorted ascending by request id, and a
  dependent must not assume any other ordering or that the report is exhaustive
  (it cancels a snapshot of the in-flight set).
- `Instance::cancel` and `Instance::complete_service` are now gated on the
  instance's own in-flight set, so a caller that cancels or answers an id it does
  not own gets the frozen `NotFound` where it previously reached the plugin.
  `callback-sink` must treat that refusal as expected rather than an error.
- `Host::cancel_services(request_id)` is `pub(crate)`, not public; the call's
  drop is its only caller.
- "Every buffer the host still holds for it" is read as the host-held copy of
  each outstanding service-request payload, since the host retains no
  plugin-allocated buffer for a live call.
- Two ledger lines are added, both only where they say something: `call <id>
  released <n> service request(s) the host still held` (n > 0) and `call <id>
  cancel unacknowledged: the plugin never resolved it as cancelled`.
- Criteria 1 and 6 had no separate failing first run: criterion 1's behaviour was
  already landed by `waker-lifecycle-fix`, and criterion 6 is an API-shape
  property satisfied by the same code as criterion 5.

Residual risks carried forward: the `Instant` deadline choice is not yet shared
with `teardown-and-release`; `cancel_all` cancels a snapshot, so calls started
after the pass are the caller's loop to own; and the ordering ledger stays an
unbounded `Vec<String>`, which the two new lines follow.
