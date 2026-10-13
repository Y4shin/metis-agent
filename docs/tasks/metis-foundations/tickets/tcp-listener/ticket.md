---
type: ticket
subtype: feature
title: Remote TCP listener, off by default
status: stable
workflow_state: ready
blocked_by: [local-transport-rpc-skeleton]
size: s
---

## What to build

The optional remote listener: the same service reachable over TCP when the user
asks for it, with no default bind address, so that enabling remote access cannot
silently expose every interface.

## Acceptance criteria

- [ ] The service listens on TCP only when explicitly enabled in configuration.
- [ ] The bind address is required from configuration, with no default.
- [ ] With no remote configuration, no TCP socket is opened, asserted by a test.
- [ ] A test connects the client over TCP and completes the echo RPC. The
      realtime stream over TCP is asserted by the ticket that owns the first
      streaming RPC, because the contract has none until `event-stream` lands.

## Blocked by

- `local-transport-rpc-skeleton`, because the remote listener serves the same
  service and reuses the transport module.
