---
type: ticket
subtype: feature
title: Local access control and surface identity
status: stable
workflow_state: ready
blocked_by: [local-transport-rpc-skeleton]
size: s
---

## What to build

Local access control from the operating system, and the identity a client
declares when it connects, so that local clients need no credential and the
daemon's logs say which surface did what.

## Acceptance criteria

- [ ] The socket and its directory are created with a configured owner and group
      and mode 700 or 770, asserted by a test that reads the resulting
      permissions.
- [ ] The connection handshake carries the protocol version, a surface name, and
      a client version, and the daemon records them; the handshake is not
      authentication.
- [ ] A mismatched protocol version is rejected with a typed error that lists
      what is supported.

## Blocked by

- `local-transport-rpc-skeleton`, because the handshake rides the local transport
  connection.
