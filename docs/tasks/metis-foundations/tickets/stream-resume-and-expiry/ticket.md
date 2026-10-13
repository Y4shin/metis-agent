---
type: ticket
subtype: feature
title: Stream resume and expired cursors
status: stable
workflow_state: ready
blocked_by: [event-stream]
size: s
---

## What to build

Reconnection without loss: the cursor is honoured, missed events replay from the
ring, an expired cursor is reported distinctly, and the client answers it with a
resync instead of a retry loop or a silent gap.

## Acceptance criteria

- [ ] A stream opened with a cursor delivers only events after that identifier.
- [ ] An identifier older than the ring produces the distinct expired-cursor
      error, and the client turns it into a resync, meaning a fresh stream that
      begins with a snapshot.
- [ ] A test disconnects mid-stream and reconnects with the last seen identifier,
      asserting no gap and no duplicate.
- [ ] A test requests an expired cursor and asserts the resync path.
- [ ] A service restart expires every cursor, asserted by restarting the service
      in a test.

## Blocked by

- `event-stream`, because there is nothing to resume until a stream and its ring
  exist.

## Implementation notes

Landed on `task/stream-resume-and-expiry` as the merge commit `b43d7dd`
("ticket(stream-resume-and-expiry): Stream resume and expired cursors"), parents
`11b6a45` and `8c5b41d`. The landing branch did not exist when this ticket was
landed, so it was created at `main`'s tip (`11b6a45`, "docs(stream-resume-and-expiry):
approve the arch spec"), the convention this spec records and the one
`event-stream` used. `8c5b41d` is the working branch
`ticket/stream-resume-and-expiry`, seven `wip:` commits off `main` at `11b6a45`.
The merge was clean with no conflicts, and the landing tree is byte-identical to
the verified `8c5b41d` (`git diff 8c5b41d HEAD` is empty), so every source file
landed exactly as verified. The branch `ticket/stream-resume-and-expiry` was
deleted after the merge, and the deviation report is committed alongside this
note at
`docs/tasks/metis-foundations/tickets/stream-resume-and-expiry/deviation-reports/stream-resume-and-expiry.md`.

Two points from the chain's history. The first attempt timed out at its 900
second budget after the tdd and verify steps and before the deviation report and
the landing; the retry's tdd step re-verified the unchanged tree at `8c5b41d` and
added no commit, and the verify step's PASS stands on that same commit. That is
why the branch carries seven `wip:` commits and no eighth: the implementation was
already complete when the retry began.

Nothing was re-run at landing, because the landing tree equals the verified tip
byte for byte. The gates recorded against `8c5b41d` through `devenv shell --`
therefore hold for the landed tree: `cargo fmt --check` clean;
`cargo clippy --all-targets -- -D warnings` clean (the one C `_FORTIFY_SOURCE`
line is the pre-existing `plugin-contract` build script); the workspace
`cargo test` at 173 passed, 0 failed, 7 pre-existing ignored;
`cargo test -p api-contracts --lib` at 34 passed; `cargo test -p daemon --test
stream` at 7 passed; `cargo test -p client --test stream` at 3 passed; `buf lint`
and `buf breaking --against '.git#branch=main' --against-config buf.yaml` green
(the proto change is prose only, no field moved or retyped); both drift checks
(`scripts/regenerate-contracts.sh --check`, `scripts/regenerate-abi-header.sh
--check`) clean; the ABI harness at `[result] PASS (73 checks)`; and the `events`
example printing `[snapshot]`, `[event]`, `[resume] cursor=2 id=3` and `[result]
PASS`.

What landed, 12 files and +988/-159 against `main`: the fallible `Ring::stream`
with the real cursor rule in `crates/api-contracts/src/events.rs` (a servable
cursor replays only what follows it, typed `EVENT` and with no snapshot, and the
floor is `max(cursor, last replayed id)` under the same lock that reads the
window); `CursorError::Expired { cursor, earliest_resumable, latest_id }`,
`CURSOR_ERROR_HEADER` (`metis-cursor-error`), the non-exhaustive `CursorRefusal`
and `cursor_refusal_headers`; one `refuse_cursor` beside `refuse_identity` and
`refuse_pair` in `crates/daemon/src/service.rs` and
`crates/client/src/skeleton.rs`, raised before the stream opens;
`ClientError::CursorExpired` in `crates/client/src/lib.rs`, read from the header
rather than message text, with the documented cursorless call as the only resync;
the two generated trees moving with the prose-only contract comments; the example
resume step; `docs/testing.md`; and the cursor, expiry, reconnect and restart
tests in `crates/api-contracts`, `crates/daemon/tests/stream.rs` and
`crates/client/tests/stream.rs`. Each of the five acceptance criteria above
traces to a behaviour-asserting test, as the deviation report records.

`docs/tasks/state.yaml` is the orchestration task pointer. It is left
uncommitted and outside this landing commit, as in the earlier landings, so the
commit holds only the ticket doc and the deviation report. The ticket's
`workflow_state` and the archive stay with `finalize-task`.

Residual risks carried into the effort unchanged: the reconnect, restart and
expiry socket tests are timing dependent (5 second receive bounds against a 25 ms
heartbeat, and a 60 second heartbeat where silence is asserted), stable across
the repeats run by the tdd, verify and deviation steps but wall-clock sensitive
under load; expiry is daemon-wide rather than per-channel, so a chatty channel
can expire a quiet channel's cursor and the only answer is a resync; resume
reaches only the in-memory ring, 1024 in `daemon::service` and 64 in
`client::skeleton`, the capacity is not configurable, and a restart expires every
cursor with no persistence; a caller that changes its channel selection while
resuming is not detected and the rule is untested; no test pins the
`FailedPrecondition` status end to end, so a later change of the code would keep
every test green while changing what a client that does not read the header sees;
the `expire_cursor_one` fixtures reach expiry by publishing under a 10 000
iteration bound rather than naming the private capacity, so a much larger
capacity would fail loudly instead of adapting; and
`docs/tasks/metis-foundations/map.md` still carries the unqualified "the snapshot
is the stream's first message".
