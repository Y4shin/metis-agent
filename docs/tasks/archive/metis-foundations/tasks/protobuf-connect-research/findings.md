---
type: findings
title: Protobuf with Connect or gRPC versus OpenAPI with JSON and SSE
status: stable
---

# Protobuf with Connect or gRPC versus OpenAPI

Research for `docs/tasks/metis-foundations/tasks/protobuf-connect-research/task.md`.
Documentation and API-surface assessment only: no generator was run, no toolchain
was installed, and no spike was built.

## Baseline used

The task names `spikes/openapi-codegen/findings.md` as the local baseline. That
path does not exist in this checkout (`spikes/` is absent at the repository
root). The archived findings at
`docs/tasks/metis-foundations/tasks/openapi-codegen-spike/findings.md` carry the
same measured numbers, and every comparison below uses them:

| Baseline item | Measured value |
| --- | --- |
| Alternative A, both sides generated | 326 handwritten lines |
| Alternative B, client only plus handwritten server | 229 handwritten lines |
| Alternative C, typify models, both HTTP layers handwritten | 276 handwritten lines |
| SSE path, entirely handwritten | 125 code lines (50 server, 75 client) |
| DTO gap between the two generated sets | 51 code lines in `convert.rs` (part of A's 207) |
| Clippy normalizations | four module-level allows, two per generated crate |
| TLS friction | reqwest switched to `rustls-tls` because `openssl-sys` failed |
| Generation toolchain | JVM plus a pinned `openapi-generator-cli-7.10.0.jar` |

## Q1. Which Rust tooling is current and mature

Answer: `prost` plus `tonic` is the mature stack; ConnectRPC Rust is the newer
stack and is pre-1.0 but explicitly production-shaped; `buffa` is young; `buf`
is the generation driver for the recommended checked-in Connect path. One API
change matters: in `tonic` 0.14, `tonic-build` no longer compiles protobuf.
Service-stub generation is in `tonic-build`, and protobuf compilation moved to
`tonic-prost-build`.

Versions and last release dates, from crates.io metadata and the crates' own
repositories:

| Crate | Version | Last release | Licence | Source of truth |
| --- | --- | --- | --- | --- |
| `prost` | 0.14.4 | 2026-06-07 | Apache-2.0 | https://crates.io/api/v1/crates/prost |
| `prost-build` | 0.14.4 | 2026-06-07 | Apache-2.0 | https://crates.io/api/v1/crates/prost-build |
| `tonic` | 0.14.6 | 2026-05-07 | MIT | https://crates.io/api/v1/crates/tonic |
| `tonic-build` | 0.14.6 | 2026-05-07 | MIT | https://crates.io/api/v1/crates/tonic-build |
| `tonic-prost-build` | 0.14.6 | 2026-05-07 | MIT | https://crates.io/api/v1/crates/tonic-prost-build |
| `connectrpc` | 0.9.1 | 2026-09-21 | Apache-2.0 | https://lib.rs/crates/connectrpc |
| `connectrpc-build` | 0.9.0 | 2026-08-25 | Apache-2.0 | https://crates.io/api/v1/crates/connectrpc-build |
| `buffa` | 0.9.2 | 2026-09-03 | Apache-2.0 | https://lib.rs/crates/buffa |
| `protox` | 0.10.0 | 2026-10-07 | Apache-2.0 OR MIT | https://lib.rs/crates/protox |
| `buf` CLI | v1.73.0 | 2026-09-11 | not verified in this run | https://github.com/bufbuild/buf/blob/main/CHANGELOG.md |

Maturity signals, all quoted from primary sources:

1. **`tonic-build` in 0.14 is stubs only.** "Provides code generation for
   service stubs to use with tonic. For protobuf compilation via prost, use the
   `tonic-prost-build` crate instead."
   ([docs.rs/tonic-build](https://docs.rs/tonic-build/latest/tonic_build/))
   **Support:** direct evidence. **Confidence:** high.
2. **`prost` and `tonic` are old and heavily used.** `prost` was first
   published 2017-06-25 with 41 releases and about 4,900 dependents; `tonic` was
   first published 2018-07-18 with 3,666 dependents; `tonic-build` has 49
   releases. (crates.io API, links above.) **Support:** direct evidence.
   **Confidence:** high.
3. **ConnectRPC Rust is pre-1.0 by its own declaration.** "**Status:** pre-1.0.
   The API surface is settling but may shift in 0.x. Production-quality runtime:
   passes the full ConnectRPC conformance suite, 3,600 server and 6,872 client
   tests across the three protocols." MSRV Rust 1.88, edition 2024.
   ([README](https://github.com/connectrpc/connect-rust/blob/main/README.md))
   **Support:** direct evidence. **Confidence:** high.
4. **The `connectrpc` crate name has a discontinuity.** The 0.2.0 changelog
   entry reads: "First release from the [anthropics/connect-rust] repository.
   This is a complete from-scratch implementation, not a continuation of the
   0.1.x releases previously published under the `connectrpc` crate name, which
   have been superseded." The 0.1.x entries on crates.io were published by a
   different account and were MIT rather than Apache-2.0.
   ([CHANGELOG](https://github.com/connectrpc/connect-rust/blob/main/CHANGELOG.md),
   [crates.io](https://crates.io/api/v1/crates/connectrpc-build)) **Support:**
   direct evidence. **Confidence:** high. **Researcher inference:** a reviewer
   should check the crates.io ownership history before adopting, not because
   anything is wrong with the code but because the name previously belonged to
   another publisher.
5. **`buffa` is new and moving fast.** First published 2026-03-14, 15 releases,
   8 of them breaking. ([lib.rs/crates/buffa](https://lib.rs/crates/buffa))
   **Support:** direct evidence. **Confidence:** high.
6. **`buffa` and `connectrpc` have a hard coupling.** The connectrpc workspace
   comment states: "The 0.9.x floor is hard rather than a preference ... 0.9.0
   widens `types::put_len_delimited_header` ... so code emitted by a 0.8.x
   codegen does not compile against a 0.9 runtime ... bump the lock and
   regenerate together."
   ([workspace `Cargo.toml`](https://github.com/connectrpc/connect-rust/blob/main/Cargo.toml))
   **Support:** direct evidence. **Confidence:** high.
7. **`tonic` master is preparing breaking changes.** "tonic's master branch is
   currently preparing breaking changes. For the most recently released code,
   look to the 0.14.x branch."
   ([docs.rs/crate/tonic](https://docs.rs/crate/tonic/latest)) **Support:**
   direct evidence. **Confidence:** high.

## Q2. Does the toolchain need a JVM, and does it need `protoc`

Answer: **no JVM on any Rust protobuf path.** Every generator above is a Rust
crate or a single Go binary (`buf`). This is a straight removal of the measured
JVM cost from the OpenAPI spike, where generation needed an ephemeral
`nix shell nixpkgs#jdk21` plus a pinned jar.

**`protoc` is required by default in every `build.rs` path, and avoidable by
three documented routes.**

1. `prost-build` needs `protoc`: "`prost-build` depends on the Protocol Buffers
   compiler, `protoc`, to parse `.proto` files ... `prost-build` will search the
   current path for `protoc` or `protoc.exe` ... When `protoc` is installed in a
   different location, set `PROTOC` ... If `prost-build` can not find `protoc`
   via these methods the `compile_protos` method will fail."
   ([docs.rs/prost-build](https://docs.rs/prost-build/latest/prost_build/))
   `prost-build` also documents compiling `protoc` from source via the
   `protobuf-src` crate and `Config::protoc_executable`.
   **Support:** direct evidence. **Confidence:** high.
2. `tonic-prost-build` delegates to `prost-build`, so the same requirement
   applies.
   ([docs.rs/tonic-prost-build](https://docs.rs/tonic-prost-build/latest/tonic_prost_build/))
   **Support:** interpretation, from the crate's stated purpose and its
   `Config` re-exporting prost configuration. **Confidence:** medium-high.
3. `connectrpc-build` needs `protoc` too: "It shells out to `protoc` (or `buf`,
   or reads a precompiled `FileDescriptorSet`) to obtain descriptors" and
   "Requires `protoc` on `PATH` (or set via `PROTOC`). To use `buf` instead, call
   `Config::use_buf`. To avoid both, precompile a `FileDescriptorSet` once and
   ship it alongside your source via `Config::descriptor_set`."
   ([docs.rs/connectrpc-build](https://docs.rs/connectrpc-build/latest/connectrpc_build/),
   [lib.rs source](https://docs.rs/connectrpc-build/latest/src/connectrpc_build/lib.rs.html))
   This directly contradicts the user guide's phrasing that the build-time path
   needs "no extra binaries needed"; see Contradictions.
   **Support:** direct evidence. **Confidence:** high.
4. `buffa-build` needs `protoc` too: "By default, requires `protoc` on the
   system PATH (or set via the `PROTOC` environment variable), the same as
   `prost-build` and `tonic-build`. If `protoc` is unavailable or outdated on
   your platform, `buf` can be used instead, see `Config::use_buf()`. Alternatively,
   feed a pre-compiled descriptor set via `Config::descriptor_set()`."
   ([docs.rs/buffa-build](https://docs.rs/buffa-build/latest/buffa_build/))
   **Support:** direct evidence. **Confidence:** high.

The three routes that avoid a `protoc` binary:

- **`protox` (pure Rust).** "An implementation of the protobuf compiler in rust,
  intended for use as a library with crates such as `prost-build` to avoid
  needing to build `protoc`." Its README shows usage with both `prost-build` and
  `tonic-build`, and it has a `bin` feature for a CLI. Version 0.10.0, MSRV
  1.85, Apache-2.0 OR MIT.
  ([github.com/andrewhickman/protox](https://github.com/andrewhickman/protox),
  [lib.rs/crates/protox](https://lib.rs/crates/protox)) **Support:** direct
  evidence. **Confidence:** high.
- **`buf generate` (Go binary replaces `protoc`).** "`local:` plugins are
  invoked directly; `protoc` itself isn't needed for this walkthrough" and "The
  Buf CLI replaces a typical `protoc` workflow with two commands: `buf build`
  for compilation, `buf generate` for per-language code generation."
  ([buf docs, quickstart](https://buf.build/docs/generate/tutorial/),
  [migrate from protoc](https://buf.build/docs/migration-guides/migrate-from-protoc/))
  `buf` is the path connectrpc recommends for checked-in code, and it is also
  the path that removes the need for `protoc` entirely.
  **Support:** direct evidence. **Confidence:** high.
- **A precompiled `FileDescriptorSet`.** `connectrpc-build` and `buffa-build`
  both accept one, which removes the compiler from the build entirely at the
  cost of a separate precompile step and a file to keep in sync.

**Verdict:** JVM no. `protoc` yes by default, with a documented pure-Rust
replacement (`protox` 0.10.0) for prost and tonic, and a documented Go-binary
replacement (`buf` 1.73.0) for every stack. There is no configuration that uses
neither a `protoc` binary nor some non-Rust compiler binary, unless you commit a
precompiled descriptor set.

## Q3. Server-streaming over a Unix domain socket and a Windows named pipe

Answer: **Unix domain sockets work on both stacks and are documented on both.
Windows named pipes are built in on neither stack; they are handwritten on both,
and their correctness is unverified in this effort.**

Protocol-level facts first, because they decide how hard the transport is:

- Connect supports three of the four RPC shapes over HTTP/1.1, and only
  bidirectional streaming needs HTTP/2: "When used with Protocol Buffer schemas,
  the Connect protocol supports unary, client streaming, server streaming, and
  bidirectional streaming RPCs, with either binary Protobuf or JSON payloads.
  Bidirectional streaming requires HTTP/2, but the other RPC types also support
  HTTP/1.1. The protocol doesn't use HTTP trailers at all, so it works with any
  networking infrastructure."
  ([Connect Protocol Reference](https://connectrpc.com/docs/protocol/))
  **Support:** direct evidence. **Confidence:** high.
- gRPC is HTTP/2 framing only: headers in HEADERS frames, messages in DATA
  frames with a length prefix, status in trailing HEADERS. The canonical
  example is `HEADERS (END_STREAM) <Length-Prefixed Message>` for the request and
  a trailing `HEADERS (END_STREAM, END_HEADERS) grpc-status = 0` for the
  response. ([gRPC over HTTP2](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md))
  **Support:** direct evidence. **Confidence:** high.

Consequences for a UDS transport:

- **tonic server:** `Server::serve_with_incoming` takes an incoming stream of
  IO. tonic ships exactly this for Unix sockets: its `uds` example binds a
  `tokio::net::UnixListener`, wraps it in
  `tokio_stream::wrappers::UnixListenerStream`, and passes it to
  `serve_with_incoming`, with `tonic::transport::server::UdsConnectInfo` in
  request extensions for peer address and credentials.
  ([examples/src/uds/server.rs](https://github.com/hyperium/tonic/blob/master/examples/src/uds/server.rs),
  [UdsConnectInfo](https://docs.rs/tonic/latest/tonic/transport/server/struct.UdsConnectInfo.html))
  **Support:** direct evidence. **Confidence:** high.
- **tonic client:** `Endpoint::connect_with_connector`: "This allows you to build
  a `Channel` that uses a non-HTTP transport. See the `uds` example for an
  example on how to use this function to build channel that uses a Unix socket
  transport." The connector must return a type implementing
  `hyper::rt::Read + rt::Write + Send + Unpin`.
  ([docs.rs/tonic Endpoint](https://docs.rs/tonic/latest/tonic/transport/struct.Endpoint.html),
  [endpoint.rs](https://github.com/hyperium/tonic/blob/master/tonic/src/transport/channel/endpoint.rs))
  **Support:** direct evidence. **Confidence:** high.
- **connectrpc client:** first-class Unix socket support, `connect_unix` and
  `lazy_unix` (available on Unix only), described as wrappers over
  `lazy_with_connector`. The generic escape hatch is described as: "The
  connector may return any stream implementing `hyper::rt::Read + Write + Send +
  Unpin` ... This is the escape hatch for transports the built-in constructors
  don't cover (Unix sockets, in-memory pipes, pre-wrapped mTLS, etc.), the same
  pattern as tonic's `Endpoint::connect_with_connector`."
  ([docs.rs/connectrpc Http2Connection](https://docs.rs/connectrpc/latest/connectrpc/client/struct.Http2Connection.html))
  **Support:** direct evidence. **Confidence:** high.
- **connectrpc server:** no built-in UDS listener. The built-in `Server` binds
  TCP only (`Server::bind` takes `ToSocketAddrs`; `from_listener` takes a
  `tokio::net::TcpListener`). The documented route is a custom accept loop:
  "When you need a policy at accept time or per connection that the built-in loop
  does not have ... listen on a Unix socket, write the loop yourself and hand
  each accepted stream to `Server::serve_connection` (or the free
  `server::serve_connection`, which takes any tower HTTP service such as an
  `axum::Router`)."
  ([guide, custom accept loops](https://github.com/connectrpc/connect-rust/blob/main/docs/guide.md),
  [docs.rs/connectrpc Server](https://docs.rs/connectrpc/latest/connectrpc/server/struct.Server.html))
  **Support:** direct evidence. **Confidence:** high.
  **Researcher inference:** this is the Connect analogue of the spike's 29-line
  `serve_uds` glue, and it is a comparable amount of code, because
  `serve_connection` supplies the timeouts, retirement, GOAWAY and peer stamping
  that the raw hyper path does not.

Consequences for a named-pipe transport:

- tonic has no built-in named-pipe support. The request is an open issue with
  users writing adapters to make `tokio::net::windows::named_pipe` satisfy
  tonic's incoming-stream requirement, and third-party crates exist that
  implement the `Connected` trait for a named-pipe server.
  ([tonic issue 1518](https://github.com/hyperium/tonic/issues/1518),
  [catalinsh/tonic-named-pipe-example](https://github.com/catalinsh/tonic-named-pipe-example),
  [cf-modkit-transport-grpc](https://docs.rs/crate/cf-modkit-transport-grpc/latest/source/src/windows_named_pipe.rs))
  **Support:** direct evidence for the absence, from a community issue and
  community crates rather than project docs. **Confidence:** medium-high.
- connectrpc has the same shape: `lazy_with_connector` on the client (with a
  `#[cfg(windows)]` named-pipe client stream wrapped in
  `hyper_util::rt::TokioIo`) and a custom accept loop around
  `Server::serve_connection` (with `NamedPipeServer`) on the server. Nothing in
  the crate's docs names Windows named pipes.
  **Support:** researcher inference from the documented connector contract.
  **Confidence:** medium.
- The effort has **no verified named-pipe evidence at all** today. The OpenAPI
  spike recorded that `reqwest`'s `windows_named_pipe` is behind `#[cfg(windows)]`
  and was "compiled on Linux, so it is compile-only here and its validity on
  Windows is not verified." That caveat carries over unchanged to any protobuf
  stack. **Support:** direct evidence from the local baseline.
  **Confidence:** high.

**Researcher inference:** for a named pipe, Connect is the lower-risk choice
even though neither stack has built-in support, because a server-streaming
Connect RPC can run over HTTP/1.1, so the pipe needs only a hyper HTTP/1.1
connection driver, which the effort has already proven on this codebase. An h2
handshake over a pipe adds a second thing that can go wrong and that nobody here
has tested.

## Q4. One shared message-type crate, and how drift is checked

Answer: **yes, one message-type set on both stacks, generated from the `.proto`.
The spike's two divergent DTO sets and the 51-line `convert.rs` disappear.**
Drift checking is either `buf generate` plus a diff, or a `build.rs` with no
committed output at all.

1. **`buf generate` path (checked-in code).** Two plugins produce the two
   trees: `protoc-gen-buffa` for message types and `protoc-gen-connect-rust` for
   service stubs, with `protoc-gen-buffa-packaging` assembling the `mod.rs`
   trees. The service tree references the message tree through a path option:
   "`buffa_module=crate::proto` tells the service-stub generator where you
   mounted the buffa output ... The underlying option is
   `extern_path=.=crate::proto`."
   ([README](https://github.com/connectrpc/connect-rust/blob/main/README.md))
   **Support:** direct evidence. **Confidence:** high.
2. **`build.rs` path.** "Output is unified: message types and service stubs in
   one file per proto, included into your crate with
   `connectrpc::include_generated!()`."
   ([guide, code generation](https://github.com/connectrpc/connect-rust/blob/main/docs/guide.md))
   `tonic-prost-build` does the same for prost plus tonic stubs, into
   `OUT_DIR`. **Support:** direct evidence. **Confidence:** high.
3. **Drift checking for committed output.** `buf` gives the same shape as the
   spike's `regenerate.sh --check`: regenerate into the tree, then
   `git diff --exit-code`. `clean: true` is the piece that makes it trustworthy,
   because it removes stale files: "`clean: true` deletes the previous contents
   of each `out` directory before generation runs, so removed `.proto` types
   don't leave behind stale generated files."
   ([buf quickstart](https://buf.build/docs/generate/tutorial/)) **Support:**
   direct evidence. **Confidence:** high.
4. **Schema compatibility gating, which OpenAPI did not give for free.**
   `buf breaking --against <baseline>` "compares the current version of your
   Protobuf schema against a past version and reports any changes that would
   break clients, servers, or the code generated from those schemas", and
   `buf lint` covers style. This is a stronger compatibility gate than a diff of
   generated code, and it is a genuine addition relative to the OpenAPI spike.
   ([buf breaking](https://buf.build/docs/breaking/)) **Support:** direct
   evidence. **Confidence:** high.
5. **A generated-code duality to expect.** buffa emits both `MyMessage` (owned)
   and `MyMessageView<'a>` (zero-copy), plus `OwnedView<V>`, and the service
   stubs depend on those views. So "one message type" means one schema and one
   type family, not one struct per message.
   ([buffa README](https://github.com/anthropics/buffa/blob/main/README.md))
   **Support:** direct evidence. **Confidence:** high.

## Q5. Encodings available, and whether a browser can reach it later

Answer: **binary Protobuf and proto3 JSON on the same routes, plus gRPC and
gRPC-Web from the same server. A browser can reach it with no gateway.**

1. **Two codecs, negotiated per request.** "The Connect protocol supports two
   message codecs: binary proto and proto3 JSON." Content types are
   `application/proto` and `application/json` for unary, and
   `application/connect+proto` and `application/connect+json` for streaming.
   ([guide](https://github.com/connectrpc/connect-rust/blob/main/docs/guide.md),
   [Connect Protocol Reference](https://connectrpc.com/docs/protocol/))
   **Support:** direct evidence. **Confidence:** high.
2. **JSON can be turned off for proto-only deployments,** in which case JSON is
   rejected at content negotiation with a bodyless HTTP 415 before the body is
   read. **Support:** direct evidence from the guide. **Confidence:** high.
3. **Curl-ability and a GET form.** The spec's own examples show
   `POST /connectrpc.greet.v1.GreetService/Greet` with a JSON body and a
   `GET ...?encoding=json&message=...` form for methods annotated
   `idempotency_level = NO_SIDE_EFFECTS`.
   ([Connect Protocol Reference](https://connectrpc.com/docs/protocol/))
   **Support:** direct evidence. **Confidence:** high.
4. **Three protocols from one server.** The 0.2.0 changelog support table lists
   Connect (unary and streaming) server and client, Connect GET, gRPC over
   HTTP/2, and gRPC-Web, each as server and client.
   ([CHANGELOG](https://github.com/connectrpc/connect-rust/blob/main/CHANGELOG.md))
   **Support:** direct evidence. **Confidence:** high.
5. **Browser without a gateway.** Connect-ES provides
   `@connectrpc/connect` and `@connectrpc/connect-web`, where the latter is
   "Adapters for web browsers and any other platform that has the fetch API on
   board", and the server side needs no proxy because Connect is plain HTTP.
   ([connect-es README](https://github.com/connectrpc/connect-es)) The Rust
   side already has the matching escape hatch: the `wasm-client` example is a
   "Browser fetch transport: same generated client used from
   `wasm32-unknown-unknown` with a custom `ClientTransport` backed by
   `web_sys::fetch`".
   ([guide, examples tour](https://github.com/connectrpc/connect-rust/blob/main/docs/guide.md))
   **Support:** direct evidence. **Confidence:** high.
6. **This is where gRPC alone loses.** A tonic-only route speaks
   `application/grpc+proto` and puts status in HTTP/2 trailers, so a browser
   needs gRPC-Web plus a proxy, and a JSON surface needs transcoding. Connect
   gets the browser and the JSON surface by choosing a different protocol, not
   by adding infrastructure. **Support:** researcher inference from the gRPC
   HTTP/2 protocol document and the Connect spec. **Confidence:** high.

## Q6. rustls on both sides, and static linking

Answer: **yes on both stacks, and neither needs native-tls, so the spike's
`openssl-sys` failure and its rustls feature change both disappear. Static
linking is plausible but unverified.**

1. **tonic offers both rustls providers.** `tls-ring` "Enables the `rustls`
   based TLS options for the `transport` feature using the `ring` libcrypto
   provider", `tls-aws-lc` does the same with `aws-lc-rs`, and
   `tls-native-roots` / `tls-webpki-roots` add trust roots. The crate features
   are `tls-ring: ["_tls-any", "tokio-rustls/ring"]` and
   `tls-aws-lc: ["_tls-any", "tokio-rustls/aws-lc-rs"]`.
   ([docs.rs/tonic](https://docs.rs/tonic/latest/tonic/), crates.io feature map)
   **Support:** direct evidence. **Confidence:** high.
2. **connectrpc uses rustls and only rustls.** Features are `server-tls`,
   `client-tls` and an umbrella `tls`; the runtime takes a
   `rustls::ServerConfig` (`Server::with_tls`), and `connectrpc::axum::serve_tls`
   is a drop-in replacement for `axum::serve`.
   ([guide, TLS](https://github.com/connectrpc/connect-rust/blob/main/docs/guide.md))
   **Support:** direct evidence. **Confidence:** high.
3. **The Connect stack's provider is `aws-lc-rs`, with no option to switch.**
   The workspace `Cargo.toml` states: "No crypto provider feature here,
   tokio-rustls already pulls in aws-lc-rs. Enabling a second provider (ring)
   would make rustls's builder() panic at runtime because it can't auto-detect
   which one to use."
   ([Cargo.toml](https://github.com/connectrpc/connect-rust/blob/main/Cargo.toml))
   **Support:** direct evidence. **Confidence:** high.
   **Researcher inference:** this is the one place where the static-linking
   preference needs checking. rustls itself is pure Rust, but `aws-lc-rs` and
   `ring` both compile C (and assembly), and I did not verify a musl or fully
   static build for either in this devenv. tonic at least lets you pick the
   provider; connectrpc does not.
4. **No TLS is needed for the local transport at all.** h2c over a Unix socket,
   or Connect over HTTP/1.1 over a Unix socket, both run cleartext. TLS only
   enters if the daemon is reachable over TCP. **Support:** researcher
   inference. **Confidence:** high.

## Q7. How the settled realtime semantics map onto streaming RPCs

Answer: overall the framing, cancellation and ordering machinery becomes
generated or structural, and the replay, fan-out and backpressure policy stays
handwritten. The 125 handwritten SSE lines are not all saved: the 50-line server
writer and the 75-line client frame parser go away, and the replay and queue
logic they sat next to does not.

Point by point:

1. **Daemon-wide event ids and a cursor: stays handwritten, but becomes a
   schema field.** The ids live in the message (`uint64 event_id`, or a cursor
   message) and the assignment, persistence and gap detection are application
   code. Nothing in the RPC layer allocates or orders ids.
   **Support:** researcher inference from the RPC model; there is no id
   allocation anywhere in the protocol specs. **Confidence:** high.
2. **Resume after a disconnect: protocol-free, so it is an application
   protocol.** Connect has no equivalent of SSE's `Last-Event-ID`, and gRPC has
   none either. The shape is a server-streaming RPC whose request message
   carries `from_cursor`, and a server that replays from its buffer. Generated:
   the client method, the streaming codec, error propagation on a truncated
   stream. Handwritten: the replay buffer, cursor validation, and the
   definition of "resync required".
   **Support:** researcher inference from the Connect spec (no resume
   mechanism) and the gRPC HTTP/2 document (no resume mechanism).
   **Confidence:** high.
3. **One delivery queue per stream with ordering equal to event-id order:
   structural on the transport side, handwritten for fan-out.** gRPC "guarantees
   message ordering within an individual RPC call", and a single
   server-streaming response is one ordered sequence on one HTTP/2 stream (or
   one chunked HTTP/1.1 body). Multi-subscriber fan-out, meaning a daemon-wide
   bus feeding per-stream queues, is not provided: that is a `tokio::broadcast`
   channel or per-subscriber `mpsc` plus a sequence assertion, and it is the same
   code the SSE path needed.
   ([gRPC core concepts](https://grpc.io/docs/what-is-grpc/core-concepts/))
   **Support:** direct evidence for ordering; researcher inference for the
   fan-out. **Confidence:** high.
4. **Slow-consumer disconnect with a resync requirement: stays handwritten.**
   There is no built-in policy. The transport supplies backpressure instead
   (the response stream is polled as the socket drains), which means a slow
   consumer slows the handler rather than being dropped. Dropping it needs a
   bounded per-subscriber queue plus a timeout or lag check, then ending the
   stream with an error. connectrpc offers two adjacent mechanisms worth reusing:
   `DeadlinePolicy::with_inter_message_timeout(d)` "detects stalled streams",
   and `with_enforce_on_streams(true)` ends a stream when the deadline passes
   ("the next item after the deadline is a `deadline_exceeded` error and the
   stream ends"). Those bound time, not queue depth, so they are a partial fit.
   ([guide, deadline policy](https://github.com/connectrpc/connect-rust/blob/main/docs/guide.md))
   **Support:** direct evidence for the mechanisms; researcher inference for
   the gap. **Confidence:** medium-high.
5. **Explicit cancellation: structural, and the client half is free.** The
   ConnectRPC guide is explicit: "Dropping a client-streaming call cancels it:
   the request body is dropped with the future", "dropping the receive half
   cancels the RPC", and on the server "Cancellation drops the inner stream at
   the next yield point with no grace period; spawn commit-critical work off the
   request future if it must outlive the caller." Per-call timeouts arrive as
   `Connect-Timeout-Ms` or `grpc-timeout` headers and can be clamped server-side
   by `DeadlinePolicy`. The OpenAPI/SSE path had none of this: dropping an SSE
   response was just closing a connection.
   ([guide](https://github.com/connectrpc/connect-rust/blob/main/docs/guide.md))
   **Support:** direct evidence. **Confidence:** high.
   One recent and relevant hardening: 0.9.1 fixed a case where "a client- or
   bidi-streaming call that ends before its request body no longer lets a
   stalled client hold that body indefinitely", with the drain bounded to 5
   seconds and 1 MiB.
   ([CHANGELOG 0.9.1](https://github.com/connectrpc/connect-rust/blob/main/CHANGELOG.md))
   **Support:** direct evidence. **Confidence:** high.
6. **Protocol versioning: partly free, partly policy.**
   - Schema versioning is conventional and enforced by tooling: package names
     such as `greet.v1` plus `buf breaking` against a baseline
     ([buf breaking](https://buf.build/docs/breaking/)).
   - Protocol-level revision exists as the `Connect-Protocol-Version` header and
     the `connect=v1` query parameter: "The `Connect-Protocol-Version` header
     distinguishes unary Connect RPC traffic from other requests that may use the
     same Content-Type. (In the future, it may also be used to support revisions
     to this protocol.)"
     ([Connect Protocol Reference](https://connectrpc.com/docs/protocol/))
   - Schema semantics have formal editions: `edition = "2023"` / `"2024"` replace
     `syntax`, with per-feature defaults
     ([Protobuf Editions overview](https://protobuf.dev/editions/overview/)).
     ConnectRPC Rust advertises edition 2024 support: "`protoc-gen-connect-rust`
     now advertises proto edition 2024 as its maximum supported edition".
   - Handwritten: the policy itself (when to cut v2, how long v1 is served, how
     clients pin), and event-payload versioning beyond protobuf field
     compatibility. Field-number compatibility means additive event changes need
     no version bump, which the JSON/SSE path did not give you.
   **Support:** direct evidence for the mechanisms; researcher inference for the
   policy gap. **Confidence:** high.

**Exactly what stays handwritten on a Protobuf route:**

- The service implementation bodies (the analogue of the spike's 103-line
  `bin/spike.rs` `ApiImpl` plus driver).
- The event producer, its cursor assignment, and its replay buffer.
- The per-subscriber queue, the lag or stall policy, and the resync protocol.
- The accept loop for a Unix socket or a named pipe, plus (for named pipes only)
  the platform connector and server wrappers.
- At least one crate-root lint allow for generated trait refinement:
  `#![allow(refining_impl_trait_internal, refining_impl_trait_reachable)]`.
- The `.proto` files themselves, `buf.yaml` and `buf.gen.yaml`, and the pinned
  plugin versions.

**What stops being handwritten:** the SSE response writer, the SSE client frame
parser and reconnect plumbing, `Last-Event-ID` header handling, heartbeats,
content-type and framing negotiation, the error-to-HTTP mapping, request size
and message size limits, deadline clamping, health and reflection services, and
the conversion layer between two DTO sets.

## Q8. What is lost relative to OpenAPI

1. **No browsable API document as the primary artifact.** OpenAPI produces a
   YAML or JSON document a human reads and a tool consumes. Protobuf produces a
   descriptor set, which is a binary artifact. The replacement is discovery:
   connectrpc ships `connectrpc-reflection` implementing
   `grpc.reflection.v1` and `v1alpha`, "so schema-aware clients, `grpcurl`,
   `buf curl`, Postman, `grpcui`, can discover and call your services without
   local proto files, over gRPC, gRPC-Web, and the Connect protocol alike."
   ([guide, server reflection](https://github.com/connectrpc/connect-rust/blob/main/docs/guide.md),
   [README](https://github.com/connectrpc/connect-rust/blob/main/README.md))
   That is a capable substitute, but it is live-server discovery, not a file a
   reviewer reads in a pull request. **Support:** direct evidence.
   **Confidence:** high.
2. **Comments survive into Rust doc comments, so rustdoc is not the loss.**
   `connectrpc-build` carries `.proto` comments into generated doc comments "as
   long as the descriptors carry source info", and `prost-build` has a
   `cleanup-markdown` feature for third-party files. So the "schema
   documentation quality" question is mostly about where the document lives, not
   whether comments exist. **Support:** direct evidence. **Confidence:** high.
3. **No REST semantics.** No resource-oriented paths, no verb choice, no path or
   query parameter binding, no media-type negotiation beyond proto and JSON.
   Every RPC is `POST /pkg.Service/Method` with one message body, plus the
   `NO_SIDE_EFFECTS` GET form. A REST surface for third parties becomes a
   gateway (transcoding, Envoy, grpc-gateway) or a second handwritten HTTP
   surface. **Support:** direct evidence from the Connect spec and the local
   baseline. **Confidence:** high.
4. **The error model moves from HTTP status to RPC status.** Connect maps 16
   canonical codes onto HTTP statuses, and the mapping is exact: `canceled` 499,
   `unknown` 500, `invalid_argument` 400, `deadline_exceeded` 504, `not_found`
   404, `already_exists` 409, `permission_denied` 403, `resource_exhausted` 429,
   `failed_precondition` 400, `aborted` 409, `out_of_range` 400, `unimplemented`
   501, `internal` 500, `unavailable` 503, `data_loss` 500, `unauthenticated`
   401. It also defines the inverse inference table for responses without an
   explicit code.
   ([Connect Protocol Reference](https://connectrpc.com/docs/protocol/))
   Two consequences: for streaming RPCs the HTTP status is always 200 and the
   error arrives inside the trailing `EndStreamResponse`, so a generic HTTP
   monitor sees success; and for plain gRPC (tonic) status arrives in HTTP/2
   trailers, which dumb proxies drop.
   ([gRPC over HTTP2](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md))
   **Support:** direct evidence. **Confidence:** high.
   **Researcher inference:** this is a real loss for generic HTTP observability
   and for retry policies that key on status codes, and a real gain in that the
   codes are richer and structured details (`ErrorDetail` carrying typed protobuf
   messages) are first-class.
5. **Non-Rust client friction.** Protobuf clients exist across Go, Java, Python,
   TypeScript, Swift, Kotlin and C++, and Connect-ES covers browser and Node,
   but each consumer needs the `.proto` plus a generator (or reflection plus a
   dynamic client). OpenAPI's ecosystem is broader for a first call: any-language
   generated clients, Postman collections, mock servers, gateway configuration.
   **Support:** direct evidence for the Connect-ES and language coverage; the
   comparative claim is researcher inference. **Confidence:** medium-high.
6. **What the OpenAPI spike got for free that Protobuf does not.** The generated
   server gave parameter structs, validators, header handling and a client method
   per operation (1,488 generated lines for two endpoints). Protobuf gives a
   client method per RPC and typed messages, but no parameter validation, no
   generated route table to read, and no JSON Schema for third-party response
   validation. **Support:** direct evidence from the local baseline.
   **Confidence:** high.
7. **Code-generation hygiene cost does not vanish, it moves.** The spike measured
   four module-level allows plus a rustls feature change plus two crates that
   fail `cargo fmt --check`. For the Protobuf stacks, connectrpc documents exactly
   one crate-root allow, and I found no published measurement of `cargo fmt
   --check` or `cargo clippy -D warnings` cleanliness for `buf generate` output.
   **Support:** direct evidence for the allow; missing evidence for the rest.
   **Confidence:** low for the comparison, high that it is unmeasured.

## Comparison table

| Stack | Versions | Generation toolchain | Streaming | Custom transport | DTO sharing | Drift checking | Licence |
| --- | --- | --- | --- | --- | --- | --- | --- |
| OpenAPI baseline (measured) | `openapi-generator-cli` 7.10.0 | JVM plus pinned jar, 1.6 to 1.8 s per run | none generated; SSE handwritten at 125 lines | `reqwest::ClientBuilder::unix_socket` (client), `hyper::server::conn::http1::serve_connection` (server, 29 lines) | two divergent sets, 51-line conversion layer | `regenerate.sh --check` diff plus four normalizations | Apache-2.0 |
| `prost` + `tonic` | prost 0.14.4, prost-build 0.14.4, tonic 0.14.6, tonic-build 0.14.6, tonic-prost-build 0.14.6 | `protoc` on PATH or `PROTOC`, invoked by `tonic-prost-build` in `build.rs`; `protoc` can be compiled from source via `protobuf-src` | all four RPC shapes over HTTP/2; ordering guaranteed per RPC | server `serve_with_incoming(UnixListenerStream)` plus `UdsConnectInfo`; client `connect_with_connector`; no built-in named pipe | one message set | `build.rs` into `OUT_DIR` (nothing committed), or `buf generate` plus `git diff --exit-code` | MIT (tonic family), Apache-2.0 (prost family) |
| `prost` + `tonic` + `protox` | plus protox 0.10.0 | no `protoc` binary; `protox` 0.10.0 compiles to a `FileDescriptorSet` in Rust, MSRV 1.85 | as above | as above | one message set | as above | Apache-2.0 OR MIT |
| ConnectRPC Rust, `buf generate` | connectrpc 0.9.1, buffa 0.9.2, buf 1.73.0, plugins `protoc-gen-buffa` / `protoc-gen-buffa-packaging` / `protoc-gen-connect-rust` (or BSR remote `buf.build/connectrpc/rust`) | buf plus two or three plugin binaries; no `protoc`, no JVM | all four over HTTP/2, first three also over HTTP/1.1; ordering structural per stream; no trailers | client `lazy_unix` / `connect_unix` / `lazy_with_connector`; server `Server::serve_connection` in a custom accept loop; no built-in named pipe | one message set (owned plus view types) | `buf generate` plus `git diff --exit-code`, `clean: true`, plus `buf breaking` | Apache-2.0 |
| ConnectRPC Rust, `build.rs` | plus connectrpc-build 0.9.0 | `protoc` (or `Config::use_buf`, or a precompiled descriptor set); no JVM | as above | as above | one message set in one file per proto via `include_generated!()` | none committed; regenerated each build into `OUT_DIR` | Apache-2.0 |

## What a Protobuf route would cost

Stated plainly, without inflating either side:

- **A readable HTTP API surface is lost.** Endpoints become
  `POST /pkg.Service/Method`. There is no document to read in a pull request,
  no path and query binding, and no generated route table. The replacement is
  reflection plus `buf curl`, `grpcurl`, Postman, `grpcui` or Buf Studio against
  a running server.
- **The error model changes shape.** Status moves from HTTP status to RPC code,
  streaming failures arrive inside an HTTP 200 body, and generic HTTP tooling
  stops being able to classify failures without knowing the protocol.
- **Non-Rust consumers gain friction and lose convenience.** They need the
  `.proto` and a generator, or reflection and a dynamic client. There is no
  swagger-codegen one-liner and no OpenAPI document to hand to a gateway.
- **A new toolchain enters the build.** `buf` 1.73.0 as a pinned Go binary plus
  two or three plugin binaries for the recommended checked-in path, or `protoc`
  plus plugins for the `build.rs` path. This replaces the JVM, so generation cost
  goes down, but the pinned-version surface goes up: buf, each plugin, buffa and
  connectrpc all move together, and buffa 0.9 is a hard floor for connectrpc 0.9.
- **A young dependency enters the tree.** `buffa` was first published
  2026-03-14 and 8 of its 15 releases were breaking. `connectrpc` is pre-1.0 by
  declaration, and its own changelog documents generated-code breaks between 0.8
  and 0.9. Budget regeneration churn, and note the crate-name discontinuity
  described in Q1.
- **Generated-code hygiene is unmeasured.** Expect at least one crate-root allow
  (`refining_impl_trait_internal`, `refining_impl_trait_reachable`) and treat
  the `fmt` and `clippy` posture as an open spike question, not as a known
  improvement over the OpenAPI spike's four allows.
- **Line-cost parity is unmeasured.** Do not carry the 326 / 229 / 276 numbers
  across. They measure generator output for an OpenAPI document, not for
  `.proto`, and the SSE 125 is the only number with a clear directional answer
  (the server writer and client parser disappear, the replay and queue logic
  does not).
- **Named-pipe support is handwritten on both stacks,** so the platform that
  matters most for a local daemon transport is the least verified part of the
  plan.

## Recommendation

**Adopt Protobuf as the contract, with ConnectRPC Rust (`connectrpc` 0.9.1,
`buffa` 0.9.2, `buf` 1.73.0, generated with `buf generate` and checked in) rather
than plain gRPC through `tonic` 0.14.6. Confidence: medium.**

The parts I am confident about:

- A Protobuf route removes the two costs the OpenAPI spike actually measured.
  The SSE path stops being handwritten protocol plumbing, and there is one
  message-type set with no conversion layer. **Confidence: high.**
- No JVM is needed. `buf` or a plugin binary replaces the JVM and jar.
  **Confidence: high.**
- Weaker but real: Connect over gRPC-only, because Connect keeps HTTP/1.1 for
  the three RPC shapes this effort needs, uses no HTTP trailers, serves gRPC and
  gRPC-Web from the same listener, and offers a JSON codec. That combination is
  what makes a Unix-socket transport reuse the hyper HTTP/1.1 pattern the effort
  has already proven, and what makes a browser client possible without a
  gateway. **Confidence: medium-high.**

The parts that make this medium rather than high:

- `connectrpc` is pre-1.0 and younger than `tonic`, and it depends on the also
  young `buffa`, with a hard version coupling between the two.
- There is no measured evidence for Connect over a Unix socket, none for either
  stack over a named pipe, and none for the generated-code hygiene or the line
  cost of the Connect path in this repository.
- If the transport were fixed to TCP with TLS and no browser client were
  planned, `tonic` plus `prost` would be the lower-risk choice, and `protox`
  would still remove `protoc`.

**Researcher inference:** the decision is currently better supported than the
OpenAPI baseline for the two costs that motivated it, and worse supported for
transport and streaming in this specific environment. The next step is not more
documentation reading, it is the smallest artifact below.

## Unresolved questions, and the smallest artifact for each

1. **Does a server-streaming Connect RPC work end to end over a Unix domain
   socket here, and what does the glue cost?** Smallest artifact: one crate with
   a single server-streaming RPC, served over a `tokio::net::UnixListener` by an
   accept loop calling `Server::serve_connection`, driven by the generated client
   over `Http2Connection::lazy_with_connector`, printing `PASS`. That is the
   exact analogue of the spike's `crates/wire/src/bin/spike.rs`.
2. **What is the handwritten line count for the same echo plus events contract
   on the Connect path?** Smallest artifact: the spike's own contract ported to
   `.proto` and Connect, counted with the spike's rule (non-blank, non-comment
   lines), producing a number comparable to 326 / 229 / 276 and to the SSE 125.
3. **Does `buf generate` output pass this repository's gates?** Smallest
   artifact: the generated tree plus `cargo fmt --check` and
   `cargo clippy --all-targets -- -D warnings`, reporting the allow attributes
   needed.
4. **Does the same client reach the server over a Windows named pipe?** Smallest
   artifact: the Unix-socket spike compiled for `x86_64-pc-windows-msvc` with a
   `#[cfg(windows)]` named-pipe connector and accept loop.
5. **Does a plaintext or TLS build link statically?** Smallest artifact:
   `cargo build --target x86_64-unknown-linux-musl` for the runtime plus the
   spike binary, with `ldd` or `readelf -d` showing no dynamic dependencies. This
   answers the `aws-lc-rs`-only constraint described in Q6.
6. **What does the transport actually do to a stalled subscriber?** Smallest
   artifact: a test that stops reading mid-stream and asserts the observable
   behaviour (backpressure, reset, or an error), so the slow-consumer policy in
   Q7 point 4 can be written against reality rather than assumption.

## Contradictions and discrepancies

1. **`connectrpc-build` and `protoc`.** The user guide says the build-time path
   needs "no extra binaries needed" and the README says "No plugin binaries
   required at build time", while the crate's own documentation says it "shells
   out to `protoc` (or `buf`, or reads a precompiled `FileDescriptorSet`)".
   ([guide](https://github.com/connectrpc/connect-rust/blob/main/docs/guide.md)
   versus [docs.rs/connectrpc-build](https://docs.rs/connectrpc-build/latest/connectrpc_build/)).
   Resolved in favour of the crate docs, which are the more specific statement;
   the guide's sentence is best read as "no `protoc-gen-*` plugin binaries".
2. **Repository naming for the Connect stack.** The crates.io `repository` field
   and the connectrpc README point at `github.com/connectrpc/connect-rust`, while
   the same changelog entry says "First release from the
   [anthropics/connect-rust] repository" and the buffa dependency is hosted at
   `github.com/anthropics/buffa`. Both URLs serve the project today. Recorded,
   not resolved; it affects a supply-chain review, not the technical decision.
3. **Baseline path.** The task names `spikes/openapi-codegen/findings.md`; that
   path does not exist in this checkout. The archived findings under
   `docs/tasks/metis-foundations/tasks/openapi-codegen-spike/findings.md` carry
   the same numbers, and the numbers agree, so nothing in the comparison depends
   on the missing copy.

## Missing evidence

- No measured line counts, generation times, or build times for any Protobuf
  stack. Every "lines saved" statement above is inference from the spike's
  structure, not measurement.
- No evidence that connectrpc server-side streaming works over a Unix socket in
  a real build. The documentation that `Server::serve_connection` exists and is
  the intended hook for "listen on a Unix socket" is documentation, not
  verification.
- No Windows verification for named pipes on either stack. The effort's only
  named-pipe evidence remains the spike's compile-only check on Linux.
- No static-linking result for `aws-lc-rs` or `ring` in this devenv.
- No `cargo fmt` or `cargo clippy` result for `buf generate` output or for
  `tonic-prost-build` output.
- The `buf` CLI licence was not verified in this run; only its version and date
  were.
- `source_check` on the `connectrpc-build`/`protoc` claim returned status
  `unclear` (`confidence 0.30`) with no automated support assessment available
  from the provider. The claim therefore rests on manual reading of the crate's
  own docs.rs page and `lib.rs` source and of `buffa-build`'s docs, which agree
  with each other. Disclosed as a validation limitation, not as a verified
  finding.

## Sources

Kept:

- [Connect Protocol Reference](https://connectrpc.com/docs/protocol/), the
  specification: HTTP/1.1 versus HTTP/2 per RPC shape, no trailers, the code to
  HTTP status table, envelope framing, EndStreamResponse.
- [connectrpc/connect-rust README](https://github.com/connectrpc/connect-rust/blob/main/README.md),
  the crate's own front page: versions, MSRV, status, `buf.gen.yaml` layout,
  plugin installation.
- [connectrpc/connect-rust user guide](https://github.com/connectrpc/connect-rust/blob/main/docs/guide.md),
  the API surface: streaming handler and client shapes, custom accept loops
  including Unix sockets, TLS, deadline policy, request limits, error model,
  example tour including the wasm fetch client.
- [connectrpc CHANGELOG](https://github.com/connectrpc/connect-rust/blob/main/CHANGELOG.md),
  dated releases, the protocol support table, the crate-name discontinuity, the
  0.9.1 streaming-body fix.
- [connectrpc workspace Cargo.toml](https://github.com/connectrpc/connect-rust/blob/main/Cargo.toml),
  the buffa 0.9 hard floor and the rustls provider constraint.
- [docs.rs/connectrpc Http2Connection](https://docs.rs/connectrpc/latest/connectrpc/client/struct.Http2Connection.html),
  `lazy_unix` / `connect_unix` / `lazy_with_connector` and the explicit
  comparison with tonic's connector API.
- [docs.rs/connectrpc Server](https://docs.rs/connectrpc/latest/connectrpc/server/struct.Server.html),
  TCP-only built-in bind.
- [docs.rs/connectrpc-build](https://docs.rs/connectrpc-build/latest/connectrpc_build/),
  the authoritative `protoc` / `buf` / descriptor-set requirement.
- [buffa README](https://github.com/anthropics/buffa/blob/main/README.md),
  pure-Rust editions-first implementation, owned plus view types, `buffa-build`
  requiring `protoc`, the BSR plugin.
- [docs.rs/buffa-build](https://docs.rs/buffa-build/latest/buffa_build/),
  confirming the `protoc` requirement and the `use_buf` / descriptor-set
  alternatives.
- [tonic crate docs](https://docs.rs/tonic/latest/tonic/), the feature list
  including `tls-ring` and `tls-aws-lc`, and the note that tonic-build is reached
  through `tonic-prost-build`.
- [docs.rs/tonic Endpoint](https://docs.rs/tonic/latest/tonic/transport/struct.Endpoint.html)
  and [docs.rs/tonic Server](https://docs.rs/tonic/latest/tonic/transport/struct.Server.html),
  `connect_with_connector` and `serve_with_incoming`.
- [tonic uds example](https://github.com/hyperium/tonic/blob/master/examples/src/uds/server.rs),
  the working Unix-socket server pattern.
- [docs.rs/prost-build](https://docs.rs/prost-build/latest/prost_build/), the
  `protoc` sourcing section.
- [protox repository](https://github.com/andrewhickman/protox) and
  [lib.rs/crates/protox](https://lib.rs/crates/protox), the pure-Rust compiler
  and its prost/tonic usage.
- [buf quickstart](https://buf.build/docs/generate/tutorial/) and
  [migrate from protoc](https://buf.build/docs/migration-guides/migrate-from-protoc/),
  `protoc` is not needed, and `clean: true` semantics.
- [buf breaking](https://buf.build/docs/breaking/), the compatibility gate.
- [gRPC over HTTP2](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md)
  and [gRPC core concepts](https://grpc.io/docs/what-is-grpc/core-concepts/),
  HTTP/2 framing, trailers, per-RPC ordering.
- [Protobuf Editions overview](https://protobuf.dev/editions/overview/),
  edition numbering.
- [connect-es](https://github.com/connectrpc/connect-es), the browser and Node
  client, and `@connectrpc/connect-web` over fetch.
- crates.io API for
  [prost](https://crates.io/api/v1/crates/prost),
  [prost-build](https://crates.io/api/v1/crates/prost-build),
  [tonic](https://crates.io/api/v1/crates/tonic),
  [tonic-build](https://crates.io/api/v1/crates/tonic-build),
  [tonic-prost-build](https://crates.io/api/v1/crates/tonic-prost-build),
  [connectrpc](https://crates.io/api/v1/crates/connectrpc),
  [connectrpc-build](https://crates.io/api/v1/crates/connectrpc-build),
  [buffa](https://crates.io/api/v1/crates/buffa), versions, dates, licences and
  MSRVs.
- Local: `docs/tasks/metis-foundations/tasks/openapi-codegen-spike/findings.md`,
  the measured baseline.

Rejected or deprioritized:

- Community named-pipe material ([tonic issue 1518](https://github.com/hyperium/tonic/issues/1518),
  [catalinsh/tonic-named-pipe-example](https://github.com/catalinsh/tonic-named-pipe-example),
  [cf-modkit-transport-grpc](https://docs.rs/crate/cf-modkit-transport-grpc/latest/source/src/windows_named_pipe.rs)):
  used only as evidence that no built-in support exists, not for any capability
  claim.
- DeepWiki pages on the Connect stack: an AI-generated summary of the primary
  sources already read directly, so it was not used for any finding.
- `buf` CLI licence: not verified, so the table records it as not checked rather
  than guessing.

## Next steps

1. Build the single-artifact transport spike from unresolved question 1. It is
   the only thing that converts the transport and streaming answers from
   documentation into evidence.
2. In the same artifact, measure the generated-code hygiene gates and the
   handwritten line count, so the Connect path gets a number comparable to
   326 / 229 / 276 and to the SSE 125.
3. Only if the transport result is favourable, extend to Windows named pipes and
   to a static musl link. Both are cheap once the Unix-socket spike exists.
