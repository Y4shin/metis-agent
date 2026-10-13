---
type: ticket
subtype: feature
title: Pairing over the remote listener
status: stable
workflow_state: ready
blocked_by: [pairing-and-tokens, serve-tcp-service]
size: s
---

## What to build

Pairing from a device that has no access to the daemon's local socket, so that a
phone can set itself up over the network.

## Acceptance criteria

- [ ] The pairing call is reachable on the remote listener and gated by the same
      preshared key.
- [ ] A test pairs over the remote transport, then completes an authenticated RPC
      and an authenticated event stream.
- [ ] Removing the preshared key and restarting closes pairing on both
      transports, asserted by a test.

## Blocked by

- `pairing-and-tokens`, because there is no pairing call to expose remotely.
- `serve-tcp-service`, because the remote listener must mount the credential
  policy, and today only the Unix socket has a service-taking front-end.

The ticket previously listed `tls-and-client-trust` as a blocker on the grounds
that a remote pairing must not send the preshared key over plaintext. ADR 0004
removed that ticket: the daemon does not terminate TLS, a reverse proxy does, and
whether remote pairing should be gated further once a proxy is in front is part
of the deferred client-trust question. Until that is settled, pairing over the
remote listener carries the plaintext exposure the map records.
