# Approval: `pairing-and-tokens` architecture spec

- Approved: 2026-10-13, in chat, by the operator.
- Approved artifact: `docs/tasks/metis-foundations/arch-spec.md`, section
  `## Pending ticket: pairing-and-tokens`.
- Ticket: `docs/tasks/metis-foundations/tickets/pairing-and-tokens/ticket.md`.

## Decisions confirmed at approval

1. **The preshared key travels in the `PairRequest` body**, not in a header.
2. **Token hashes live in the landed SQLite store** (`daemon::state`), with the
   hash rule the store pinned: SHA-256 over the token's canonical lowercase hex
   text. The daemon never stores the token or the preshared key.
3. **The client keeps the token text**, because the database holds only hashes
   and a hash cannot be replayed as a credential: the platform keychain through
   `secret-tool` or `security` when the tool is on `PATH`, else an owner-only file
   at `$XDG_DATA_HOME/metis/client/<surface>.token`, mode 600, reporting which
   backend answered. This is criterion 6 as written.
4. **Authorization is middleware.** `connectrpc` 0.9.1 provides
   `connectrpc::Interceptor` (`intercept_unary`, `intercept_streaming`) attached
   through `ConnectRpcService::with_interceptor`, so the spec uses an
   `AuthInterceptor` rather than a hand-rolled tower layer or per-handler calls.
   The `Pair` procedure path is the only exemption.
5. **Local IPC needs no token.** The socket's OS ACL authenticates local clients,
   matching the map and the `local-identity-and-acl` premise that local clients
   need no credential. Each listener mounts its own service configuration:
   `AuthPolicy::LocalAcl` on the local socket, `AuthPolicy::TokenRequired` on the
   TCP listener. No peer-address sniffing is needed, and pairing stays reachable
   on both transports as the map says, so a client can pair over the protected
   local socket and then use its token remotely.
6. **`api_contracts::keychain` is shared** by the daemon's `keystore:` secret
   reference and the client's credential store, because `api-contracts` is the
   only crate both depend on and two copies of the shell-out logic would drift.

## Consequences worth noting

- `intercept_streaming` authorizes a stream at establishment, so
  `event-stream` inherits its check instead of calling one, which removes the
  residual `local-identity-and-acl` recorded for per-handler recording.
- The `Pair` exemption is by procedure path, so a future pre-auth RPC must be
  added to that list deliberately; this is recorded as a residual.
- `client::skeleton` stays a second `DaemonService` implementation for the
  identity and transport tests; the daemon's real service is the one this ticket
  adds, and the daemon effort mounts it.
