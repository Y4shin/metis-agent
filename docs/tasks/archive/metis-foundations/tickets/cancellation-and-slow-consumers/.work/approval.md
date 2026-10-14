# Approval: `cancellation-and-slow-consumers` architecture spec

- Approved: 2026-10-14, in chat, by the operator via the batched decision
  questionnaire that followed the draft.
- Approved artifact: `docs/tasks/metis-foundations/arch-spec.md`, section
  `## Pending ticket: cancellation-and-slow-consumers`.
- Ticket: `docs/tasks/metis-foundations/tickets/cancellation-and-slow-consumers/ticket.md`.

## Decisions taken at approval

1. **The wrapper generates the request identifier and the daemon echoes it.** A
   bounded opaque `RequestId` is generated per cancellable call from the operating
   system's random source, travels as `metis-request-id` request metadata, is
   exposed on the handle as `EventStream::request_id()`, and is echoed verbatim in
   the response metadata of unary and streaming calls alike by one interceptor both
   the daemon and the skeleton mount. The map's correlation decision therefore
   lands whole rather than as a request-side half.
2. **Criterion 2's disconnect test observes a second stream plus a cancellation
   count.** A second stream under its own identifier keeps receiving after the first
   client disconnects, its `Cancel` still answers `CANCELLED` afterwards, the ring
   still holds every published event, and the registry's cancellation count is
   unchanged by the disconnect. The count is why `Registry` carries the two read
   methods `in_flight()` and `cancellations()`, which the daemon effort can use as
   metrics.
3. **A slow-consumer drop is an in-stream error carrying a trailer header.** The
   serving stream ends with a terminal `Err` holding `metis-stream-error:
   slow-consumer` in trailers on a `ResourceExhausted` status, which is the seam
   `event-stream`'s `Result` item type was reserved for, and the wrapper gains a
   trailer read beside its response-header read so the drop arrives as the typed
   `ClientError::StreamDropped` rather than as a transport failure or a clean end.
4. **A cancelled stream ends with the same refusal mechanism, value `cancelled`.**
   The status is `Canceled`, and the wrapper reports `ClientError::StreamCancelled`,
   so a cancelled stream cannot be mistaken for one that completed. The `Cancel`
   response remains the distinguishable signal for the cancel call itself.

## State verified before approval

- `Ring::stream`'s item type is already `Result<v1::Event, ConnectError>`, and the
  `event-stream` section records it as the seam a later drop policy refuses
  in-stream through, so the policy needs no signature change.
- `Subscription::recv` currently swallows `broadcast::error::RecvError::Lagged` and
  continues, which is the silent skip this ticket replaces with the terminal
  refusal. The bound is the broadcast channel's capacity, which `Ring::new` sets to
  the ring's capacity (1024 in `daemon::service`, 64 in `client::skeleton`).
- `connectrpc::ConnectError` has `with_trailers`, the server renders a Connect
  streaming failure as an `EndStreamResponse::error` envelope carrying the error's
  trailers, and the client attaches that metadata to the error it returns from
  `ServerStream::message()`. So a trailer header is readable at the client without
  a new wire mechanism, and the response headers of an established stream cannot
  carry it.
- `connectrpc::Response` has `with_header` for any body, `StreamResponse` is
  `Response<PayloadStream>`, `UnaryResponse` is `Response<Payload>`, and
  `ConnectRpcService::with_interceptor` appends to the chain, so one echo
  interceptor can sit beside the landed `AuthInterceptor` rather than replace it.
- `CallOptions::with_header` exists and every generated method has a
  `_with_options` variant, so a per-call request header needs no client-wide
  default.
- `getrandom` is already a workspace dependency, used by `daemon::token` for token
  minting, and the client crate does not depend on it yet.
- `identity::from_headers` returns a `ClientIdentity` whose `surface()` is the
  string the registry keys on, and `Pair`'s handler already reads it the same way.

## Not settled here, by design

Out of scope for this ticket: writing into a stream, a second buffering layer or
per-subscriber queue, a configurable drop bound, retries or an automatic resync in
the wrapper, a second identifier space for requests, cancellation of unary work
that has nothing behind it, and the plugin table's own `cancel` (which stays the
plugin contract's and stops one invoke inside an instance). Each is recorded as a
residual or assigned to the daemon effort in the section.
