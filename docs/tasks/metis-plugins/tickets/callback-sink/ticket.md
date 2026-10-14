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
