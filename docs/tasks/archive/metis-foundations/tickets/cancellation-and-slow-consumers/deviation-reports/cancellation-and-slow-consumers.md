---
type: deviation report
title: Deviation report for cancellation-and-slow-consumers
status: stable
---

## Deviation report: cancellation-and-slow-consumers

Implemented on `ticket/cancellation-and-slow-consumers` (HEAD `e960712`, eleven
`wip:` commits off `main` at `bb1bd07`), 19 files and +4084/-565. Compared against
the approved `## Pending ticket: cancellation-and-slow-consumers` section of
`docs/tasks/metis-foundations/arch-spec.md` (approval record `.work/approval.md`,
committed as `bb1bd07`) and the ticket doc's five acceptance criteria.

I re-ran the ticket's own tests on that commit through `devenv shell --`:
`cargo test -p api-contracts --lib` (48 passed, 0 failed), `cargo test -p daemon
--test cancel` (7 passed, 0 failed) and `cargo test -p client --test stream` (5
passed, 0 failed), plus the two additivity checks this report speaks to (`cargo run
-p plugin-fixture --bin abi-harness` at `[result] PASS (73 checks)` and `bash
scripts/regenerate-abi-header.sh --check` at "the ABI header matches the crate").
The rest of the gate set is the tdd worker's to declare and the slice-verifier's
to confirm.

### API surface changes

**Every surface the section named landed, and no named surface is missing.** The
entries below are the actuals, including four additions the section did not name
and two places where the actual shape departs from the plan's letter.

- **Planned:** `Subscription::recv` stops hiding a lag and reports what the window
  lost; `Ring::stream` turns the first lag into its terminal refusal.
  **Actual:** both, and the return type is a **new public enum `Live`** in
  `crates/api-contracts/src/events.rs`: `Live::Event(Event)` and
  `Live::Lagged { skipped: u64 }`, so `recv` is now `Option<Live>` where it was
  `Option<Event>`. That replaces a landed public signature rather than adding
  beside it. `Ring::stream` sets a `done` flag with the refusal, so the poll after
  the refusal ends the stream, and `Next::Lagged` joins `Next::Event`,
  `Next::Heartbeat` and `Next::Ended`. The landed unit test
  `a_subscription_delivers_published_events_in_identifier_order` changed shape in
  the same commit (a `next_event` helper), which the section's "stops hiding a lag"
  implies but does not say.
  **Impact:** `api-contracts` is shared, so the daemon effort and every later
  reader of `Subscription::recv` must match the enum. Nothing outside
  `Ring::stream` and that file's tests calls `recv` today, so the practical reach
  is the daemon effort and the two front-end tickets, and the wire is unaffected.

- **Planned:** the section named `Ring::stream` and the guard as the pieces of the
  drop policy; it named no assembly a handler wraps a stream in.
  **Actual:** `pub fn cancellable<S>(inner: S, guard: Option<InFlight>) -> impl
  Stream<Item = Result<v1::Event, ConnectError>> + Send` is new and public in
  `api-contracts::events`. It boxes the inner stream, selects the guard's
  cancellation against it (biased, so a message already produced wins), holds the
  `InFlight` guard for the stream's lifetime, and emits the typed `Cancelled`
  refusal once and then ends. The daemon's handler and both skeleton services wrap
  their streams in it, so a cancellation ends a stream identically in the daemon
  and in process.
  **Impact:** this is the one assembly a dependent front-end or the daemon effort
  must reuse rather than re-implement; it is where "a cancel ends its own stream"
  and "a dropped stream cancels nothing" are actually enforced.

- **Planned:** `CancelOutcome` in the proto bullet, in `Registry::cancel`'s
  signature and in `Client::cancel`'s return type; the plan never says which Rust
  type the client exposes.
  **Actual:** the generated wire enum exists as planned, **and**
  `api_contracts::cancel::CancelOutcome { Cancelled, AlreadyFinished }` is a
  hand-written mirror with `to_proto()` and `from_proto()`, so `Registry::cancel`,
  `Client::cancel` and both handlers pass the Rust value and the conversion happens
  at the wire edge. `from_proto` returns `None` for `UNSPECIFIED` and for a value a
  later version adds, and `Client::cancel` maps that to `ClientError::Call` rather
  than guessing.
  **Impact:** the wrapper's public API names no generated type, the property
  `client` has kept since `event-stream`; dependents name
  `api_contracts::cancel::CancelOutcome` and `api_contracts::request::RequestId`,
  because the client re-exports neither (only `EventStream`), exactly as they
  already name `api_contracts::events::Event`.

