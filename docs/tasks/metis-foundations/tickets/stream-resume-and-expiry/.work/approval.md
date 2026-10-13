# Approval: `stream-resume-and-expiry` architecture spec

- Approved: 2026-10-14, in chat, by the operator via the batched decision
  questionnaire that followed the draft.
- Approved artifact: `docs/tasks/metis-foundations/arch-spec.md`, section
  `## Pending ticket: stream-resume-and-expiry`.
- Ticket: `docs/tasks/metis-foundations/tickets/stream-resume-and-expiry/ticket.md`.

## Decisions taken at approval

1. **An expired cursor is refused before the stream opens, through a response
   header.** `Ring::stream` becomes fallible at establishment
   (`Result<_, CursorError>`), the daemon's handler maps the error to
   `ErrorCode::FailedPrecondition` plus `metis-cursor-error: expired` in the shape
   `refuse_identity` and `refuse_pair` already have, and the client's landed
   `typed_refusal` chain reports `ClientError::CursorExpired` from the call that
   opened the stream. The stream item type stays `Result` for the slow-consumer
   policy `cancellation-and-slow-consumers` owns, but the expiry itself is never
   an in-stream event.
2. **The resync is explicit and cursorless.** `stream_events(channels, 0)` is the
   documented resync: a fresh stream that begins with the snapshot. No hidden
   retry, no backoff, and no named `resync_events` helper: the caller that asked to
   resume decides after reading `ClientError::CursorExpired`.
3. **A resumed stream carries the replay and no snapshot.** A servable non-zero
   cursor opens with `since(cursor, channels)` typed `EVENT`, then the live
   subscription. The map's "begins with a snapshot message" is read as the
   cursorless stream's rule, and the three proto comments that describe the
   placeholder are corrected to say so.
4. **A restart's expiry is spelled `expired`.** One wire value and one
   `CursorError::Expired { cursor, earliest_resumable, latest_id }` cover both the
   moved window and the restarted daemon, because the client's answer is the same
   in both cases. The daemon's message text names which cause it was.
5. **The walkthrough keeps its step.** `crates/client/examples/events.rs` gains one
   resume step, as the draft presented, because a runnable half per ticket is this
   effort's convention and the resume path is this ticket's headline. The draft
   marked it conditional on the operator keeping it; the approval did not strike it,
   so it is written into the section as required rather than optional.

## State verified before approval

- `main` is at `fc52b36` (`ticket: finalize event-stream`), pushed, working tree
  clean, and `docs/tasks/state.yaml` is `map: metis-foundations`, `task: null`, so
  the effort's frontier is exactly the ticket this section covers plus
  `cancellation-and-slow-consumers`, `reverse-proxy-mode` and
  `windows-named-pipe`.
- The landed placeholder is real and named as one: `floor = cursor.max(head
  identifiers)` in `Ring::stream`, with the snapshot always first, plus the proto
  field comment that says `stream-resume-and-expiry` replaces the rule.
- A restart already expires every cursor with no new machinery: a fresh `Ring`
  answers `earliest_resumable() == 0` and `latest_id() == 0`, so every non-zero
  cursor is beyond the newest identifier the ring assigned.
- The refusal plumbing the ticket reuses exists on both ends:
  `identity::refusal_headers` and `auth::pair_refusal_headers` on the daemon's side,
  `ConnectError::with_headers`, and the `refusal` / `typed_refusal` chain plus the
  response-header read on the stream path in `crates/client/src/lib.rs`. buffa emits
  proto comments into the generated Rust, so the corrected comments move
  `crates/api-contracts/generated/buffa/metis.v1.rs` and
  `crates/api-contracts/generated/connect/metis.v1.rs` and
  `scripts/regenerate-contracts.sh` belongs to this ticket.
- The daemon stream test already carries the `Fixture`, `serve` and `serve_over_tcp`
  helpers the reconnect and restart tests are built from, and its `Fixture::service`
  builds a fresh `Service` over the shared store, which is what "restart the
  service" means in criterion 5. `serve_uds_service` leaves its socket file behind,
  so the restart test serves the second service on a second socket path.

## Not settled here, by design

The slow-consumer drop policy and the `Cancel` RPC (`cancellation-and-slow-consumers`),
per-channel cursors, cursor persistence and a configurable ring capacity, an
automatic client-side resync, and the map's open question of how a client tells a
deliberate stream drop from a transport failure. Each is assigned to a later ticket
or recorded as a residual in the section.
