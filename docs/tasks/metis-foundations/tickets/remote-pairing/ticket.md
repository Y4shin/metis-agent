---
type: ticket
subtype: feature
title: Pairing over the remote listener
status: stable
workflow_state: ready
blocked_by: [pairing-and-tokens, tls-and-client-trust]
size: s
---

## What to build

Pairing from a device that has no access to the daemon's local socket, so that a
phone can set itself up over the network.

## Acceptance criteria

- [ ] The pairing call is reachable on the remote listener and gated by the same
      preshared key.
- [ ] A test pairs over TLS using only the remote transport, then completes an
      authenticated RPC and an authenticated event stream.
- [ ] Removing the preshared key and restarting closes pairing on both
      transports, asserted by a test.

## Blocked by

- `pairing-and-tokens`, because there is no pairing call to expose remotely.
- `tls-and-client-trust`, because a remote pairing must not send the preshared
  key over plaintext.
