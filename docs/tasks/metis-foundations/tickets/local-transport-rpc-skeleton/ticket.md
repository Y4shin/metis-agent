---
type: ticket
subtype: feature
title: Local transport and the echo RPC skeleton
status: stable
workflow_state: ready
blocked_by: [contract-pipeline]
size: m
---

## What to build

The first complete path between two processes: shared Unix-socket listener and
connector glue, a client wrapper that hides the transport, and a minimal service,
so that a terminal-side example calls the daemon's echo RPC over the local socket
and prints the answer.

## Acceptance criteria

- [ ] A shared transport module can serve a Connect service over a Unix domain
      socket and connect a client to one, working over any asynchronous
      bidirectional stream.
- [ ] The client wrapper performs the echo RPC over the local transport without
      the caller naming a transport.
- [ ] A test starts the service on a temporary socket path and asserts the echoed
      response, and the test cleans up the socket directory.
- [ ] An example binary performs the same call and prints the response.
- [ ] The endpoint comes from the platform default, with an environment variable
      override honoured.
- [ ] The client stays free of Tauri and terminal UI dependencies.

## Blocked by

- `contract-pipeline`, because there is no service to call until the contract and
  its generated code exist.
