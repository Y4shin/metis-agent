---
type: deviation report
title: Deviation report for stream-resume-and-expiry
status: stable
---

## Deviation report: stream-resume-and-expiry

Implemented on `ticket/stream-resume-and-expiry` (HEAD `8c5b41d`, seven `wip:`
commits off `main` at `11b6a45`). Compared against the approved
`## Pending ticket: stream-resume-and-expiry` section of
`docs/tasks/metis-foundations/arch-spec.md` (approval record:
`.work/approval.md`, committed as `11b6a45`) and the ticket doc's five acceptance
criteria. The chain's first run timed out at its 900 second budget after the tdd
and verify steps and before this report and the landing, so the retry's tdd step
re-verified the unchanged tree and added no commit; `8c5b41d` is the commit both
the verify step and the resumed tdd step gated.

I re-ran the ticket's own tests on that commit through `devenv shell --`:
`cargo test -p api-contracts --lib` (34 passed, 0 failed) and
`cargo test -p daemon --test stream -p client --test stream` (7 passed and 3
passed, 0 failed). The full gate set on the same commit is the slice-verifier's
PASS: workspace 173 passed, 0 failed, 7 pre-existing ignored; `cargo fmt --check`
and `cargo clippy --all-targets -- -D warnings` clean; `buf lint` and `buf
breaking` against `main` green; `regenerate-contracts.sh --check` and
`regenerate-abi-header.sh --check` clean; the ABI harness at `[result] PASS (73
checks)`; and the `events` example printing `[snapshot]`, `[event]`, `[resume]
cursor=2 id=3` and `[result] PASS`.

### API surface changes

**No planned surface differs from the actual one.** Every export the section named
landed as specified. The entries below are the actuals I confirmed, plus the small
additive and test-only details the section did not name.

- **Planned:** `Ring::stream(&self, channels: &[&str], cursor: u64, heartbeat:
  Duration) -> Result<impl Stream<Item = Result<v1::Event, ConnectError>> + Send
  + 'static, CursorError>`, refused under the same lock that reads the window and
  builds the opening messages, and returned before the stream opens.
  **Actual:** exactly that signature in `crates/api-contracts/src/events.rs`, with
  the expiry predicate `cursor != 0 && (cursor > latest || cursor.saturating_add(1)
  < earliest)` inside the one lock; the cursorless floor stays `max(head ids)` and
  the resumed floor is `max(cursor, last replayed id)`.
  **Impact:** `cancellation-and-slow-consumers` inherits a fallible `Ring::stream`
  and threads the same `?` at its call sites, and it keeps the item type's `Result`
  for its drop policy, which nothing this ticket lands ever fails.

- **Planned:** `CursorError::Expired { cursor, earliest_resumable, latest_id }`
  with a `Display` naming the two causes; `CURSOR_ERROR_HEADER =
  "metis-cursor-error"`; the non-exhaustive `CursorRefusal { Expired }` with
  `as_str`, `parse` and `Display`, an unknown value parsing to `None`; and
  `cursor_refusal_headers(&CursorError) -> HeaderMap`.
  **Actual:** all five, in the shapes `identity::refusal_headers` and
  `auth::pair_refusal_headers` set. `CursorError` is not `#[non_exhaustive]` (the
  section marked only `CursorRefusal` so), and the `Display` cause split is
  `cursor > latest_id` for the restarted identifier space and everything else for
  the moved window. One additive detail the section did not name:
  `impl std::error::Error for CursorError`, consistent with `IdentityError` and
  `AuthError`.
  **Impact:** none on dependents. A caller can box the error, and the wire
  vocabulary stays one value, so a later second refusal adds a header value rather
  than a breaking change.

- **Planned:** `crates/daemon/src/service.rs` maps the refusal through a private
  `refuse_cursor` beside `refuse_identity` and `refuse_pair`, as
  `ErrorCode::FailedPrecondition` plus `events::cursor_refusal_headers(&error)`;
  `crates/client/src/skeleton.rs` maps it identically in its private `events`
  helper, whose signature becomes `ServiceResult<ServiceStream<Event>>`.
  **Actual:** both, with a private `refuse_cursor` of the same name and the same
  status code in each crate. The daemon handler binds the stream with `?` and then
  wraps it in `Ok(Response::stream(stream))` rather than the section's one-liner,
  which is the same value and the same mapping written so type inference does not
  depend on it. The declaration is still checked before the cursor in both
  handlers, `RecordingDaemon` included.
  **Impact:** an expired cursor is a refusal at stream establishment on every
  front-end, so `windows-named-pipe` and `reverse-proxy-mode` inherit it without a
  line of their own.

