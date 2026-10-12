---
type: task
subtype: prototype
title: Does the Connect and Protobuf path work here, and what does it cost?
status: stable
workflow_state: ready
blocked_by: []
---

## Question

The `protobuf-connect-research` findings recommend Protobuf with ConnectRPC Rust
at medium confidence and name five artifacts that only a spike can settle. This
task is that spike. It must answer:

1. Does a server-streaming Connect RPC work end to end over a Unix domain socket
   here: an accept loop over `tokio::net::UnixListener` calling
   `Server::serve_connection`, driven by the generated client over
   `lazy_with_connector`, reaching `PASS`?
2. What is the handwritten line count for this effort's echo plus events
   contract on the Connect path, counted the way the OpenAPI spike counted
   (non-blank, non-comment lines), so it is comparable to 326, 229, 276, and the
   125 handwritten SSE lines?
3. Does `buf generate` output pass `cargo fmt --check` and
   `cargo clippy --all-targets -- -D warnings` under the workspace lints, and
   which allow attributes does it need?
4. Does the same client and server reach each other over a Windows named pipe,
   at least as a compile-only check for `x86_64-pc-windows-msvc`?
5. Does a plaintext build link statically: does `cargo build --target
   x86_64-unknown-linux-musl` plus `ldd` or `readelf -d` show no dynamic
   dependencies?
6. If time allows: what does the transport do to a stalled subscriber, since the
   effort's slow-consumer policy currently assumes a disconnect with a resync
   requirement?

## Alternatives worth comparing

- The recommended stack: `connectrpc` 0.9.1 with `buffa` 0.9.2, `buf` 1.73.0, and
  checked-in generated code.
- `prost` plus `tonic` 0.14.6 with `tonic-prost-build`, which the research calls
  the lower-risk choice if the transport were TCP with TLS and no browser client
  were planned.
- Keep the measured OpenAPI path: client-only generation at 229 handwritten
  lines with a handwritten server.

Only build the second alternative if the first cannot be made to work, and say
so plainly if it was not built.

## Smallest artifact that answers it

A spike workspace under `spikes/connectrpc/` with its own `[workspace]` table,
never added to the root workspace members list. It holds a `.proto` carrying one
ordinary RPC and one server-streaming RPC, the generated server and client, and
one binary that serves over a Unix socket and drives both RPCs, printing `PASS`.
Keep the OpenAPI spike's contract shape so the line count is comparable.

## Environment note

`buf` is a pinned Go binary and the generation plugins are either BSR remote
plugins or locally installed binaries. Prefer the remote plugin route and
report which route worked, along with every pinned version. No JVM is needed on
this path; if something does need one, say so.

## Who reacts

The user, who must then choose between this path and the measured OpenAPI path.
The research is the baseline for the comparison, and the OpenAPI spike is the
measured counterfactual.

## What it unblocks

The architecture decision, the `contracts/` layout, the `api-contracts` crate
shape, the client stack, and the realtime transport decision. An inconclusive
result is acceptable: record what was learned, abandon the spike, and keep the
OpenAPI baseline as the fallback.

## Completion evidence

`findings.md` beside this task with frontmatter `type: findings`, `status:
stable`, answering each question with its result, the measured line count, the
exact commands including how `buf` and the plugins were obtained, the pinned
versions, and remaining uncertainty. The spike source stays in
`spikes/connectrpc/` as reference.
