# Approval: `serve-tcp-service` architecture spec

- Approved: 2026-10-13, in chat, by the operator.
- Approved artifact: `docs/tasks/metis-foundations/arch-spec.md`, section
  `## Pending ticket: serve-tcp-service`.
- Ticket: `docs/tasks/metis-foundations/tickets/serve-tcp-service/ticket.md`.

## Why this ticket exists

`pairing-and-tokens` added `daemon::auth::AuthInterceptor` and
`daemon::service::service(store, preshared, policy) -> ConnectRpcService<Router>`,
and `api_contracts::transport::serve_uds_service` mounts such a service on a Unix
socket. `connectrpc` 0.9.1 keeps the interceptor chain on `ConnectRpcService` and
offers no conversion back to a `Router`, so the landed TCP front-ends
(`serve_tcp` and `serve_tcp_listener`), which take a bare `Router`, cannot carry
the policy. A TCP listener mounted through the landed surface therefore serves
unauthenticated, which is the hole the credential work exists to close. The
deviation report for `pairing-and-tokens` named this as its one blocker, the
operator chose a follow-up ticket rather than an amendment, and `remote-pairing`
was reblocked onto it.

## Decisions confirmed at approval

- Two additive functions rather than a redesign: `serve_tcp_listener_service`
  (the accept loop, delegating to the shared private `serve_connection`) and
  `serve_tcp_service` (bind plus delegate), mirroring the landed
  `serve_tcp`/`serve_tcp_listener` split so a caller that must report a
  kernel-assigned port keeps that shape.
- The landed Router-taking front-ends keep their signatures and behaviour; both
  shapes are legitimate, and the new ones are additive.
- The test mounts `AuthPolicy::TokenRequired` over a real TCP socket and asserts
  a refusal without a credential, a served call with a paired token, and the
  `Pair` exemption, reusing the auth fixture.
- A parity assertion covers both front-ends, because two families over one
  connection body is the risk this ticket introduces.
- No contract, generated tree, ABI layout or plugin change.

## Consequence recorded in the section

Two parallel front-end families now exist over one shared connection body, one
Router-taking and one service-taking, so a future front-end must choose
deliberately. The shared private body plus the parity seam is what keeps them
from diverging in behaviour.

## Follow-on

`remote-pairing` is blocked by this ticket and by `pairing-and-tokens`. Once this
lands, a remote listener can mount the credential policy, and the deferred
client-trust question (ADR 0004) is unaffected: TLS remains the reverse proxy's
job.