- **Planned:** `ClientError::CursorExpired` read from `CURSOR_ERROR_HEADER`
  through the landed `typed_refusal` chain, with the cursorless
  `stream_events(channels, 0)` as the documented resync and no hidden retry.
  **Actual:** `ClientError::CursorExpired` is fieldless, and the new
  `cursor_refusal` arm is the fourth in `typed_refusal`, after version, credential
  and pairing. Neither `stream_events` nor `EventStream::message` retries.
  **Impact:** dependent tickets get a wrapper that never retries, and criterion 2
  ("the client turns it into a resync") is satisfied in the explicit form the
  operator chose at approval rather than as an automatic second call. The map's
  fog about distinguishing a deliberate slow-consumer drop from a transport
  failure stays open, because `CursorExpired` is an establishment refusal and not
  a mid-stream event.

- **Planned:** the proto change is prose only, with no field moved or retyped, and
  `scripts/regenerate-contracts.sh` belongs to the ticket because buffa emits the
  comments into the generated Rust.
  **Actual:** four comments corrected (`rpc StreamEvents`,
  `StreamEventsRequest.last_event_id`, the `Event` message and
  `EventKind.EVENT_KIND_SNAPSHOT`), no descriptor change, and both generated trees
  (`crates/api-contracts/generated/buffa/metis.v1.rs` and
  `crates/api-contracts/generated/connect/metis.v1.rs`) moved with the prose.
  `buf breaking` against `main` and both drift checks are green.
  **Impact:** `cancellation-and-slow-consumers` appends `Cancel` to the same file
  and regenerates the same trees, and the corrected comments carry the rule it
  must keep.

- **Planned:** `Event`, `publish`, `snapshot`, `since`, `subscribe`,
  `earliest_resumable` and `latest_id` keep their signatures and behaviour.
  **Actual:** kept, with the bodies of `latest_id`, `earliest_resumable`,
  `snapshot` and `since` moved onto private `State` methods of the same names and
  behaviour. `StreamState.pending` changed from `VecDeque<Event>` to
  `VecDeque<(Event, EventKind)>`, because a resumed stream's opening messages are
  ordinary events while a cursorless stream's are the snapshot.
  **Impact:** none. Both are private, and the unit-tested behaviour of the public
  methods is unchanged.

### Abstraction usage

Used/was specified: yes.

- `Ring::since`, `Ring::earliest_resumable`, `Ring::latest_id` and the landed
  suppression floor: the replay is `since`, read under the same lock as the
  decision, with the subscription still taken before the window, so the interrupted
  window neither loses nor duplicates an event.
- `Ring::stream` as the one assembly of a stream's shape: the daemon's service and
  both skeleton services still call it, so resume exists once and no front-end can
  drift. `serve_stream` remains the only connection path.
- `identity::from_headers` with the shared typed refusal, called before the cursor
  in the daemon's handler and in `RecordingDaemon`, so a stream is never refused
  for a cursor before its declaration is checked.
- The landed refusal plumbing on both ends: `ErrorCode` plus
  `ConnectError::with_headers` on the daemon's side, and the `refusal` /
  `typed_refusal` chain plus the response-header read on the stream path on the
  client's side. No new wire mechanism was introduced.
- The landed test fixtures, with one test-only shape change: the daemon stream
  test's `Fixture`, `serve` and `serve_over_tcp`, the client stream test's own
  private temporary socket base, and the ACL and paired-token fixtures. `serve` in
  `crates/daemon/tests/stream.rs` now returns its listener `JoinHandle` so the
  restart test can drop the served service; the three earlier callers bind it to
  `_served`, so their behaviour is unchanged.
- Additivity holds: the ABI layout and plugin crates are untouched, the harness
  stays at 73 checks, and `Echo`, `Pair` and the cursorless stream keep their
  signatures and behaviour.

### Out-of-scope changes

- `crates/api-contracts/generated/buffa/metis.v1.rs` and
  `crates/api-contracts/generated/connect/metis.v1.rs` changed by generation, not
  by hand. The section named this as the consequence of correcting the proto
  comments.