- **Planned:** `pub enum RequestIdError { Malformed }` and a `RequestId` carrying
  the parse rule.
  **Actual:** as planned, plus `#[non_exhaustive]` on `RequestIdError` (the shape
  `CredentialRefusal`, `PairRefusal` and `CursorRefusal` already have, and the
  opposite of the single-valued `CursorError`), and `Clone`/`Hash`/`PartialEq`/
  `Eq` on `RequestId` because it is a map key. The rule is the section's exactly,
  implemented as `is_ascii_graphic() && byte != b' '` (the same range as
  `0x21..=0x7E`), and the generated identifier is 16 random bytes as 32 lowercase
  hex characters, a length the section left open.
  **Impact:** none on the wire (the daemon enforces the same rule on the header and
  on the `CancelRequest` body), and the non-exhaustive attribute is the effort's
  standing pattern for a refusal vocabulary.

- **Planned:** `EventStream::message()` maps a terminal error "through the refusal
  chain extended to trailers".
  **Actual:** the extension is in `crate::refusal` and `typed_refusal`, in
  `crates/client/src/lib.rs`, not in `stream.rs`, which keeps calling it. Response
  headers are still read first, then the trailers, then the transport error, and
  the new `stream_refusal` arm is the fifth in `typed_refusal`, behind version,
  credential, pairing and cursor.
  **Impact:** the section's sentence holds behaviourally, with one widening worth
  naming: because the trailer fallback sits in the shared chain, a **unary** failure
  that carries trailer metadata would now map to a typed error as well. No daemon
  path puts a `metis-*` refusal in unary trailers today, so nothing observable
  changed, but the mapping is no longer stream-only.

- **Planned:** `EchoRequestId` copies "the incoming identifier into the response
  metadata".
  **Actual:** it copies the header value **verbatim, a malformed one included**,
  from the request headers into `Response::headers` on the unary and the streaming
  path, and it is the inner layer of the daemon's chain
  (`with_interceptor(AuthInterceptor).with_interceptor(EchoRequestId)`, so a refused
  call never reaches it). Validation stays where the section put it, in the
  handlers, so an `Echo` or `Pair` call declaring an unreadable identifier is still
  served and now echoes it back.
  **Impact:** a caller can always correlate the answer with the call it made, which
  is the map's correlation decision, and nothing new can be refused because of the
  echo.

- **Planned:** the daemon's `Cancel` refuses a malformed identifier with a plain
  `InvalidArgument`, and the skeleton carries "the same registry and the same
  registration" as the in-process service.
  **Actual:** the daemon does exactly that, through a private `refuse_request_id`
  beside `refuse_cursor`; the skeleton **does not**. `SkeletonDaemon::cancel` parses
  the identifier and answers `CancelOutcome::AlreadyFinished` when it cannot
  (`Err(_) => CancelOutcome::AlreadyFinished`), on the stated ground that the
  skeleton is declaration-free test support, and `RecordingDaemon`, which does check
  the declaration, is lenient about the identifier the same way
  (`from_headers(..).ok().flatten()`).
  **Impact:** the section's seam 2 says "a malformed identifier is refused as
  `InvalidArgument`", and that is asserted only on the daemon's path
  (`crates/daemon/tests/cancel.rs::a_malformed_identifier_is_refused_as_invalid_argument`).
  An in-process test through the skeleton sees the lenient answer instead, so the
  two services disagree on one input. No client can reach it, because the wrapper
  only ever sends its own generated identifier, so this is test-support drift rather
  than a wire difference. It is the one place the daemon and the skeleton do not
  answer identically.

- **Planned:** `Registry::register`'s entry "lives exactly as long as the guard, so
  it is removed when the work ends for any reason".
  **Actual:** that holds, and the uncased duplicate is decided: a second
  registration under the same surface and identifier **replaces** the first entry,
  and the earlier guard's `Drop` removes only its own entry (checked with
  `Arc::ptr_eq`), so a drop can never unregister another registration's work. A
  caller that reuses a generated identifier therefore loses the ability to cancel
  its first piece of work, and if the newer guard drops first the older work runs
  with no entry at all.
  **Impact:** none today, because the wrapper generates a fresh identifier per
  stream and the section calls an identifier collision a caller reusing a name
  deliberately; it is recorded because the daemon effort registers work of its own
  and needs the rule written down rather than inferred.

