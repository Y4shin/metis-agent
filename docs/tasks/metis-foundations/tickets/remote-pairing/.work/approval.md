# Approval: `remote-pairing` architecture spec

- Approved: 2026-10-13, in chat, by the operator.
- Approved artifact: `docs/tasks/metis-foundations/arch-spec.md`, section
  `## Pending ticket: remote-pairing`.
- Ticket: `docs/tasks/metis-foundations/tickets/remote-pairing/ticket.md`.

## State verified before approval

- `pairing-and-tokens` and `serve-tcp-service` both landed, so this ticket's two
  blockers are done and the credential policy can be mounted on TCP through
  `serve_tcp_service` and `serve_tcp_listener_service`.
- `daemon::secret::resolve_preshared_key() -> Result<Secret, SecretError>`, where
  an absent variable is `SecretError::Unresolved`.
- `daemon::service::service(store, preshared: Secret, policy)`.
- `daemon::auth::PairError::{WrongPresharedKey, AlreadyPaired, Store}`, and
  `AuthInterceptor` already exempts the `Pair` procedure path.
- `Client::with_remote_addr` and the fallible `with_surface` compose the remote
  case, so no new client constructor is needed.

## Decisions taken at approval

1. **Criterion 2 narrowed.** It read "then completes an authenticated RPC and an
   authenticated event stream". The ticket now asserts the RPC over the remote
   transport, and the authenticated stream is assigned to `event-stream`, which
   owns the first streaming RPC and inherits the token requirement and the
   `Pair` exemption through the interceptor. This is the same narrowing the
   operator approved for `tcp-listener`.
2. **An absent preshared key closes pairing rather than the daemon.**
   `resolve_preshared_key()` returns `Ok(None)` when `METIS_PRESHARED_KEY` is
   unset or empty, `service(...)` takes `Option<Secret>`, and `Pair` is refused
   with `PairingDisabled` while every other call keeps working. A configured but
   unresolvable reference stays a loud error. Closing pairing revokes nothing:
   tokens minted earlier keep serving, because revocation stays the token-row
   delete.
3. **Pairing refusals become typed on the client** through a new
   `metis-pair-error` header with the values `disabled`, `wrong-key` and
   `already-paired`, mirroring the credential channel. This closes the residual
   `pairing-and-tokens` recorded, where a wrong preshared key arrives as
   `ClientError::Call` carrying `PermissionDenied`.

## Not changed by this ticket

Plaintext on the remote transport, which the map records and ADR 0004 accepts
because remote exposure is meant to sit behind a reverse proxy.
`reverse-proxy-mode` owns the trusted-proxy rule and the plaintext warning. The
daemon binary still runs no listener; the remote wiring is the daemon effort's.
