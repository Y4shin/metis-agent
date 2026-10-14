---
type: ticket
subtype: feature
title: The sample plugin loads end to end in CI
status: stable
workflow_state: ready
blocked_by: [callback-sink, teardown-and-release]
size: m
---

## What to build

The sample plugin gate. A packaging step writes a signed bundle containing the C
fixture's library and a hand-written manifest, signing it with a test key, and an
integration test loads that bundle through the same entry the daemon uses,
dispatches a capability, observes an event through the sink, and releases the
instance cleanly with the ledger asserting the order. The gate runs as a CI step
that fails the pipeline. The daemon's compiled-in root set stays empty, so no
shipped build trusts the test key.

## Acceptance criteria

- [ ] The packaging step produces a signed bundle from the C fixture's library and
      a hand-written manifest, and it fails loudly when the manifest and the
      library disagree.
- [ ] The integration test loads the bundle through the public load entry,
      dispatches a capability, receives a response, observes at least one event,
      and releases the instance with the ledger asserting the teardown order.
- [ ] The same test fails when one byte of the bundle is altered and when the
      chain is signed by a key that is not a configured root.
- [ ] The test key is used only by the tests, and the daemon's default root set is
      empty.
- [ ] The gate runs in CI as a step that fails the pipeline, alongside the
      existing contract and ABI checks.

## Blocked by

- `callback-sink`
- `teardown-and-release`
