---
type: task
subtype: grilling
title: How progress, plugin-originated events, and logs reach the daemon
status: stable
workflow_state: done
blocked_by: []
mode: human
---

## Decision to settle

The host-side surface that carries what a plugin says outside its response
payload:

- the sink shape: a caller-supplied callback interface on the host, a queue the
  daemon polls, or a channel the daemon installs, given that the runtime starts
  no thread and no runtime;
- what each callback delivers: the topic or level, the payload bytes, the
  request id, and the instance name, since several instances share one host
  today;
- whether progress and event payloads are kept, because the host currently
  releases them and records only the event topic and the log text;
- who owns a payload after delivery: the host releases the plugin buffer, so
  the sink must receive a copy or a borrow with a stated lifetime;
- what happens when the sink is slow, full, or panics: the callback is an
  `extern "C"` frame inside plugin code, so blocking there stalls the plugin's
  poll;
- ordering: whether a sink may assume events from one instance arrive in the
  order the plugin emitted them;
- whether a host with no sink is legal, which is what the tests and the
  harness want today;
- thread-safety: the contract says a callback may arrive on any thread, and
  the current implementation holds mutexes for that reason.

## Parent decisions

Already settled and not to be reopened:

- The host table's callback shapes are frozen by `docs/plugin-contract.md`
  section 6: `progress`, `emit_event`, `log`, and `service_request`.
- Panic containment is per callback frame, and every callback catches its own
  panic today, because only its own frame can.
- There is no completion callback. A call completes when a poll returns `Ok`.
- Payloads are plugin-owned when handed to the host and are released through
  the plugin's `release_buffer`, which the host may call from inside a
  callback.

## Choices already known

- The host is one per library today, and instances share it, so a sink that
  only receives a topic cannot say which instance spoke.
- `Host::logs()`, `Host::events()` and `Host::pending_services()` are the
  current test-facing accessors, and the runtime's integration tests read
  them, so any sink design has to keep those tests working or replace them.
- The daemon already owns its own event and log destinations; this decision is
  only about the boundary the runtime offers the daemon.
- A blocking sink inside a callback stalls a poll, which the contract's no
  blocking rule already covers for the plugin side and would now cover for the
  host side too.

## Recommended starting answer

A sink trait set on the host at construction, with a no-sink default for the
tests that do not care. Each callback delivers an owned copy: the instance
name, the request id where the callback has one, the topic or level, and the
payload bytes, after the plugin buffer is released. Ordering is per instance
and in emission order, which the current mutex-guarded vectors already give.
The sink must not block, and a panic inside it is caught and recorded on the
ledger like any other callback panic. The daemon decides whether it wants
backpressure by putting its own queue behind the sink.

## Dependent-task implications

- Tickets for the sink surface, for copying payloads out, for the instance name
  on each callback, and for the tests and harness checks that assert
  ordering and delivery.
- `metis-daemon` consumes the sink and maps events onto its own realtime
  stream; it does not get a second event path.
- `teardown-and-unload` has to say what happens to a sink's queued items at
  shutdown.

## Remaining fog

- Whether the sink is one trait with four methods or one method per callback
  set individually.
- Whether the request id on an event is meaningful when the event is not tied
  to a call.

## Decisions recorded

**B1, one sink trait, set on the host at construction, with a recording
default.** The callbacks are methods of one object, which gives the daemon one
thing to implement and keeps delivery on the plugin's poll path so the runtime
still starts nothing. The default implementation records into today's `logs()`,
`events()` and `pending_services()` accessors, so the runtime's tests and the ABI
harness keep working unchanged.

**B2, the delivered fields.** The instance name is on all four callbacks; the
request id is on progress and on service requests; an event carries the topic and
the payload; a log carries the level and the message. The instance name is
required because the host is one per library and instances share it, and because
a pending service records no instance today. An event carries no request id
because the frozen `emit_event` has none, so a plugin that wants correlation puts
an id in the payload, which is what that field is for. The level is a typed enum
over the four documented values.

**B3, the sink carries service requests.** Its return value is the status the ABI
expects, so the sink accepts or refuses synchronously, and the daemon answers
later through the runtime's existing `complete_service`. No new ABI entry, and no
second mechanism beside the sink.

**B4, the sink receives an owned copy.** The copy is taken before the plugin
buffer is released, so a sink can neither hold a reference into plugin memory nor
be surprised by release ordering, and it never calls `release_buffer` itself. A
borrow valid for the callback, with the release after the callback returns, is the
recorded variant if the copy cost ever matters.

**B5, the sink is infallible and must not block.** A panic inside it is caught per
frame and recorded on the ledger like today's callback panics. The runtime never
queues on the sink's behalf, so a daemon that wants backpressure puts its own
queue behind the sink, and a sink that is slow loses its own data rather than the
plugin's poll. This implies the rule stated with it: a sink must not re-enter the
runtime or the plugin, because it is called inside the plugin's poll frame.

