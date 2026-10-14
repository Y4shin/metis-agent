---
type: deviation report
title: Deviation report for event-stream
status: stable
---

## Deviation report: event-stream

Implemented on `ticket/event-stream` (HEAD `64bd7d5`, seven `wip:` commits off
`main` at `4235803`). Compared against the `## Pending ticket: event-stream`
section of `docs/tasks/metis-foundations/arch-spec.md` and the ticket doc's
acceptance criteria. I re-ran the gates on that commit: `cargo fmt --check`
(OK), `cargo clippy --all-targets -- -D warnings` (clean),
`cargo test --workspace` (163 passed, 0 failed),
`bash scripts/regenerate-abi-header.sh --check` (no drift),
`bash scripts/regenerate-contracts.sh --check` (no drift), `buf lint` (OK),
`buf breaking --against '.git#branch=main' --against-config buf.yaml` (OK),
`cargo run -p plugin-fixture --bin abi-harness` (`[result] PASS (73 checks)`),
and `cargo run -p client --example events` (`[result] PASS`).

### API surface changes

- **Planned:** the spec left the `Event.payload` wire type to the implementer,
  with JSON text in a `string` as the accepted fallback, and required the choice
  to be recorded here.
  **Actual:** `string payload = 4`, documented as JSON text, chosen because a
  `string` pulls no well-known type or schema dependency into generation and both
  the ring and the client treat the payload as opaque text.
  **Impact:** `stream-resume-and-expiry`, `cancellation-and-slow-consumers` and
  the daemon's real event sources must encode payloads as JSON text. A later
  native encoding is a FILE-mode breaking change against a committed field 4.
  This is the delegated choice, recorded as the spec required.

- **Planned:** `enum EventKind { EVENT_KIND_UNSPECIFIED = 0; SNAPSHOT = 1;
  HEARTBEAT = 2; EVENT = 3; }`.
  **Actual:** the proto declares the idiomatic prefixed constants
  (`EVENT_KIND_SNAPSHOT`, `EVENT_KIND_HEARTBEAT`, `EVENT_KIND_EVENT`), and buffa
  additionally emits the shorter Rust aliases `EventKind::Snapshot` and
  `EventKind::Heartbeat` under `#[allow(non_upper_case_globals)]`.
  **Impact:** the spec's spelling `EventKind::SNAPSHOT` does not exist; callers
  write `EventKind::Snapshot` or `EventKind::EVENT_KIND_SNAPSHOT`.
  `cancellation-and-slow-consumers` adds kinds and will follow whichever spelling
  the reconciled section names, so the reconciler should correct it.

- **Planned:** `Ring::new(capacity)`, `publish(channel, kind, payload) -> u64`,
  `snapshot(&[&str]) -> Vec<Event>`, `since(id, &[&str]) -> Vec<Event>`,
  `earliest_resumable()`, and "a live subscription handle for the serving side".
  **Actual:** all of those, plus `latest_id()`, `subscribe() -> Subscription`,
  `Ring::stream(channels, cursor, heartbeat) -> impl Stream<Item =
  Result<v1::Event, ConnectError>> + Send + 'static`,
  `Event::{to_message, from_message}`, a clonable `Ring`, `Ring::new(0)` raised
  to capacity 1, and `0` from `latest_id()`/`earliest_resumable()` on an empty
  ring.
  **Impact:** `stream(...)` is the one assembly the daemon and the in-process
  test service share, so the snapshot-then-live-then-heartbeat order cannot drift
  between them. `stream-resume-and-expiry` consumes `since`, `earliest_resumable`
  and the cursor floor; the floor rule (`cursor.max(head ids)`) is the
  placeholder it replaces.

- **Planned:** `Service::new(store, preshared, heartbeat: Duration)` with
  `HEARTBEAT_DEFAULT` at 15 seconds, and the `service(store, preshared, policy)`
  factory.
  **Actual:** as planned, plus `Service::ring() -> Ring` so the daemon can
  publish into the served ring, and `Service::mount(self: Arc<Self>, policy) ->
  ConnectRpcService<Router>`, with `service(...)` delegating to it. A private
  `EVENT_CAPACITY = 1024` sets the window in `daemon::service` and again in
  `client::skeleton`.
  **Impact:** `Service::new` is a signature change, which is why
  `crates/daemon/tests/auth.rs` moves in the same commit. The ring capacity is
  not configurable and is stated twice; recorded as a residual rather than fixed
  here.

- **Planned:** `Client::stream_events(&self, channels: &[&str], last_event_id:
  u64) -> Result<EventStream, ClientError>`, where `EventStream` yields typed
  events and no transport or generated type is named.
  **Actual:** that signature, with `EventStream::message() ->
  Result<Option<Event>, ClientError>` as the drain method and the yielded type
  being `api_contracts::events::Event`. The refusal path was refactored into
  `refusal(err)` and `typed_refusal(&HeaderMap)`, both `pub(crate)`, because a
  Connect streaming response carries its status in-stream: `stream_events` reads
  the refusal headers on the response the stream was established with, so an
  unauthorized stream fails at the call that opened it instead of at the first
  event.
  **Impact:** consumers of this ticket drain with `message()`. The wrapper's
  struct holds `hyper::body::Incoming` privately, so the transport stays
  invisible, but `hyper` is now a real dependency rather than a dev one.