### Abstraction usage

Used/was specified: yes.

- The landed refusal machinery on both ends, extended rather than duplicated:
  `ConnectError::with_trailers` and `stream_refusal_headers` on the daemon's side,
  the `refusal` / `typed_refusal` chain on the client's. The new vocabulary follows
  the landed spelling exactly (`metis-stream-error`, values `slow-consumer` and
  `cancelled`, non-exhaustive parse, `None` for a value this build does not know).
- The landed `Result` item seam: the drop is the stream's own terminal error item,
  which is what `event-stream`'s section reserved the seam for, and the snapshot,
  cursor, suppression-floor and heartbeat behaviour is unchanged.
- `Ring::stream` as the one assembly of a stream's shape, still called by the
  daemon's handler and by both skeleton services, with `events::cancellable` the one
  gate around it. `serve_stream` remains the only connection path.
- `identity::from_headers` and `ClientIdentity::surface()` as what the registry keys
  on, called first in both new handlers, with the recording preserved for the stream
  in `RecordingDaemon`.
- The interceptor chain `Service::mount` already builds: `with_interceptor` appends,
  so the echo rides beside `AuthInterceptor` rather than replacing it, as the
  section's existing-abstractions bullet required.
- `getrandom`, the workspace dependency the daemon already uses for token minting,
  as the client's source for a generated identifier; it is the ticket's one new
  dependency edge and the section named it.
- The landed test fixtures: the daemon cancel test carries its own `Fixture`,
  `serve`, `raw_client`, `wait_for` and `stall_past_the_bound` rather than
  extracting a shared module, which is the repo's fixture-per-binary convention, and
  the client stream test keeps its private socket base. `next` in the daemon file
  now carries a five second timeout, the bound the landed stream tests already use.
- Additivity holds: the ABI layout and plugin crates are untouched, the harness
  stays at 73 checks, the proto addition is additive with `buf lint` and
  `buf breaking` green, a call that declares no identifier is served as before, and
  `Echo`, `Pair`, the cursor and expiry semantics keep their signatures and
  behaviour.

### Out-of-scope changes

- `crates/api-contracts/tests/contract.rs`: `DummyDaemon` gained the `cancel` arm so
  the generated trait stays implementable from outside the crate, and the file
  gained `cancel_is_declared_as_a_unary_call_with_an_outcome` (procedure path,
  `StreamType::Unary`, the byte-level field numbering of both messages, the three
  `CancelOutcome` numbers) plus the route-set assertion. The section named no
  contract test for `Cancel`; this follows the file's style and the `event-stream`
  precedent, where the same file also went beyond the plan's letter.
- `crates/daemon/tests/auth.rs`: the recording daemon gained a `cancel` arm that
  records the procedure path and delegates, because the trait addition would not
  compile otherwise. Behaviour unchanged.
- `crates/client/src/skeleton.rs`: the serve path changed shape. `serve_uds` and
  `serve_uds_with` now call `transport::serve_uds_service` with a
  `ConnectRpcService` carrying `EchoRequestId`, through the new private `service`
  and `service_with`. `router()` and `router_with()` stay exported and unchanged,
  and the callers that serve them themselves (the identity recording tests, the
  duplex echo test) mount no echo. The ACL is unchanged: `serve_uds` was already a
  delegation over `serve_uds_service`, and both apply `PathAcl`.
- `crates/daemon/tests/cancel.rs` (new, 708 lines): its own fixture rather than a
  shared `tests/support` module, matching the convention the other integration-test
  binaries follow. `stall_past_the_bound` fills the window by publishing under a
  10 000 iteration bound rather than restating the daemon's private
  `EVENT_CAPACITY`; the client stream test carries a second copy of that helper.
- `crates/client/tests/stream.rs`: two new tests and the second
  `stall_past_the_bound` copy. The client's drop test constructs `Ring::new(4)` so
  the bound is reachable from a test, the one place a smaller-than-production window
  is built for this policy.
