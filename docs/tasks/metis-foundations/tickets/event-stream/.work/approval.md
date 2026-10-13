# Approval: `event-stream` architecture spec

- Approved: 2026-10-13, in chat, by the operator.
- Approved artifact: `docs/tasks/metis-foundations/arch-spec.md`, section
  `## Pending ticket: event-stream`.
- Ticket: `docs/tasks/metis-foundations/tickets/event-stream/ticket.md`.

## Decisions taken at approval

1. **The event ring lives in `crates/api-contracts/src/events.rs`.** The daemon
   serves it, and the client's in-process test service and the walkthrough example
   also need one, so the shared crate is the only home both reach. The example
   therefore stays at `crates/client/examples/` and `docs/testing.md` keeps its
   entry. A daemon-owned ring would have forced the example into the daemon crate,
   and a second test ring would have meant two rings to keep in step.
2. **`Cancel` is out of scope.** `cancellation-and-slow-consumers` appends the RPC
   and owns the drop policy; this ticket's `last_event_id` only carries a cursor.
3. **The payload field's wire type is delegated to the implementing agent.** JSON
   text in a `string` is the accepted fallback, a more native encoding is welcome
   if it does not pull a schema or well-known-type dependency into generation for
   little gain, and the choice must be recorded in the deviation report because
   `buf breaking` runs in FILE mode. The ticket's "What to build" section states
   the same latitude, and the arch spec records it explicitly rather than pinning
   a type the operator did not want to pin.

## State verified before approval

- `contracts/proto/metis/v1/daemon.proto` holds one service with `Echo` and the
  `Pair` RPC appended by `pairing-and-tokens`; the file's own comment says later
  tickets append to the same service and package.
- `buf.yaml` runs `STANDARD` lint with no exception yet, and its comment reserves
  the `RPC_RESPONSE_STANDARD_NAME` exception for the streaming message.
- `AuthInterceptor` already implements `intercept_streaming`, so the stream
  inherits the credential policy and this ticket supplies the wire test that
  `pairing-and-tokens` and `remote-pairing` both recorded as missing.
- `crates/client/tests/remote.rs` binds loopback listeners, and
  `crates/daemon/tests/auth.rs` has the `serve_over_tcp` and paired-token fixture
  helpers the stream tests can reuse.

## Not settled here, by design

Resume and expiry semantics, the slow-consumer drop policy, the `Cancel` RPC, the
daemon's real event sources, idle detection for heartbeats, and ring durability
across restarts. Each is assigned to a later ticket or recorded as a residual in
the section.
