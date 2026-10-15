---
type: ticket
subtype: feature
title: One callback sink for progress, events, logs and service requests
status: stable
workflow_state: ready
blocked_by: [async-initialisation]
size: m
---

## What to build

One sink trait carrying progress, events, logs and service requests, set on the
host when the host is built, with a default implementation that records into the
accessors the existing tests read, so a host with no sink stays legal. Every
callback names the instance. Progress and service requests carry the request id,
an event carries its topic and payload and no request id, and a log carries a
typed level and the message. Payloads are owned copies taken before the plugin's
buffer is released. Sink methods cannot fail and must not block, a panic inside
one is caught in its own frame and recorded, delivery is ordered per instance, and
a service request's return value is the status the ABI expects with the answer
delivered later through the completion entry. The host-facing frames are asserted
panic-free in a dedicated CI step.

## Acceptance criteria

- [ ] Each callback kind is delivered with the instance name and the fields it
      has, and events carry no request id.
- [ ] A payload copy outlives the plugin's buffer, which is released as soon as
      the callback returns, including when the callback panics.
- [ ] A panicking sink is caught and recorded, and the plugin's call still
      completes.
- [ ] Callbacks from one instance arrive in emission order, and callbacks from
      different instances are not ordered against each other.
- [ ] A service request is accepted or refused synchronously and answered later
      through the completion entry.
- [ ] A host with no sink records into the existing accessors, and the existing
      tests and the ABI harness pass unchanged.
- [ ] The panic-free check runs in its own CI step, documented as the only place
      the assertion holds, including that it proves nothing in a profile that does
      not link or does not unwind.

## Blocked by

- `async-initialisation`

## Implementation notes

Landed on `task/callback-sink` as the merge commit `a5634a7`
("ticket(callback-sink): One callback sink for progress, events, logs and service
requests"), parents `66a4ad3` (the landing branch tip) and `c611deb` (the working
branch `ticket/callback-sink`, nine `wip:` commits). The merge was clean with no
conflicts and the landing tree is byte-identical to the verified `c611deb`
(`git diff c611deb HEAD` is empty), so every source file landed exactly as
verified. The branch `ticket/callback-sink` was deleted after the merge.

The ticket diff over `66a4ad3` is nine files, 1612 insertions and 118 deletions:
the new `crates/plugin-runtime/src/sink.rs` (185 lines, the `Sink` trait, the
typed `LogLevel`, the private `Records` and the private `RecordingSink`),
`crates/plugin-runtime/src/lib.rs` (+493, `Host::with_sink`, `Host::for_instance`,
the per-instance `CallbackTarget`, the four annotated hand-off frames, the four
delivery frames and one lock discipline),
`crates/plugin-runtime/src/load.rs` (+7, the load entry naming the instance),
the new `crates/plugin-runtime/tests/sink.rs` (937 lines, 10 cases),
`Cargo.toml` (the `no-panic` workspace dependency and the `no-panic` profile),
`crates/plugin-runtime/Cargo.toml` (the optional dependency and the
`no-panic-check` feature), `Cargo.lock`, `.github/workflows/ci.yml` (+10, the
"Plugin host callback panic-free check" step of its own) and `docs/testing.md`
(+64, that step recorded as command 11 with what it proves and what it does not).

Verification ran against `c611deb`: `cargo fmt --check` clean, `cargo clippy
--all-targets -- -D warnings` clean, `cargo test -p plugin-runtime --test sink`
10 passed and 0 failed, the full `cargo test` green across every test binary with
zero failures, `cargo test --profile no-panic -p plugin-runtime --features
no-panic-check --no-run` exit 0 with no `detected panic` line, and
`cargo run -q -p plugin-fixture --bin abi-harness` reporting `[result] PASS (82
checks)`. No pre-existing test file is in the diff, so criterion 6's "existing
tests pass unchanged" holds. All seven acceptance criteria were checked against
the code and hold.

The worker recorded these divergences from the arch spec, and the deviation
report and the verifier both agree they are not blockers:

- `Host::for_instance(name)` is one new public item beyond the spec's Exports
  list, because the ABI carries exactly one opaque pointer per host table, so the
  instance name has to travel there. The load path applies it with the manifest's
  name. A caller that drives `Loaded::init_plugin` by hand without it delivers
  the empty string as the instance name. No landed dependent needs it.
- The payload handed to a sink is the decoded body, with the ABI's one-byte
  envelope stripped, matching what the runtime already does for call responses
  and capability lists. A dependent therefore observes a decoded JSON body, which
  is a contract decision it cannot infer from the trait alone.
- A host built with `Host::with_sink` answers nothing through `logs()`, `events()`,
  `pending_services()` and `take_service()`, since the recording default owns
  that storage and `Records` is crate-private. A consumer that wants both
  implements its own recording sink.
- A buffer that is not one of this build's payloads, and a log level outside the
  four the contract documents, are recorded on the ordering ledger with no
  delivery, rather than a value being invented. `LogLevel::from_abi` returns
  `Option` for the second case.
- The panic-free assertion covers the four hand-off frames, not the delivery
  frames. Measured: a frame containing `std`'s mutex, a dropped heap value or a
  dropped caught panic payload fails the attribute at every `opt-level` tried, so
  the four host frames delegate to `deliver_*` frames that hold the
  `catch_unwind`. `docs/testing.md` records this scope, and the check proves
  nothing in a profile that does not link or does not unwind.
- `no-panic` 0.1 (resolved 0.1.37) is a new optional, feature-gated,
  build-time proc-macro dependency; the crate graph is unchanged with the feature
  off, which is every shipped and default build.
- `crates/plugin-runtime/src/lib.rs` no longer calls `.lock().unwrap()`. One `lock`
  helper takes a poisoned mutex's contents, because a poisoned ledger lock would
  make a recorded sink panic unreadable. This is a behaviour change beyond the
  ticket, and the trade (a stale record instead of a second panic) is deliberate.

Residual risks carried forward: a host no load named delivers the empty string
as the instance name; the sink's borrowed `&str` arguments must be copied by a
sink that keeps them, since only payloads are owned; the default recording sink
keeps printing the progress line and a host with a custom sink prints nothing;
the panic-free step covers the hand-off frames only, leaving the delivery frames
to the per-frame catch and the sink tests; `no-panic` is pinned as `0.1`, so a
later 0.1.x that changed the attribute's mechanism would change what the CI step
proves; and `tests/sink.rs` serialises on one file-wide mutex because the
fixture's buffer ledger is process-wide, so a future leak of that lock would hang
the file rather than fail one case.
