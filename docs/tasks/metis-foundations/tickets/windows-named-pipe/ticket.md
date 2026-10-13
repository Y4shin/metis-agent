---
type: ticket
subtype: feature
title: Windows named pipe transport
status: stable
workflow_state: ready
blocked_by: [local-transport-rpc-skeleton]
size: s
---

## What to build

The Windows local transport, restricted to the intended account, so that a
Windows desktop client reaches the daemon without a network listener.

## Acceptance criteria

- [ ] The transport module serves and connects a Connect service over a duplex
      byte-mode named pipe.
- [ ] The pipe name comes from the platform default with an environment variable
      override, and the override is rejected unless it stays a local pipe path.
- [ ] The pipe descriptor restricts access to the daemon's account plus SYSTEM,
      and remote clients are rejected.
- [ ] The Windows path compiles for the Windows target in CI, and local test runs
      skip it with an explicit skip rather than a silent pass.
- [ ] A rejected connection from another account or a remote attempt is recorded
      as a documented gap if no Windows runner is available, naming the check
      that would close it.

## Blocked by

- `local-transport-rpc-skeleton`, because the pipe implementation shares the
  transport module and the service it carries.
