---
type: ticket
subtype: feature
title: TLS on the remote listener and client trust
status: deprecated
workflow_state: done
blocked_by:
- tcp-listener
size: m
---

**Superseded, not built.** The operator decided on 2026-10-13 that the daemon
does not terminate TLS at all: a reverse proxy does when the remote listener is
exposed, and client trust is deferred past the first efforts because it needs
rethinking from the ground up. ADR 0004 records the decision and its
consequences. The acceptance criteria below are kept as the record of what was
charted, not as work: no certificate setting, no pin, and no "client
certificates" check enters the daemon. `reverse-proxy-mode` owns the
trusted-proxy rule and the plaintext warning.

## What to build

A remote transport that is safe to enable: TLS on the daemon, and a client that
verifies a publicly trusted certificate through the operating system and a
self-signed one through a pinned key learned during pairing.

## Acceptance criteria

- [ ] The daemon serves TLS with rustls, reading the certificate and key from
      configuration references at startup, with self-signed generation available
      on first enable, and changing either requires a restart.
- [ ] The client verifies the operating system trust store first and falls back
      to a pinned public-key fingerprint, with hostname verification always on
      and no CA-file option.
- [ ] A test with a temporary self-signed certificate succeeds through the pinned
      path, and a mismatched pin fails closed with a typed error.
- [ ] Renewing a certificate while reusing the key does not break the pin.
- [ ] The daemon build is checked for static linking and reports no dependency on
      platform dynamic libraries.
- [ ] No client certificates are involved anywhere, and a test or check asserts
      that the daemon does not request one.

## Blocked by

- `tcp-listener`, because TLS terminates on the remote listener.
