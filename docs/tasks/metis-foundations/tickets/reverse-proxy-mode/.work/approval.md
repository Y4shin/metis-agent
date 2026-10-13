# Approval: `reverse-proxy-mode` architecture spec

- Approved: 2026-10-14, in chat, by the operator via the batched decision
  questionnaire that followed the draft.
- Approved artifact: `docs/tasks/metis-foundations/arch-spec.md`, section
  `## Pending ticket: reverse-proxy-mode`.
- Ticket: `docs/tasks/metis-foundations/tickets/reverse-proxy-mode/ticket.md`.

## Decisions taken at approval

1. **The daemon reads `X-Forwarded-For` and `X-Forwarded-Proto`, and only from a
   trusted peer.** No RFC 7239 `Forwarded` parser, no second precedence rule, and
   no header read at all when the peer is not named.
2. **The exposure warning is the forwarded information's only consumer.** The
   bind-time warning fires on a non-loopback listener with no trusted proxy named,
   and an honest warning fires once per process when a secret-bearing call,
   pairing, arrives on a plaintext client leg. The forwarded scheme decides the
   second, which is what makes the rule a product behaviour rather than a helper
   nothing calls. No recorded per-request client address is added.
3. **Trusted proxies are exact `IpAddr` literals**, comma separated, with loopback
   not trusted implicitly: the trust rule stays visible in configuration, which is
   what an operator audit needs.
4. **The front-end prints the startup warning at bind**, through the `eprintln!`
   `serve_tcp_listener_service` already uses for its connection errors, so any
   caller that starts the listener inherits it. The decision itself is the public
   pure function the tests assert.

## State verified before approval

- `connectrpc::PeerAddr` is public (`pub use server::PeerAddr`) and
  `RequestContext::peer_addr()` reads exactly that extension, which the library's
  own axum path stamps per connection. The landed TCP front-end discards the peer
  today (`let (stream, _) = listener.accept();` in
  `serve_tcp_listener_service`), so the stamp is the missing piece that makes the
  rule reachable.
- The same front-end already writes to stderr for its connection errors, and
  `serve_uds_service` reads its ACL from the environment inside the front-end,
  which is the precedent for resolving `METIS_TRUSTED_PROXIES` at bind.
- `ConfigError` is non-exhaustive and its own doc says the transport tickets that
  follow add their settings, so the malformed-entry variant belongs there.
  `resolve_remote_addr` treats an absent or empty variable as off and anything
  malformed as a loud error, which the new variable copies.
- The workspace has no `tracing` or `log` dependency, so a warning is stderr text
  behind a once-per-process latch rather than a logging event. That is recorded as
  a residual.
- `Pair` is the one call that carries a secret in a message body and it is exempt
  from the interceptor, so the handler is the place a per-request warning can be
  decided. The Unix socket path has no peer, so `ctx.peer_addr()` is `None` there
  and nothing is forwarded.

## Not settled here, by design

Terminating TLS in the daemon, a certificate, key or trusted-CA setting, or a
client-side fingerprint (ADR 0004 keeps all of it out), CIDR ranges in the trusted
list, walking a multi-hop forwarded chain, rate limiting or IP allow-listing,
a recorded per-request client address, a logging framework, and the deferred
question of what a client should verify about a proxied endpoint.
