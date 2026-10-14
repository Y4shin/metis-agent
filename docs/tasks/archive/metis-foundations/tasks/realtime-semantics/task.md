---
type: task
subtype: grilling
title: Realtime protocol semantics for the WebSocket channel
status: deprecated
workflow_state: done
blocked_by: []
mode: human
---

## Decision to settle

The semantics of the realtime channel, written down far enough that
`contracts/asyncapi.yaml`, the `client`, and the daemon's event path can be
implemented without inventing behaviour on the fly:

- correlation and request IDs, and which operations are request/response over
  HTTP versus over WebSocket;
- event IDs or cursors, and what a cursor identifies;
- protocol versioning and negotiation, and what a mismatch does;
- subscriptions: how a client subscribes and unsubscribes, scoping, and what
  happens to subscriptions across a reconnect;
- cancellation of in-flight work, including acknowledgement and races;
- ordering guarantees within a subscription and across subscriptions;
- reconnection and resume, including what the client must re-request;
- expired cursors, and how a client recovers without silent gaps;
- slow consumers: queue bounds, what is dropped, and whether the connection is
  severed;
- exactly which of the above the walking skeleton's single dummy event
  exercises, and which are specified but implemented later.

## Parent decisions

Already settled and not to be reopened:

- The realtime part is designed in this effort, not deferred.
- AsyncAPI is the document format; WebSocket carries the realtime channel over
  the same bidirectional stream as HTTP, on all three transports.
- The walking skeleton implements one dummy request and one dummy event.
- The daemon owns authoritative state; the client holds no authoritative
  state and must be able to rebuild what it displays.

## Choices already known

- Request/response over HTTP versus over WebSocket is an open choice that
  drives everything else.
- Resume needs a cursor and a bounded server-side buffer, or an explicit
  resync path.
- The map's Constraints list names this full set of semantics as required
  before implementation.
- The client shares one HTTP and WebSocket API across the local and remote
  transports.

## Decisions recorded

**D1, capability parity and transport choice (superseded by D15).** The HTTP/OpenAPI surface
carries the full feature set: every capability reachable over WebSocket is
also reachable over HTTP. WebSocket is an alternate path for the same
operation, chosen per operation and possibly by client preference, never the
only path to a capability. Consequences: OpenAPI is the complete functional
contract; AsyncAPI references the same payload schemas instead of defining
parallel ones; the daemon serves the same operation regardless of transport;
and anything expressible only as a stream needs an HTTP form as well, such as
chunked responses or server-sent events.

**D2, operation addressing and framing (superseded by D15).** The OpenAPI operationId is the
canonical operation name. A WebSocket request frame is
`{correlation_id, operation, payload}` and the reply is
`{correlation_id, ok|error, payload}`, using the same payload schemas as HTTP.
Errors are typed payloads carrying a code, with the HTTP status derived from
the same code. The daemon routes both transports to the same handler per
operation.

**D3, event identity and resume.** One daemon-wide monotonic counter assigns
every event its id, held in a bounded in-memory ring of recent events. A client
resumes by presenting its last seen id, and the server replays ring events
matching that client's subscriptions. An id older than the ring returns a typed
resync error. There is no per-connection buffer and no resume token.

**D4, durability.** The event stream is ephemeral and is not the source of
truth. After a daemon restart the ring is empty, every cursor has expired, and
clients resync state over HTTP. Long-running work that survives a restart is
represented by daemon state, which the client refetches.

**D5, subscriptions (form superseded by D15).** `subscribe` and `unsubscribe` are ordinary operations
over the D2 framing, naming a channel and an optional filter. Channels are
`daemon` for lifecycle and configuration events, `session:<id>`, `task:<id>`,
and `workflow_run:<id>`, with plugin channels later. The subscribe reply carries
`{cursor, snapshot}` for that channel. Subscriptions live on the connection and
are never remembered across connections; a reconnect re-subscribes with the
stored cursor.

**D6, ordering.** One delivery queue per connection, so the order the client
observes equals daemon-wide event id order, with events for different
subscriptions interleaved in that order. Within-subscription ordering follows
from it.

