---
type: map
title: Metis foundations
status: stable
blocked_by: []
---

# Metis foundations

## Destination

The network and plugin interfaces are real. `contracts/` is authoritative and
holds an OpenAPI document covering ordinary HTTP and the server-sent events
stream, plus shared payload schemas. The plugin
contract crate defines the manifest and the versioned C ABI. The
`api-contracts` crate carries generated Rust network types, and the `client`
crate speaks HTTP, including the event stream, to the daemon over Unix domain
sockets on Linux and macOS and duplex byte-mode named pipes on Windows, with
the same API reachable over an explicitly enabled TCP/TLS listener. A thin
end-to-end skeleton runs: the terminal asks the daemon something over the local
IPC transport, subscribes to the daemon channel, and prints the response and
the event, proving the transport and realtime assumptions before anything is
built on top.

## Constraints

- `contracts/` is the single source of truth for network types. Generated code
  stays generated, and handwritten behaviour does not live inside it.
- Network DTOs are boundary types, not the internal model of the harness,
  persistence, or plugin system. Convert at application boundaries.
- The `client` crate stays UI-independent: no Tauri, no terminal UI library.
- Local IPC is the default desktop transport. The TCP/TLS listener is off
  unless explicitly enabled.
- Unix socket directories and sockets are restricted to the intended users.
  Windows pipes are local-only with security descriptors for the intended
  users and remote pipe clients rejected.
- The detailed realtime semantics are defined before they are implemented:
  correlation and request IDs, event IDs or cursors, protocol versioning,
  subscriptions, cancellation, ordering, reconnection and resume, expired
  cursors, and slow consumers.
- Rust only. No TypeScript network client unless a standalone browser client
  is introduced.

## Decisions so far

- From `docs/rough-plan.md`, "Network contracts" and "Local and remote
  transports": OpenAPI for HTTP, AsyncAPI for WebSocket, `contracts/`
  authoritative, one endpoint implementation shared across local and remote
  transports.
- From "api-contracts": verify generator capabilities before assuming OpenAPI
  and AsyncAPI can generate complete interoperable client and server
  implementations.
- From "client": handwritten, UI-independent, sharing one HTTP and WebSocket
  API across the three transports.
- From "plugin-contract": manifest types, the versioned C ABI, and the
  Rust-facing trait and export adapters; small dependencies; SDK functionality
  feature-gated or extracted into `metis-plugin-sdk`.
- The walk is a walking skeleton first: prove the transports and the contract
  tooling end to end before widening either.
- Scope is one effort and one spec covering the whole destination: contracts,
  generated types, client, plugin contract ABI, and the walking skeleton. There
  are no separate slice specs.
- Transports in scope: Unix domain sockets for local IPC and TCP/IP for remote
  access.
- The realtime part is designed in this effort, not deferred. The walking
  skeleton implements one request and one event against dummy endpoints to
  prove the transports; the realtime semantics are defined here.
- HTTP is the only network transport, and the OpenAPI document is the complete
  functional contract for it. Realtime events are a server-sent events stream
  declared in the same document. AsyncAPI and WebSockets are dropped.
- Events carry a daemon-wide monotonic id in a bounded in-memory ring. Resume
  is by cursor, and an expired cursor or a daemon restart forces a resync. The
  event stream is ephemeral and is not the source of truth.
- Channels are `daemon`, `session:<id>`, `task:<id>`, and `workflow_run:<id>`.
  A stream selects one or more channels through query filters and begins with a
  snapshot. Streams do not survive a reconnect, so a resync is a fresh stream.
  There is one delivery queue per stream, so observed order equals event-id
  order across channels.
- Cancellation is an explicit, idempotent operation carrying the target
  correlation id; connection loss cancels nothing. A slow consumer is
  disconnected with a typed resync-required notice rather than having events
  dropped. Protocol major version lives in the URL path prefix (`/v1`) on every
  request.
- The stream's cursor is the SSE `id`, resume uses the standard `Last-Event-ID`
  request header, keepalive is SSE comment lines, the snapshot is the first
  event, and an overflowing consumer receives a resync event before the stream
  closes. A stalled stream is closed on the server's idle timeout. Correlation
  ids are client-generated opaque strings, echoed verbatim.
- The walking skeleton proves the `/v1` prefix, one dummy HTTP request over the
  local IPC transport, and one `daemon` channel stream delivering the dummy
  event.
- The plugin ABI is a small primitive set, not a C rendering of the Rust trait:
  a versioned entry point returning a function table (instance lifecycle,
  capability listing, one generic `invoke`, `cancel`, `release_buffer`,
  `shutdown`) plus host callbacks (`complete`, `progress`, `emit_event`, `log`).
  Everything else is payload. Within a major version the table is append-only,
  so a newer host reads an older plugin's shorter table. Capability descriptors
  name a kind, and V1 kinds are `tool` and `provider`.
- Two throwaway prototypes validate that ABI strategy: an ABI mechanics spike
  and an authoring trait spike. Either may be abandoned with an inconclusive
  result, which blocks neither the remaining tasks nor the spec.
