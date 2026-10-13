---
type: ticket
subtype: feature
title: Reverse-proxy mode and the plaintext warning
status: stable
workflow_state: ready
blocked_by: [tls-and-client-trust]
size: s
---

## What to build

The deployment shape where a reverse proxy terminates TLS, and an honest warning
in every other plaintext case.

## Acceptance criteria

- [ ] Configuration names trusted proxy addresses, and forwarded headers are
      honoured only when the peer is one of them and ignored otherwise.
- [ ] A non-loopback plaintext listener without named trusted proxies emits the
      startup warning, and naming a trusted proxy suppresses it.
- [ ] A test asserts forwarded information is accepted from a named proxy and
      ignored from an unnamed peer.

## Blocked by

- `tls-and-client-trust`, because proxy mode is the alternative to terminating
  TLS in the daemon and shares the listener configuration.
