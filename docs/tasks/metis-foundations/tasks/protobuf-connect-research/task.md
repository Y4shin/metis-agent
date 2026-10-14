---
type: task
subtype: research
title: Is Protobuf with Connect or gRPC a better fit than OpenAPI with a JSON and SSE API?
status: deprecated
workflow_state: done
blocked_by: []
---

## Question

Should the network contract be Protobuf with ConnectRPC (or plain gRPC through
tonic) instead of OpenAPI with a JSON and server-sent-events API? The user
raised it because the OpenAPI spike left two costs: the event stream was
entirely handwritten, and generating both sides produced two divergent DTO
sets. Protobuf RPCs make streaming and the message types first-class, so the
question is whether that is a real win or a side-grade.

Answer these, with primary sources:

1. Which Rust tooling is current and mature: `prost` with `tonic`, the
   ConnectRPC Rust implementation, `buffa`, and `tonic-build` against
   `buf generate`. Name versions and last release dates.
2. Does the toolchain need a JVM, and does it need a `protoc` binary, or is
   there a pure-Rust path (`protox`, `buffa`, or a vendored compiler)?
3. Do server-streaming RPCs work over a Unix domain socket and a Windows named
   pipe, and what does each stack require for a custom transport (tonic
   connectors, hyper's connection API, Connect over HTTP/1.1)?
4. Is one message-type crate shared by client and server, so there is no DTO
   duplication and no conversion layer, and how is generated code committed and
   drift-checked (`buf generate` plus a diff check, or a `build.rs`)?
5. What encodings are available (binary Protobuf, Connect's JSON) and could a
   browser client reach it later without a gateway?
6. Does TLS work with rustls on both sides, matching the effort's static-linking
   preference?
7. How do the effort's settled realtime semantics map onto streaming RPCs:
   daemon-wide event ids and a cursor, resume after a disconnect, one delivery
   queue per stream with ordering equal to event-id order, slow-consumer
   disconnect with a resync requirement, explicit cancellation, and protocol
   versioning. State exactly what stays handwritten.
8. What is lost relative to OpenAPI: schema documentation quality, REST
   semantics, tooling for non-Rust clients, the error model (RPC status versus
   HTTP status), and anything the OpenAPI spike got for free.

## Decision this unblocks

Which contract technology the effort adopts, which in turn decides the
`contracts/` layout, the `client` crate's stack, the realtime transport, and
whether the codegen spike's measured numbers are the right baseline at all.

## Trusted source boundaries

- Primary: the crates' own repositories, documentation, and release notes
  (`prost`, `tonic`, `connectrpc`, `buffa`, `buf`), and the Connect, gRPC, and
  Protocol Buffers specifications.
- Local baseline: `spikes/openapi-codegen/findings.md` and
  `docs/tasks/metis-foundations/tasks/openapi-codegen-spike/findings.md`, whose
  measured numbers are 326 handwritten lines for both-sides generation, 229 for
  client-only generation with a handwritten server, 276 for typify models, SSE
  entirely handwritten at 125 lines, four module-level allows plus a rustls
  feature change, and a JVM at generation time.
- Do not run generators or build spikes for this task. This is a documentation
  and API-surface assessment, and it must not duplicate the codegen spike's
  measurements.

## Evidence required for completion

- A comparison table across the viable stacks with versions, toolchain
  requirements, streaming support, transport injection, DTO sharing, drift
  checking, and licence.
- A verdict on each of the eight questions with a source for every material
  claim.
- An explicit statement of what a Protobuf route would cost, for example the
  loss of a readable HTTP API surface and any non-Rust client friction.
- A recommendation with its confidence, plus the unresolved questions that only
  a spike could settle.

## Dependent tasks

The architecture choice, the `api-contracts` crate shape, the contract
generation decision, and the realtime transport decision. If the answer is that
Protobuf needs its own spike for the transport and streaming questions, say so
and name the smallest artifact that would settle it.