**D7, cancellation.** An explicit `cancel` operation carrying the target
correlation id, acknowledged with a typed result, idempotent, and returning an
already-finished result when it lost the race. Connection loss cancels nothing,
and a final response arriving after a cancel is discarded by the client.

**D8, slow consumers.** A bounded per-connection delivery queue. On overflow
the server stops delivering, sends a typed resync-required notice, and closes
the connection. Events are never dropped silently and never stall daemon work.
The bound is configurable.

**D9, protocol versioning.** One integer major version, carried in the URL path
prefix (`/v1`) on both HTTP and the WebSocket upgrade path. An unknown version
is rejected with a typed error listing supported versions. Minor changes stay
additive and never change the path.

**D10, resync.** A resync is a fresh subscribe: the reply carries the channel
snapshot and the cursor at which it is valid, so there is no dedicated snapshot
endpoint. The snapshot contents per channel are defined in the contract.

**D11, liveness (superseded by D15).** Protocol-level WebSocket ping/pong, with a server ping
interval and an idle timeout, and a client missed-pong threshold after which it
closes and reconnects with jittered exponential backoff and its stored cursor.
There is no application-level ping operation, because keepalive is not a
capability and needs no HTTP parity. Intervals are configurable in
`metis-daemon`.

**D12, correlation IDs.** A client-generated opaque string, unique per
connection, echoed verbatim by the server and never assigned or rewritten. HTTP
may echo it in a response header for tracing. Cancellation references it.

**D13, skeleton scope (form superseded by D15).** The walking skeleton proves
the `/v1` prefix, one dummy HTTP request over the local IPC transport, and one
`daemon` channel stream delivering the dummy event. Resume, expiry,
cancellation, and slow-consumer behaviour are specified here and covered by
contract tests where the ring and the daemon are implemented.

**D14, HTTP event stream.** A subscription over HTTP is a server-sent events
response (`text/event-stream`): the snapshot arrives as an SSE event, later
events as `event` events, the cursor is the SSE `id`, resume uses the standard
`Last-Event-ID` request header, heartbeats are comment lines, and an
overflowing consumer receives a typed resync event before the connection
closes. The same channel, filter, and event schemas serve both transports.

**D15, SSE is the only realtime transport (supersedes D1, D2, D5 in form, D11,
and D13 in form).** AsyncAPI and WebSockets are dropped: the realtime protocol
is a server-sent events stream declared in the same OpenAPI document as the rest
of the API. Unchanged: the daemon-wide event id and ring (D3), ephemerality and
resync on restart (D4), one delivery queue per stream with observed order equal
to event-id order (D6), explicit idempotent cancellation (D7), disconnect with a
resync notice rather than dropped events (D8), the major version in the URL path
(D9), resync as a fresh stream beginning with a snapshot (D10), client-generated
correlation ids (D12), and SSE as the stream format (D14). Changed: the parity
rule is moot because HTTP is the only transport, the WebSocket operation
envelope is moot, a subscription is a filtered `GET` on the event endpoint
rather than a subscribe operation, liveness is SSE comment lines plus a server
idle timeout instead of ping and pong, and the skeleton exercises one HTTP
request plus one stream instead of one request on two transports.
`contracts/asyncapi.yaml` is removed.
- Which of the above the walking skeleton exercises.

## Recommended starting answer

HTTP for client-initiated request/response; WebSocket only for server-pushed
events and the subscriptions that select them. A subscription is an explicit
message per channel with an explicit unsubscribe. Every server event carries a
monotonically increasing per-connection event ID. Resume is
`since=<event_id>` against a bounded replay buffer; an expired cursor returns a
distinct typed error that forces a full state resync over HTTP rather than
leaving a silent gap. Slow consumers get a bounded per-connection queue and
are disconnected with a resync-required reason once it overflows. Cancellation
is an explicit request referencing the correlation ID with an idempotent
acknowledgement. The protocol version is exchanged during the handshake and a
mismatch is rejected with a typed error.

