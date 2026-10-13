# Approval: `local-identity-and-acl` architecture spec

- Approved: 2026-10-13, in chat, by the operator.
- Approved artifact: `docs/tasks/metis-foundations/arch-spec.md`, section
  `## Pending ticket: local-identity-and-acl`.
- Ticket: `docs/tasks/metis-foundations/tickets/local-identity-and-acl/ticket.md`.

## State verified before approval

- HEAD `fb3d374` (`main`, pushed, clean tree). `local-identity-and-acl` is on the
  ready frontier; `local-transport-rpc-skeleton` landed.
- `client::default_endpoint()` returns `/tmp/metis-daemon.sock`, and
  `resolve_endpoint()` prefers `METIS_ENDPOINT`. Both return `PathBuf` today, so
  both signatures gain a `Result` with this ticket.
- The map's fog entry and the landed transport section both record the conflict:
  `/tmp` is shared and sticky, so it cannot carry the configured ownership and
  mode the ACL requires.
- `connectrpc` 0.9.1 provides the whole metadata path:
  `ClientConfig::with_default_header` and `with_default_headers`, `config_mut()`,
  `RequestContext::header`, `ConnectError::new(ErrorCode, message)` and
  `ConnectError::with_headers`.
- The contract has one RPC (echo), so no proto change is needed for the
  declaration.

## Decisions taken at approval

1. **Endpoint default.** `$XDG_RUNTIME_DIR/metis/daemon.sock` when that variable
   is set and non-empty, else `$TMPDIR/metis/daemon.sock`, else the typed
   `EndpointError::NoPrivateBase`. No silent `/tmp` fallback. `METIS_ENDPOINT`
   still wins, and the ACL test uses it to point at its own temporary base.
2. **Identity channel.** Per-request metadata headers (`metis-protocol-version`,
   `metis-surface`, `metis-client-version`) set as `ClientConfig` default headers
   and read through `RequestContext`. No `Handshake` RPC and no connection state,
   because the same channel carries `Authorization` later and the stream ticket
   needs it per request anyway.
3. **Socket ACL configuration.** The daemon owns the socket directory as its own
   user. An optional `METIS_SOCKET_GROUP` names a group that user belongs to, and
   the mode follows: 700 without a group, 770 with one. No chown and no
   privileged requirement. Configured ownership for a root daemon is left to the
   daemon effort if it is ever needed.

## Deviation from the earlier outline, approved

The downstream outline previously said the `ClientError` credential variants land
in this ticket. This ticket adds no credential, because the declaration is
explicitly not authentication, so those variants move to `pairing-and-tokens`.
The section's interface contract states that, and the outline sentence is
superseded here.
