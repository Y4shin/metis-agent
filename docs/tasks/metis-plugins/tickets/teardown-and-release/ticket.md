---
type: ticket
subtype: feature
title: One release entry, quarantine and shutdown reporting
status: stable
workflow_state: ready
blocked_by: [async-initialisation, call-cancellation]
size: m
---

## What to build

One asynchronous release entry: unregister the instance, cancel or drain in-flight
calls to the caller's deadline, call the plugin's shutdown entry, drop every
future, destroy the instance, unload the library, and report the ordering-ledger
entries it added. An instance that cannot be released stays mapped and is
reported, which covers a shutdown that returns a non-ok status, a quarantined
instance whose abandoned poll never returns, and an unload refused for any reason.
Disabling and enabling are registry state only and touch nothing else. The
daemon's shutdown deadline ends the wait and produces the report. The
drop-after-unload fault is proved in a subprocess, and no outstanding-clone gate is
added.

## Acceptance criteria

- [ ] The release entry performs the documented order and the ledger proves it,
      including the case where in-flight calls had to be cancelled first.
- [ ] A shutdown that returns a non-ok status, and an unload refused for another
      reason, both leave the library mapped and appear in the report.
- [ ] A quarantined instance keeps its library resident and is reported at
      shutdown.
- [ ] Disabling refuses new calls without touching in-flight calls or the library,
      and enabling accepts calls again with no reload.
- [ ] The daemon's shutdown deadline ends the wait, and the report names every
      instance that could not be released.
- [ ] The drop-after-unload fault is reproduced in a subprocess, and the release
      entry still has no outstanding-clone gate.

## Blocked by

- `async-initialisation`
- `call-cancellation`