- `crates/api-contracts/generated/buffa/metis.v1.rs` and
  `crates/api-contracts/generated/connect/metis.v1.rs`: regenerated, with about
  1900 lines of buffa churn because `CancelRequest` and `CancelResponse` are
  declared before `message Event` and generation follows declaration order.
  `scripts/regenerate-contracts.sh --check` is clean, so the tree is reproducible.
- `crates/client/Cargo.toml` and `Cargo.lock`: the `getrandom` edge the section
  named. The pre-existing double blank line in that manifest is still there, now
  just above `[dev-dependencies]`.
- `docs/testing.md`: the `Event stream` section gained the
  `cargo test -p daemon --test cancel` command with its properties, extended the
  `cargo test -p client --test stream` entry with the two terminal refusals, and
  extended the `events` example entry with the `[cancel]` step. No command was
  removed.
- `crates/client/examples/events.rs`: the `[cancel]` step prints the identifier and
  the outcome (`outcome=Cancelled`) and additionally asserts that the cancelled
  stream's next read is `ClientError::StreamCancelled`, where the section asked for
  a `[cancel]` line and the usual `[result] PASS`.
- Doc-only corrections beyond the section's list: the crate docs of `api-contracts`
  and `client` name the new modules and errors; `Ring::stream`'s "nothing fails
  today" paragraph now names the slow-consumer refusal; `client/Cargo.toml`'s
  comment about "response headers" reads "response metadata"; the proto file's
  header comment reads "Three ordinary RPCs, the event stream, and cancellation".
- Nothing was removed from the contract, the ABI, the plugin crates, the manifests
  or the existing check sets, and no existing field, message, sentinel or refusal
  spelling changed.
- Effort-level residue not fixed here: the arch spec's
  `## Downstream interface contracts (outline)` still closes the cancellation bullet
  with "keeps the item type's `Result` for its drop policy, which nothing landed
  here ever fails", which this ticket makes false; and
  `docs/tasks/metis-foundations/map.md`'s fog entry about telling a deliberate
  slow-consumer drop from a transport failure still reads as open, though the
  approved section answers it for the drops the daemon causes (the typed
  `ClientError::StreamDropped` against `ClientError::Call`). Both belong to the
  spec-reconciler, and the approved section names the first as this ticket's own
  interface contract.

### Ticket doc update needed?

Yes: append the land-worker's note plus these facts to `## Implementation notes`.

- The public shapes the section left to the implementer: `events::Live` replacing
  `Subscription::recv`'s return type, `events::cancellable` as the one cancellation
  gate, `api_contracts::cancel::CancelOutcome` as the hand-written mirror of the
  generated enum with `to_proto`/`from_proto`, and `RequestIdError` as
  non-exhaustive.
- The two foreign test files the trait addition forced (`contract.rs`,
  `daemon/tests/auth.rs`) and the contract assertions that went beyond the plan's
  letter.
- The skeleton's leniency: a malformed identifier is answered `AlreadyFinished`
  there and refused as `InvalidArgument` in the daemon, and only the daemon's answer
  is asserted.
- The registry's duplicate-identifier rule (newest wins, `Arc::ptr_eq`-guarded
  removal), and that the guard is registered after the cursor is served, so a
  refused stream leaves nothing in flight.
- The tests' timing shape: the daemon's stalled-reader test relies on a synchronous
  publish burst on the default current-thread test runtime to outrun the server's
  reads, so switching that test to `flavor = "multi_thread"` would make it flaky
  rather than wrong; both `stall_past_the_bound` copies are bounded at 10 000
  publishes rather than naming the private ring capacity; `next` in the daemon
  cancel test carries a five second timeout.
- The residual that the drop bound is the ring capacity and is still not
  configurable, so the client stream test builds a four-slot ring for a reachable
  bound.

### User attention needed?

**Yes, briefly, and none of it changes the daemon's caller-visible behaviour.** The
approved section left four public shapes to the implementer (`Live` replacing
`Subscription::recv`'s landed return type, the new public `events::cancellable`,
the `CancelOutcome` mirror, and `RequestIdError` being non-exhaustive) and one
service pair that disagree on an input (a malformed identifier: `InvalidArgument`
in the daemon, `AlreadyFinished` in the skeleton). The reconciler should record the
four in the landed section and the fifth beside the seams, and the outline sentence
claiming the item type "nothing landed here ever fails" needs correcting. No scope
was added or removed, and every planned export landed.