This is a starting answer to be grilled, not a decision. Each point can be
overturned in the conversation.

## Downstream work this may create

The `contracts/asyncapi.yaml` content, the client's reconnection, resume, and
subscription handling, the daemon's event bus and subscription bookkeeping
(in `metis-daemon`), the skeleton's dummy event, and the expiry and resync
behaviour the surfaces have to handle. Any of these can graduate into its own
ticket when the answer is recorded.

**D16, the realtime transport is a Connect server-streaming RPC.** The contract
architecture changed after this task closed: HTTP with server-sent events is gone,
and events arrive on a server-streaming RPC declared in the same proto as the
rest of the API. This supersedes D14, changes D9 (versioning is the proto package
`metis.v1`, not a URL path prefix) and D2's notion of operations (RPCs, not
operation ids). Unchanged: D3, the daemon-wide event id and ring; D4,
ephemerality and resync; D5 in substance, a request selects channels and the
stream begins with a snapshot; D6, one delivery queue per stream with ordering
structural per stream; D7, an explicit idempotent cancel RPC, where dropping a
stream cancels nothing by itself; D8, bounded backpressure and then a plain
stream drop that the client answers with a resync; D10, resync as a fresh
stream; and D12, client-generated correlation ids carried in request metadata.
Heartbeats are ordinary stream messages, and the cursor is the `last_event_id`
request field.

## Alternatives considered

- Transport split: everything over one multiplexed WebSocket, and a hybrid
decided per operation length. Both rejected in favour of HTTP carrying the full
feature set with WebSocket as a per-operation alternate path (D1).
- WebSocket framing: HTTP-shaped frames carrying method, path, status, and
headers, and a streaming-only WebSocket surface. Both rejected in favour of
shared operation IDs and a typed envelope (D2).
- Cursor identity: per-connection sequences with a resume token, and
per-subscription counters. Both rejected in favour of one daemon-wide counter
and ring (D3).
- Durability: a persisted ring, and persisting only long-running work events.
Both rejected in favour of an ephemeral stream with HTTP resync (D4).
- Ordering: one queue per subscription, and unguarded delivery. Both rejected
in favour of one delivery queue per connection (D6).
- Slow consumers: dropping the oldest events, and applying backpressure into
the daemon. Both rejected in favour of disconnect with a resync notice (D8).
- Versioning: a header or hello-frame field, and unversioned additive change.
Both rejected in favour of the version in the URL path on both transports (D9).
- Resync: a dedicated snapshot operation, and per-operation refetch with no
snapshot. Both rejected in favour of a fresh subscribe returning
`{cursor, snapshot}` (D10).
- HTTP event stream: chunked NDJSON with a `since` query parameter, and long
polling. Both rejected in favour of server-sent events (D14).

## Dependent-task implications

- `contract-generators`: its finding that no AsyncAPI to Rust generator exists
supports dropping AsyncAPI entirely. The chosen tooling must express a
`text/event-stream` response.
- `authentication-scheme`: the credential travels in the `Authorization` header
on every request, including the event stream. Version negotiation is settled
here.
- `metis-daemon`: the event bus and ring, one delivery queue per stream,
per-channel stream snapshots, explicit cancellation, and the configurable ring
size, queue bound, keepalive interval, idle timeout, and backoff parameters.
- `client`: reconnection with backoff, cursor storage and resubscribe, snapshot
application, cancellation, and the server-sent events stream behind the same
API.
- `metis-surfaces`: projections must tolerate a resync-required disconnect and
a full snapshot refresh.
- The skeleton's tickets: one dummy HTTP request over the local IPC transport,
one `daemon` channel stream, and one dummy event.

## Remaining fog

- The snapshot payload contents per channel: session, task, workflow run, and
daemon. This is contract content rather than a new decision.
- The numeric constants: ring size, queue bound, ping interval, idle timeout,
and backoff parameters. All configurable in `metis-daemon`.
- Whether plugin-produced events get their own channels and namespacing, which
lands with `metis-plugins`.
