---
type: task
subtype: grilling
title: How progress, plugin-originated events, and logs reach the daemon
status: stable
workflow_state: ready
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

## Completion evidence

The sink shape, the delivered fields, the ownership rule, the failure rule,
and the ordering guarantee, recorded as decisions with the alternatives
considered.
