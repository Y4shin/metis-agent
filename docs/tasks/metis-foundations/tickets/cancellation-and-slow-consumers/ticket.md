---
type: ticket
subtype: feature
title: Cancellation and slow-consumer handling
status: stable
workflow_state: ready
blocked_by: [event-stream]
size: m
---

## What to build

Two related guarantees on the stream: a client can stop work with an explicit
idempotent call, and a client that cannot keep up is disconnected rather than
silently missing events, with the client treating that disconnect as a resync.

## Acceptance criteria

- [ ] An explicit cancel call carrying the target request identifier is
      idempotent, returns a distinguishable result when the work already
      finished, and stops in-flight work.
- [ ] Dropping a stream cancels nothing by itself, asserted by a test that
      disconnects the client and observes the work continuing.
- [ ] A consumer that stops reading meets bounded buffering and then a plain
      stream drop, with no events skipped silently.
- [ ] The client treats a dropped stream as resync-required and never presents the
      gap as current state.
- [ ] A test drives a deliberately stalled reader and asserts the drop.

## Blocked by

- `event-stream`, because cancellation and backpressure are properties of the
  stream.

## Implementation notes

Landed on `task/cancellation-and-slow-consumers` as the merge commit `9465e0d`
("ticket(cancellation-and-slow-consumers): Cancellation and slow-consumer
handling"), parents `bb1bd07` and `e960712`. The landing branch did not exist when
this ticket was landed, so it was created at `main`'s tip (`bb1bd07`,
"docs(cancellation-and-slow-consumers): approve the arch spec"), the convention
the shared conventions section of `docs/tasks/metis-foundations/arch-spec.md`
records and the one `stream-resume-and-expiry` used. `e960712` is the working
branch `ticket/cancellation-and-slow-consumers`, eleven `wip:` commits off `main`
at `bb1bd07`. The merge was clean with no conflicts, and the landing tree is
byte-identical to the verified `e960712` (both trees hash to `cb005d58`, and
`git diff e960712 HEAD` is empty), so every source file landed exactly as
verified. The branch `ticket/cancellation-and-slow-consumers` was deleted after
the merge, and the deviation report is committed alongside this note at
`docs/tasks/metis-foundations/tickets/cancellation-and-slow-consumers/deviation-reports/cancellation-and-slow-consumers.md`.

Nothing was re-run at landing, because the landing tree equals the verified tip
byte for byte. The gates recorded against `e960712` through `devenv shell --`
therefore hold for the landed tree: `cargo fmt --check` clean; `cargo clippy
--all-targets -- -D warnings` clean (the one C `_FORTIFY_SOURCE` line is the
pre-existing `plugin-contract` build script, not a Rust lint); the workspace
`cargo test` at 197 passed, 0 failed, 7 ignored, up from `main`'s 173 passed;
`cargo test -p api-contracts --lib` at 48 passed; `cargo test -p daemon --test
cancel` at 7 passed; `cargo test -p daemon --test stream` at 7 passed; `cargo
test -p client --test stream` at 5 passed; `buf lint` and `buf breaking --against
'.git#branch=main' --against-config buf.yaml` green, because the `Cancel` RPC and
its two messages are additive; both drift checks
(`scripts/regenerate-contracts.sh --check`,
`scripts/regenerate-abi-header.sh --check`) clean; the ABI harness at `[result]
PASS (73 checks)`, unchanged from `main`; and the `events` example printing
`[snapshot]`, `[event]`, `[resume]`, `[cancel] request_id=...
outcome=Cancelled` and `[result] PASS`. The verify step also repeated the two new
integration binaries five times each with no failure on any iteration, so the
stalled-reader tests are not obviously flaky.

What landed, 19 files and +4084/-565 against `main`: the `Cancel` unary RPC and
`CancelOutcome` in `contracts/proto/metis/v1/daemon.proto` with both generated
trees regenerated in the same commits;
`api_contracts::request::{RequestId, RequestIdError, EchoRequestId}` with
`REQUEST_ID_HEADER`, the interceptor that echoes a declared identifier back in the
response metadata; `api_contracts::cancel::Registry` with the RAII `InFlight`
guard and the Rust `CancelOutcome` mirror that crosses the wire through
`to_proto` / `from_proto`; `StreamError::{SlowConsumer, Cancelled}`,
`StreamRefusal`, `stream_refusal_headers`, `Live::{Event, Lagged}`,
`Subscription::recv` no longer hiding a lag, `Ring::stream` turning the first lag
into its terminal refusal, and `cancellable`; the daemon's `Cancel` handler and
its registration of stream work, with the skeleton serving the same behaviour
through its socket; `Client::cancel`, `EventStream::request_id`, per-call
identifier generation, and `ClientError::{StreamDropped, StreamCancelled}` read
from the stream's trailing metadata; the new tests at the three seams
(`crates/api-contracts/src/{cancel,events}.rs`,
`crates/daemon/tests/cancel.rs`, `crates/client/tests/stream.rs`); the example's
`[cancel]` step and the `docs/testing.md` commands. Each of the five acceptance
criteria above traces to a behaviour-asserting test, as the deviation report
records, and three mutation checks were shown to fail the tests that should catch
them before being reverted.

Residual risks carried into the effort. Neither stalled-reader test pins a
specific skipped count; both parse any positive number, so a regression reporting
the wrong magnitude would stay green while the direction (a lag is surfaced) is
still checked. Both stalled-reader tests also rely on the default current-thread
test runtime so a synchronous publish burst outruns the server's reads, which
makes a switch to `flavor = "multi_thread"` flaky rather than wrong. A malformed
identifier answers `InvalidArgument` in the daemon but is lenient in the
skeleton, which registers no cancellable work and answers `AlreadyFinished`; the
divergence is deliberate and the acceptance criteria do not pin the skeleton.
`scripts/regenerate-contracts.sh` and `scripts/regenerate-abi-header.sh` need
`rustfmt` on `PATH` and therefore fail outside `devenv shell`, which is the
documented usage. The generated buffa tree churns about 1900 lines because
`CancelRequest` and `CancelResponse` are declared ahead of `message Event` and
generation follows declaration order; `--check` is clean, so the tree is
reproducible rather than hand-edited. Two foreign test files
(`crates/api-contracts/tests/contract.rs` and `crates/daemon/tests/auth.rs`) carry
their own `DaemonService` implementations and gained the `cancel` arm so they keep
compiling.

`docs/tasks/state.yaml` is the orchestration task pointer. It is left
uncommitted and outside this landing commit, as in the earlier landings, so the
commit holds only the ticket doc and the deviation report. The ticket's
`workflow_state` and the archive stay with `finalize-task`.