- `crates/daemon/tests/stream.rs`: the `serve` helper's return type, and a new
  `expire_cursor_one` helper that publishes until the window begins past identifier
  2, bounded at 10 000 publishes with an assertion, rather than restating the
  daemon's private `EVENT_CAPACITY`. The client stream test carries the same rule
  as an inline loop in that file's own style, so the rule now exists in two test
  binaries and neither reads the private capacity.
- `crates/client/examples/events.rs`: the `[resume]` step prints `cursor`, `id`,
  `kind` and `payload`, where the section asked for a `[resume]` line and the
  usual `[result] PASS`.
- `docs/testing.md`: the `Event stream` section gained the resume and expiry
  properties under its existing commands and its example command. No new command
  was added, because the ticket adds tests to files the section already lists.
- Effort-level documentation residue, not fixed here:
  `docs/tasks/metis-foundations/map.md` still says "the snapshot is the stream's
  first message". The approved section read that sentence as the cursorless
  stream's rule and corrected the proto comments to say so, so the map itself was
  not edited. The sentence now contradicts a resumed stream's shape unless it is
  read that way, and the next map edit or the spec-reconciler should add the
  cursorless qualifier.
- Nothing was removed from the contract, the ABI, the plugin crates, the
  manifests, or the existing check sets. No dependency was added: `http` was
  already a dependency of `api-contracts` and of `client`.

### Ticket doc update needed?

Yes. The doc ends at `## Blocked by`, so the land-worker creates the
`## Implementation notes` heading and records:

- the landing paragraph, as the earlier notes do: the landing branch, the merge
  commit, the working branch `ticket/stream-resume-and-expiry` at `8c5b41d` with
  its seven `wip:` commits, and this report's path;
- the chain history, because it explains the commit count: the first run timed out
  after tdd and verify with the verifier's PASS on record, and the retry at plus 50
  percent only re-verified the unchanged tree, adding no commit;
- the cursor rule and where it lives: `CursorError::Expired { cursor,
  earliest_resumable, latest_id }`, `CURSOR_ERROR_HEADER`
  (`metis-cursor-error`), the non-exhaustive `CursorRefusal { Expired }`,
  `cursor_refusal_headers`, and the fallible `Ring::stream`;
- the wire shape of the refusal: `ErrorCode::FailedPrecondition` plus the header,
  raised before the stream opens, with the cursorless `stream_events(channels, 0)`
  as the resync and no automatic retry;
- the proto prose correction and the two generated trees that moved with it, with
  no field change;
- the measured counts: workspace 173 passed, 0 failed, 7 pre-existing ignored;
  `api-contracts --lib` 34; daemon stream 7; client stream 3; the ABI harness
  unchanged at 73 checks; and the example now printing `[resume]`;
- the acceptance criteria ticked, each naming its test;
- the residuals below.

### User attention needed?

No. The scope did not change and no API surface differs from the approved section,
so nothing here needs a decision. Three settled facts are worth knowing:

- The exact status code (`FailedPrecondition`) is set by both `refuse_cursor`
  implementations, but no test pins it end to end: the integration tests assert the
  header and the typed client error, and the one assertion that names the code
  constructs the error inside a unit test. A later change of the code would keep
  every test green while changing what a non-Metis client sees.
- `docs/tasks/metis-foundations/map.md` still carries the unqualified "the
  snapshot is the stream's first message".
- The reconnect and restart tests keep the landed timing dependence, a 25 ms
  heartbeat against 5 second bounds. I ran both binaries once here with no flake,
  and the verifier ran them three times with no flake.

Residual risks, as the implementation and verification reports state them, plus
the ones I found here:

- Expiry is daemon-wide, not per-channel: a chatty channel can expire a quiet
  channel's cursor, and the answer is a resync, as the section records.
- The replay reaches only the in-memory window, 1024 in `daemon::service` and 64
  in `client::skeleton`, still not configurable, and a restart expires every cursor
  with no persistence.
- A caller that changes its channel selection while resuming is not detected. The
  section documents that such a caller resyncs, and the rule is untested: the tests
  only ever resume on a selection whose state the caller already holds.
- `expire_cursor_one` reaches the expiry by publishing, bounded by an assertion,
  rather than by naming the daemon's capacity. It keeps working if the capacity
  changes and fails loudly if the window cannot move.
- `refs/heads/task/stream-resume-and-expiry` does not exist and this report's path
  was absent until now. Both belong to the chain's later steps, and the working
  branch is intact at `8c5b41d` for them. `docs/tasks/state.yaml` is the only dirty
  file, and it is the harness's task pointer, not the ticket's.