**B6, delivery is per instance, in emission order, under that instance's lock.**
Callbacks from different instances have no ordering relationship, and nothing is
promised about a callback's position relative to its call's completion. The
implementation already serialises with mutexes, so this makes today's behaviour
the contract rather than a new mechanism.

**B7, a host with no sink is legal.** The no-sink case is the default recording
sink, so the tests that read the accessors are unchanged. At shutdown the runtime
flushes nothing, because it holds no queue of its own, and the daemon flushes or
drops its own queue before it releases the instance.

**B8, absence of panics in the host-facing frames is asserted with `#[no_panic]`
where the crate's constraints allow it.** The operator's suggestion, accepted with
the limits the crate itself documents, because those limits decide where it can
run:

- Panic detection happens at link time over the whole dependency graph, so
  `cargo check` and a library-only build prove nothing. The check must run
  wherever a linker runs, which `cargo test` does.
- It requires `panic = "unwind"`, so it cannot be enabled in a profile that
  aborts, and functions that need optimization to be proved may fail in an
  unoptimized build. It therefore goes behind a feature and a dedicated CI run at
  `opt-level = 1`, not into the default development loop, where a spurious link
  error would break the red-green cycle.
- `const fn` is unsupported, and the attribute proves nothing about the plugin's
  own C code or about a panic on the plugin's side, so the per-frame catch stays
  the runtime guarantee and the attribute is a compile-time assertion on the
  host's own frames.
- Candidates, in the order they matter: the four host callbacks, the waker's
  `wake` and `clone`, the poll and drop wrappers, the buffer release path, and the
  sink delivery path.

## Completion record

**The decision in the operator's terms.** The runtime offers one sink trait, set
on the host when it is built, with a default that records into the accessors the
tests already read. Every callback says which instance spoke; progress and
service requests also say which request; an event carries its topic and payload
and no request id, because the ABI has none; a log carries its level and message.
The sink is handed owned copies, must not block, cannot fail, and has its panics
caught and recorded, with backpressure left to the daemon that wants it. Delivery
is ordered per instance. No sink is required. The host's own callback frames are
additionally asserted not to panic at build time with `#[no_panic]`, in a CI run
that can satisfy that crate's requirements.

**Alternatives considered and rejected.**

- Four separately settable callback slots, or no trait at all with the daemon
  polling queues. Rejected: one object, one implementation for the daemon, and the
  recording default keeps the existing tests honest.
- Leaving the instance name off events and logs, or adding a request id to events.
  Rejected: instances share a host, and `emit_event` has no request id field.
- Keeping services on today's pending list while the sink carries the rest.
  Rejected: two mechanisms for one thing.
- Answering every service request with `Unsupported` in v1. Rejected: it removes
  working functionality the fixture may exercise.
- A borrow with a stated lifetime, or handing the sink the raw buffer to release.
  The first stays available; the second hands every implementer a lifetime
  obligation it will get wrong.
- A bounded queue inside the runtime, or blocking until the sink accepts.
  Rejected: the first hides loss from the daemon that owns the destination, and
  the second stalls the plugin's poll.
- Global ordering across instances, or no ordering guarantee at all.
- A mandatory sink, or discarding payloads when no sink is set.

**Constraints and rationale.** Delivery happens inside the plugin's `extern "C"`
frame, so nothing in the sink may block, unwind or re-enter, and a copy is the
only ownership rule that asks nothing of the sink. The runtime starts no queue, no
thread and no runtime, so ordering is per instance and the daemon owns any queue
and any backpressure. Panic containment stays per frame, and `#[no_panic]` is an
additional assertion on the host's frames rather than a replacement for it.

**Dependent-task implications.**

- `plugin-runtime` owns the sink trait, the recording default, the copy-out, the
  per-instance ordering lock, the ledger entry for a sink panic, and the
  `#[no_panic]` annotations behind a feature.
- `metis-daemon` implements the sink, forwards types onto its own realtime stream,
  owns its own queue and backpressure, and answers service requests through
  `complete_service`.
- CI gains a check that links with the feature enabled at `opt-level = 1` and
  `panic = "unwind"`, documented as the only place the assertion holds.
- The tickets cover the sink surface, the payload copy, the instance name on each
  callback, the ordering and delivery tests, and the `#[no_panic]` job, with the
  fixture driving at least one callback of each kind.

**Remaining fog, all spec level.**

- The exact trait and method names, which `to-spec` pins.
- Whether the level enum lives in `plugin-contract` or in the runtime, given that
  the ABI already passes it as an integer.

**Completion evidence.** The eight recorded decisions, the rejected alternatives,
the constraints and rationale, the dependent-task implications and the remaining
fog, confirmed by the operator.
