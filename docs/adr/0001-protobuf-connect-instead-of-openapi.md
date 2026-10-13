# 0001. Protobuf with ConnectRPC instead of OpenAPI with JSON and SSE

- Status: accepted
- Date: 2026-10-13

## Context

`docs/rough-plan.md` proposed OpenAPI for ordinary HTTP requests and AsyncAPI for
WebSocket realtime communication, with `contracts/` authoritative. The
`metis-foundations` Wayfinder session removed AsyncAPI and WebSockets in favour
of a server-sent-events stream in the same OpenAPI document, and then asked
whether the OpenAPI path was worth its cost at all. Two measured spikes and one
documentation assessment answered that.

Measured OpenAPI result, from `spikes/openapi-codegen/`:

- 229 handwritten lines for the client-only path, of which 125 were the SSE
  framing, resume, and drain, and 326 for generating both sides.
- Generating both sides produced two divergent DTO sets, needing 51 lines of
  conversion that could not use `From` impls because of the orphan rule.
- Generation needed a JVM and a pinned `openapi-generator-cli` jar.
- Generated output needed four module-level allows and a rustls feature change
  before it passed `fmt` and `clippy`.

Measured Protobuf result, from `spikes/connectrpc/`:

- 255 handwritten lines including a compile-only Windows named-pipe path, 213 on
  the exercised Unix path, with no separate conversion layer.
- One message set shared by both halves, since both generate from the same
  `.proto`.
- A server-streaming RPC reached PASS over a Unix domain socket, the Windows path
  compiled for `x86_64-pc-windows-msvc`, and the musl build was static-pie with no
  dynamic entries.
- No JVM: `buf` and BSR plugins generate the code.
- Generated output needed a rustfmt post-step and three allow attributes.

## Decision

The network contract is hand-written Protobuf under a versioned package,
generated with pinned `buf` plugins into committed Rust, and spoken over
ConnectRPC. `buf lint` and `buf breaking` run in CI, and a regenerate-and-diff
check fails when the committed generated code is stale.

`contracts/` stays the single source of truth, so the contract remains
spec-first. Nothing derives the `.proto` from Rust code, which keeps a client in
any language able to consume it.

## Consequences

- `api-contracts` becomes the generated Protobuf message crate plus the
  generated Connect client and server stubs, shared by the daemon and the client.
- The realtime transport is a server-streaming RPC with the cursor in the request
  rather than an HTTP `Last-Event-ID` header, and the protocol major version is
  the proto package rather than a URL path prefix.
- The API stops being a readable HTTP surface: endpoints become
  `POST /pkg.Service/Method`, reachable with `buf curl`, `grpcurl`, or Buf
  Studio rather than plain HTTP tooling.
- `connectrpc` 0.9.1 and `buffa` 0.9.2 are pre-1.0 and version-coupled, so
  upgrades cost regeneration and are pinned exactly.
- TLS over Connect with rustls and the static-linking posture remain unverified.
- The OpenAPI work is not discarded: it is the measured counterfactual in
  `spikes/openapi-codegen/`, and the alternative if Connect proves unsuitable.
