---
type: map
title: Metis foundations
status: stable
blocked_by: []
---

# Metis foundations

## Destination

The network and plugin interfaces are real. `contracts/` is authoritative and
holds hand-written Protobuf service definitions under a versioned package,
generated with `buf` into committed Rust. The plugin contract crate defines the
manifest and the versioned C ABI. The `api-contracts` crate carries the
generated Protobuf messages and the generated Connect client and server stubs,
and it is the one message set both sides use. The `client` crate speaks Connect
to the daemon over Unix domain sockets on Linux and macOS and duplex byte-mode
named pipes on Windows, with the same service reachable over an explicitly
enabled TCP/TLS listener. Realtime is a server-streaming RPC with a resumable
cursor. A thin end-to-end skeleton runs: the terminal calls the echo RPC and
drains the event stream over the local IPC transport, proving the transport and
streaming assumptions before anything is built on top.

## Constraints

- `contracts/` is the single source of truth for network types: hand-written
  Protobuf under a versioned package, generated with pinned `buf` plugins into
  committed Rust. Generated code stays generated, and handwritten behaviour
  does not live inside it. `buf lint` and `buf breaking` run in CI.
- The proto stays a plain Protobuf and Connect contract, free of Rust-only
  concepts, so a client in any language can consume it.
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

- From `docs/rough-plan.md`'s intent, changed by measurement: `contracts/` is
  authoritative and holds Protobuf service definitions; the wire protocol is
  ConnectRPC rather than OpenAPI and AsyncAPI; one service implementation is
  shared across local and remote transports.
- From "api-contracts": generator capabilities were verified by spikes, not
  assumed. Protobuf with ConnectRPC was measured against OpenAPI and chosen.
- From "client": UI-independent, sharing one Connect client across the three
  transports, with the transport injected and the generated client wrapped.
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
- Connect over HTTP is the only network protocol, and the proto is the complete
  functional contract. AsyncAPI, WebSockets, and server-sent events are dropped.
  Defining the API in daemon code and exporting a document was measured and is
  not taken; the spec stays first.
- Events carry a daemon-wide monotonic id in a bounded in-memory ring. Resume
  is by cursor, and an expired cursor or a daemon restart forces a resync. The
  event stream is ephemeral and is not the source of truth.
- Channels are `daemon`, `session:<id>`, `task:<id>`, and `workflow_run:<id>`.
  A stream selects one or more channels through request fields and begins with a
  snapshot message. Streams do not survive a reconnect, so a resync is a fresh stream.
  There is one delivery queue per stream, so observed order equals event-id
  order across channels.
- Cancellation is an explicit idempotent RPC carrying the target request id;
  dropping a stream cancels nothing by itself. A slow consumer sees bounded
  backpressure and then a plain stream drop, which the client answers with a
  resync, rather than events being dropped silently. The protocol major version
  is the proto package (`metis.v1`).
- The stream's cursor travels in the request (`last_event_id`), the snapshot is
  the stream's first message, and heartbeats are ordinary stream messages.
  Correlation ids are client-generated opaque strings, carried in request
  metadata and echoed verbatim in the response metadata.
- The walking skeleton proves the package version, one dummy RPC over the local
  IPC transport, and one stream delivering the dummy event.
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
  about the trait, so no macro is justified. Its two open questions are settled:
  the authoring trait carries a cancellation token on the per-request context, so
  a call blocked in I/O is dropped promptly, and the adapter always returns
  pending while the ABI keeps its inline path for direct implementers. The gaps
  it exposed are recorded: no lifecycle hook, `Default`
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
  `Authorization: Bearer` on every request, including the stream RPC. Clients
  keep it in the platform keychain when possible, else in an owner-only file,
  and pin
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
- The contract is spec-first Protobuf with ConnectRPC, generated by pinned `buf`
  plugins, with `buf lint` and `buf breaking` in CI. Generated code is committed
  with a drift check that regenerates and fails on a diff, and it needs a rustfmt
  post-step plus three allow attributes. Measured cost: 255 handwritten lines
  including the compile-only Windows path and 213 for the Unix path, against 229
  for the best OpenAPI path, whose SSE handling was entirely handwritten and
  whose generated server duplicated the DTOs. One message set, no conversion
  layer, no JVM, a static musl build, and a compile-only Windows named pipe were
  all verified.
- The contract pipeline is measured end to end: hand-written Protobuf under
  `metis.v1` generates through the two pinned BSR remote plugins into committed
  trees under `crates/api-contracts/generated/`, mounted with `include!` and two
  module-level clippy allows, one of which stays dormant until the contract
  declares its first enum. One regeneration script with a byte-for-byte drift
  check, `buf lint`, and `buf breaking` run in CI, and the everyday build needs
  no generator, `protoc`, or JVM. The rustfmt post-step takes
  `normalize_comments=true`, because buffa's multi-line doc comments otherwise
  become indented code blocks that rustdoc collects as doctests which cannot
  compile. `buf breaking` reads buf's built-in default configuration rather than
  `contracts/buf.yaml`'s `breaking` block, because the CI command runs from the
  repository root.
- The plugin boundary is real and measured: `plugin-contract` holds the
  primitives with `repr(C)` Rust as the source of truth, the generated header is
  committed and drift-checked, and a fixture `cdylib` plus a 41-check harness
  prove the ABI through a real shared-library load, covering the append-only
  prefix, the payload envelope's failure modes, per-side buffer release
  including a dropped completion, and panic containment on both sides of the
  boundary. cbindgen needs every ABI type in its `[export] include` list and
  every out-of-surface public constant in its `exclude` list, because it emits
  the crate's public constants regardless of that list. `plugin-compatibility`
  and `plugin-authoring-trait` extend this layout additively.

## Fog

- None recorded yet. New fog is added here when a task surfaces a question that
  is in scope but not yet sharp enough to be a task.
- Windows pipe verification, from the `windows-pipe-security` findings: the exact
  error a remote client observes, whether a low-integrity same-user process is
  denied, and the Windows runner needed for the descriptor and pipe-mode
  assertions.
- TLS over Connect with rustls is unverified. The spike proved a static musl
  build without TLS, not a TLS handshake.
- Pinning and churn policy for the pre-1.0 dependencies: `connectrpc` 0.9.1 and
  `buffa` 0.9.2 move together, and `buffa` has had breaking releases. Exact
  pinning, the regeneration cost of a minor bump, and when to take one.
- Whether the Connect JSON codec is enabled for outside clients, and what that
  means for the auth and error model.
- Whether the contract pipeline's BSR dependency should stay online: the
  regeneration and drift checks resolve both pinned plugins from the BSR over the
  network on every run, anonymous rate limiting can fail them for a non-code
  reason, and no `BUF_TOKEN` is pinned. A token in CI or vendored pinned plugins
  would remove the dependency, at the cost of a secret or a vendored binary.
- How the client distinguishes a deliberate slow-consumer stream drop from a
  transport failure, so the resync decision is not guesswork.

## Out of scope

- Provider implementations.
- Full daemon behaviour beyond the thin skeleton endpoint. `crates/daemon` and
  `crates/terminal` belong to `metis-daemon`.
- The plugin loader and runtime host, which belong to `metis-plugins`.
- The plugin authoring SDK, which belongs to `metis-plugin-sdk`.
- Agent loop, sessions, and workflows, which belong to `metis-agent-core`.
- Any UI or application shell.