- **Planned:** the spec named no new dependencies.
  **Actual:** `client` gains `hyper` as a runtime dependency and moves `http`
  from `[dev-dependencies]` to `[dependencies]`; the workspace adds
  `futures-util = { version = "0.3", default-features = false, features = ["std"] }`
  and `tokio`'s `sync` feature; `api-contracts` adds `futures-util`.
  **Impact:** the wrapper's dependency footprint grew beyond the generated client
  and `connectrpc`. An auditor of that boundary should know `hyper` is named
  there privately, and that the streaming trait `connectrpc` speaks is
  `futures_core::Stream`.

- **Planned:** "The example prints the snapshot and one event."
  **Actual:** a new `crates/client/examples/events.rs` rather than an extension of
  `crates/client/examples/echo.rs`, which is untouched. It serves the skeleton
  over a ring, opens the stream through the wrapper, prints `[snapshot]`,
  publishes a second event, prints `[event]`, and exits non-zero on failure.
  **Impact:** `docs/testing.md` documents both, and the two walkthroughs cover
  one path each.

### Abstraction usage

Used/was specified: yes.

- `serve_stream` and the private `serve_connection`, with `connectrpc`'s native
  server-streaming support: no new transport path, and
  `serve_tcp_listener_service` serves the stream over TCP.
- `AuthInterceptor::intercept_streaming`: the stream is refused without a
  credential and served with a paired token. That is the streaming half
  `pairing-and-tokens` recorded as untested, and it carries the stream-over-TCP
  assertion `tcp-listener` criterion 4 deferred, so both debts are paid here.
- `identity::from_headers` in the streaming handler with the shared typed
  refusal, which the downstream-contracts outline required of this ticket.
- The landed test helpers (`TempSocket`, `reserve_loopback_port`, `serve_over_tcp`,
  the paired-token fixture) and the ACL helpers, unchanged.
- Additivity holds: the ABI layout and plugin crates are untouched, the harness
  stays at 73 checks, and `Echo` and `Pair` keep their signatures and behaviour.

Two factual corrections to the tdd report, neither changing a verdict:

- It reports "seven unit tests" in `events.rs`; the file has nine (`#[test]` ×6
  and `#[tokio::test]` ×3).
- It reports the `EventKind` members as `SNAPSHOT`/`HEARTBEAT`/`EVENT`; the
  declared names are the prefixed ones above, with the shorter aliases.

### Out-of-scope changes

- `crates/client/examples/events.rs` is new rather than an edit of `echo.rs`.
- `crates/client/src/skeleton.rs`: `SkeletonDaemon` was a unit struct and is now
  a struct with fields, built through `new()` and `with_ring()`; `router_with`,
  `serve_uds_with` and `HEARTBEAT_DEFAULT` were added. Already-landed
  test-support surface changed shape, with every in-tree caller updated in the
  same commit and the workspace green.
- `crates/daemon/tests/auth.rs` edited, which the spec's `Service::new` signature
  change requires; the edit is that change, not a papering-over.
- `crates/api-contracts/tests/contract.rs` extended beyond the plan's letter: the
  `StreamEvents` procedure path, the byte-level field numbering of
  `StreamEventsRequest`, `StreamType::ServerStream`, and message round trips.
  Consistent with the file's existing style and with the spec's contract seam.
- `contracts/README.md` gained a sentence about the lint exception; the spec
  listed only `docs/testing.md` under documentation.
- Cosmetic: a double blank line in `crates/client/Cargo.toml` between
  `http = "1"` and `[dev-dependencies]`.
- Nothing was removed from the contract, the ABI, the plugin crates, or the
  existing check sets.

### Ticket doc update needed?

Yes. The land-worker must create the `## Implementation notes` heading, since the
doc ends at `## Blocked by`, and record:

- the landing paragraph, as the earlier notes do;
- the delegated payload decision (`string` JSON text) and its lock-in;
- the `EventKind` naming, including the buffa aliases;
- the 163-test workspace suite, the harness baseline unchanged at 73, and the new
  example as the second walkthrough;
- the new dependencies (`futures-util`, `hyper` and `http` at runtime in
  `client`, `tokio/sync`);
- `SkeletonDaemon`'s shape change and the `Service::{ring, mount}` additions;
- the residual list below.

### User attention needed?

Yes, on settled facts rather than on an open decision.

- The contract gained its first enum and its first streaming RPC, and the wire now
  carries a `string payload` whose encoding is a convention rather than a schema.
  Both are the approved shape, and both are expensive to change under FILE-mode
  breaking.
- The wrapper now depends on `hyper` at runtime, which the plan did not name.
- Two finalized tickets' deferred stream obligations are paid here, so nothing is
  silently outstanding from `tcp-listener` or `remote-pairing`.

Residual risks, as the implementation and verification reports state them, plus
the two found here:

- Heartbeats are unconditional rather than idle-detected; the ring is per-process
  and lost on restart; the slow-consumer policy is the library default; the
  payload encoding is opaque text.
- `last_event_id` only floor-suppresses live events. It is a placeholder that
  `stream-resume-and-expiry` replaces, and it is not real resume.
- `EVENT_CAPACITY` and `HEARTBEAT_DEFAULT` each exist twice, in `daemon::service`
  and `client::skeleton`, and can drift. A coherence-pass candidate.
- Timing-dependent tests use a 25 ms heartbeat and 5 s bounds. Three repeats of
  both stream suites showed no flake, and I re-ran the workspace suite here.
- `refs/heads/task/event-stream` is not created and the deviation report path was
  absent until this report; both belong to the chain's later steps, and the
  working branch is intact at `64bd7d5` for them.
