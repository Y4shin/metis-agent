---
type: ticket
subtype: feature
title: Service-taking TCP front-end
status: stable
workflow_state: ready
blocked_by: []
size: s
---

## What to build

The TCP twin of `serve_uds_service`: a front-end that serves a mounted
`ConnectRpcService` rather than a bare `Router`, so the remote listener can
enforce the credential policy `pairing-and-tokens` added.

`connectrpc` 0.9.1 keeps the interceptor chain on `ConnectRpcService` and offers
no conversion back to a `Router`, so an auth policy cannot travel through the
landed `serve_tcp(addr, Router)` and `serve_tcp_listener(listener, Router)`.
Until this ticket lands, a TCP listener mounted through the landed surface serves
unauthenticated, which is the hole the credential work exists to close.

This is its own ticket rather than part of `pairing-and-tokens` because it is
transport wiring with its own test, and because the credential ticket's criteria
cover the lifecycle rather than the listener surfaces.

## Acceptance criteria

- [ ] `serve_tcp_service` serves a `ConnectRpcService<Router>` over TCP,
      mirroring `serve_uds_service` and reusing the same private connection body
      as `serve_stream`, with no second accept loop. If the bind-and-report shape
      needs it, a listener-taking twin accompanies it, the way `serve_tcp`
      delegates to `serve_tcp_listener`.
- [ ] A test mounts the daemon's service with `AuthPolicy::TokenRequired` over a
      real TCP socket: a call with no credential is refused with the typed
      missing-credential error, a call with a paired token is served, and the
      `Pair` procedure stays exempt.
- [ ] The landed `serve_stream`, `serve_uds`, `serve_uds_service`, `serve_tcp` and
      `serve_tcp_listener` keep their signatures and behaviour, and the existing
      transport tests are unchanged.
- [ ] No contract, generated tree, ABI layout or plugin change: the drift, lint,
      breaking and harness gates stay green.

## Blocked by

Nothing. It completes the transport surface `pairing-and-tokens` introduced, and
`remote-pairing` is blocked by it.