- The ABI mechanics spike was conclusive and amended those decisions: the status
  enum gains `pending`, host callbacks are panic-guarded too, the host retires
  its own pending entry after a successful cancel and still releases a dropped
  completion's buffer, the host table carries a `user_data` pointer, `size` is
  the table's first field with an append-only prefix, and cbindgen needs an
  explicit export list. The generated header, the layout assertions, and the C
  compile check all work and are cheap.
- The authoring trait spike is also conclusive: a trait dispatches through one
  generic `invoke`, and only about 90 of roughly 780 translation lines know
  about the trait, so no macro is justified. It leaves two decisions for
  `plugin-contract` (the cancellation shape, and always-pending versus a
  first-poll inline attempt) and exposes gaps: no lifecycle hook, `Default`
  being required for construction, no route for `emit_event`, no consistency
  check between descriptor names and dispatch names, and an error variant that
  carries control flow.
- The remote transport uses rustls on both sides with static linking preferred
  and no dynamically linked system libraries on the daemon. Plaintext TCP is
  allowed with a warning unless reverse-proxy mode names trusted proxies. The
  bind address has no default. TLS material and configuration are read at
  startup, and changing either requires a restart. Clients check the OS trust
  store first and fall back to a fingerprint pin from pairing; no client
  certificates and no CA-file configuration.
- Local IPC authenticates with the host ACL: the daemon owns the socket as a
  configured `uid:gid` with mode 700 or 770 on Unix, and a security descriptor
  on Windows. Remote clients hold a per-surface opaque token minted at pairing
  and stored by the daemon only as a hash; revocation removes it and restarts.
  A token grants the full exposed surface with no per-client permissions, and
  every paired client can reach every agent. It travels as
  `Authorization: Bearer` on every request, including the event stream. Clients
  keep it
  in the platform keychain when possible, else in an owner-only file, and pin
  the server's public key fingerprint (SPKI). Pairing is reachable on both local
  IPC and the remote listener, gated by the preshared key, and closes by
  removing that key and restarting. Connection handshakes carry the protocol
  version, surface name, and client version.
- Windows local IPC uses tokio named pipes in both the listener and the
  connector, with a target-gated `windows-sys` for the security descriptor and
  token queries and no `interprocess` dependency. The pipe descriptor is
  `D:P(D;;GA;;;NU)(A;;GA;;;<daemon-user-SID>)(A;;GA;;;SY)`, mirroring Unix mode
  0700, with the residual that any same-user process can connect. The endpoint
  override is validated to start with the local pipe prefix.
- `plugin-contract` holds the primitives, the payload types, the authoring
  trait, and both adapters, with the authoring side feature-gated;
  `metis-plugin-sdk` re-exports them and owns the async executor, helpers, core
  plugins, and CLI. The ABI is the source of truth in `repr(C)` Rust, with a
  `cbindgen` header committed and checked in CI.
- The payload envelope is one header byte: four bits of payload format version,
  one codec bit (JSON or MessagePack), three reserved. Compatibility within an
  ABI major version is additive, with unknown fields ignored and per-capability
  versions in the descriptors. Each side releases only what it allocated, and
  panics are contained at the boundary into an internal-error status.
- These ABI decisions are provisional by the user's instruction and may be
  revised during implementation.
- The endpoint path is the platform default with an environment variable
  override.
- This effort owns `plugin-contract`: the manifest types and the versioned C
  ABI. `metis-plugins` consumes it and owns the host side, and
  `metis-plugin-sdk` wraps it for Rust authors.
- The walking skeleton's client side uses the `client` crate directly, as an
  example or test binary. It does not use `crates/terminal`, which belongs to
  `metis-daemon`.

## Fog

- None recorded yet. New fog is added here when a task surfaces a question that
  is in scope but not yet sharp enough to be a task.
- Windows pipe verification, from the `windows-pipe-security` findings: the exact
  error a remote client observes, whether a low-integrity same-user process is
  denied, and the Windows runner needed for the descriptor and pipe-mode
  assertions.
- The contract technology is open. OpenAPI with a JSON and SSE API is measured:
  229 handwritten lines for client-only generation, with SSE fully handwritten.
  Research recommends Protobuf with ConnectRPC Rust at medium confidence, which
  would remove the DTO duplication and the handwritten SSE plumbing but brings
  pre-1.0 dependencies and a handwritten named pipe. A spike is answering the
  transport, line-cost, and static-link questions before the choice is made.
- Spec-first or code-first for the HTTP contract: whether `contracts/openapi.yaml`
  stays authoritative with generated server and client code, or the daemon's Rust
  types become the source and the documents are exported from them. Whether
  generated Rust server code works well enough is being verified by a spike.
- Generated code is committed to the repository with a CI drift check that
  regenerates and fails on a diff. A Java toolchain in CI is acceptable if
  spec-first codegen is chosen. These hold whichever side owns the contract.

## Out of scope

- Provider implementations.
- Full daemon behaviour beyond the thin skeleton endpoint. `crates/daemon` and
  `crates/terminal` belong to `metis-daemon`.
- The plugin loader and runtime host, which belong to `metis-plugins`.
- The plugin authoring SDK, which belongs to `metis-plugin-sdk`.
- Agent loop, sessions, and workflows, which belong to `metis-agent-core`.
- Any UI or application shell.
