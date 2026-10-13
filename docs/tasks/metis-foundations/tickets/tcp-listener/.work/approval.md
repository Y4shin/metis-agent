# Approval: `tcp-listener` architecture spec

- Approved: 2026-10-13, in chat, by the operator.
- Approved artifact: `docs/tasks/metis-foundations/arch-spec.md`, section
  `## Pending ticket: tcp-listener`.
- Ticket: `docs/tasks/metis-foundations/tickets/tcp-listener/ticket.md`.

## State verified before approval

- HEAD `c0fa15a` (`main`, pushed to `origin`, clean tree). `tcp-listener` is on
  the ready frontier; `local-transport-rpc-skeleton` landed.
- `crates/api-contracts/src/transport.rs` carries `serve_stream`, `serve_uds`,
  `uds_transport` and `local_authority`; `client::Client` has `local()` and
  `with_endpoint()` only and no public transport injection point;
  `client::skeleton` serves the skeleton router over a Unix socket; the
  transport test is `crates/client/tests/echo.rs` behind `#![cfg(unix)]`.
- `METIS_ENDPOINT` in `client::lib` is the landed configuration surface, and
  `crates/daemon` is a stub (`fn main() {}`), so no server config crate exists.
- The contract has no streaming RPC: `StreamEvents` appears nowhere in
  `contracts/proto` or `crates/api-contracts`, so `event-stream` has not landed.

## Decision taken at approval

The ticket's acceptance criterion 4 was narrowed. It read "completes both the
echo RPC and the event stream", which the graph cannot support: `event-stream`
is not a blocker and no streaming RPC exists to call. It now reads "completes
the echo RPC", with the stream-over-TCP assertion assigned to the ticket that
owns the first streaming RPC. `serve_stream` is already generic over any
asynchronous bidirectional stream, so only the RPC was missing, not the
transport.

## Surfaces the operator approved as drafted

- `transport::{serve_tcp, tcp_transport, REMOTE_ADDR_ENV, resolve_remote_addr,
  ConfigError}`, with `METIS_TCP_ADDR` as the single switch: naming an `IP:port`
  literal enables the listener, an absent or empty value disables it, and a
  malformed value or a hostname is a typed error rather than a silent disable.
- `Client::with_remote_addr`, with no remote constructor that reads the
  environment, because the bind address is the server's setting.
- `skeleton::serve_remote() -> Result<Option<SocketAddr>, ConfigError>`,
  returning the bound address so tests never guess a port.
- A new `crates/client/tests/remote.rs`, not `#[cfg(unix)]`, because TCP is the
  first transport every target has.

## Not resolved here, by design

- TLS, the proxy trust rule, and the plaintext warning are `tls-and-client-trust`
  and `reverse-proxy-mode` work; this listener is plaintext and records the
  residual.
- Authentication is `local-identity-and-acl` and `pairing-and-tokens`. Until
  they land, an enabled listener serves the same unauthenticated surface as the
  local socket, which is why the bind address has no default.
- Hostname resolution for either side is not in this ticket.
