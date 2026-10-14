---
type: findings
title: ConnectRPC and Protobuf spike, server streaming over a Unix socket, fmt, clippy, Windows, and static linking
status: stable
---

# ConnectRPC spike

Prototype for `docs/tasks/metis-foundations/tasks/connectrpc-spike/task.md`.
It is the spike the `protobuf-connect-research` findings asked for. All source
lives under `spikes/connectrpc/` with its own `[workspace]` table, its own
lockfile and its own target directory. It is not a member of the root workspace,
and nothing outside the spike directory and this file was changed.

## Verdict

Conclusive. A server-streaming Connect RPC works end to end over a Unix domain
socket, driven by the generated client over `lazy_with_connector`, and reaches
`PASS`. The same generated service also compiles for a Windows named pipe and
for `x86_64-unknown-linux-musl`, where the binary links statically and runs. The
recommended stack (connectrpc 0.9.1, buffa 0.9.2, buf 1.73.0) is usable here
with no JVM and no `protoc`, using BSR remote plugins only. The measured
handwritten surface is 255 non-blank, non-comment Rust lines for the Unix plus
Windows transport and the driver, and 213 lines if the compile-only Windows path
is excluded. Raw generated output does not pass the repository fmt or clippy
gates without normalizations; the specific attributes and reformats are named
below.

One research claim did not survive contact: there is no `Server::serve_connection`
in connectrpc 0.9.1. The documented and working route is `ConnectRpcService::new`
plus a hyper accept loop, which the spike uses.

## The spike

`spikes/connectrpc/proto/metis/spike/v1/spike.proto` carries the same contract
shape as the OpenAPI spike: one ordinary RPC and one server-streaming RPC with a
resumable cursor.

```protobuf
service SpikeService {
  rpc Echo(EchoRequest) returns (EchoResponse);
  rpc StreamEvents(StreamEventsRequest) returns (stream Event);
}
```

`EchoRequest.message` and `EchoResponse.message` mirror the OpenAPI schema,
`EchoResponse.received_at` mirrors its `receivedAt` date-time as an RFC 3339
string, and `Event` carries `id`, an `EventKind` enum and `payload`. The stream
request's `last_event_id` mirrors the OpenAPI `Last-Event-ID` header. Connect has
no comment frame, so heartbeats are ordinary messages with
`kind = EVENT_KIND_HEARTBEAT`.

The spike workspace holds:

- `generated/buffa/metis.spike.v1.rs`: message types plus zero-copy views and
  serde impls (`protoc-gen-buffa`, 1655 code lines).
- `generated/connect/metis.spike.v1.rs`: the `SpikeService` trait, register
  helper and generated client (`protoc-gen-connect-rust`, 437 code lines).
- `crates/wire/src/lib.rs`: the hand-written module tree that mounts the two
  generated trees, the Unix-socket accept loop, the `lazy_with_connector` client
  transport, and the compile-only Windows named-pipe server and client.
- `crates/wire/src/bin/spike.rs`: the `SpikeService` implementation and the
  driver that serves over a Unix socket and prints `[result] PASS`.
- `crates/wire/src/bin/stall.rs`: a separate slow-subscriber diagnostic for Q6.
- `scripts/regenerate.sh`: BSR remote generation plus a drift check.

Observed output of the main driver:

```
[echo] service=metis.spike.v1.SpikeService message="hello over uds" received_at=1791849619371
[stream] messages=["1", "2", "3"] heartbeats=3
[stream] resume_after=2 first_message=Some("3")
[result] PASS
```

## Toolchain, buf and the plugins

Every version used, and how each was obtained:

| Component | Version | Obtained via |
| --- | --- | --- |
| `connectrpc` runtime | 0.9.1 | crates.io, pinned `=0.9.1` |
| `buffa` runtime | 0.9.2 | crates.io, pinned `=0.9.2` |
| `protoc-gen-buffa` | 0.9.2, BSR remote | `remote: buf.build/anthropics/buffa:v0.9.2` |
| `protoc-gen-connect-rust` | 0.9.0, BSR remote | `remote: buf.build/connectrpc/rust:v0.9.0` |
| `buf` CLI | 1.73.0 | `nix shell nixpkgs#buf --command buf ...` |
| Rust | 1.101.0-nightly (080cc2714 2026-10-11) | devenv nightly, and rustup `nightly-2026-10-12` |
| Cargo | 1.101.0-nightly (29c5daa1c 2026-10-07) | devenv nightly |

The remote plugin route worked. `buf generate` ran both plugins from the Buf
Schema Registry; no local plugin binary was installed and no `protoc` was used.
The BSR connect plugin's latest published version is `v0.9.0`, so the plugin
version and the runtime version differ (plugin 0.9.0, runtime 0.9.1). That is
expected: the codegen crate is published separately from the runtime, and
connectrpc's own README example pins the same `v0.9.0` remote tag.

One local-only plugin exists, `protoc-gen-buffa-packaging`, which assembles the
`mod.rs` module trees. The spike avoids it by running both codegen plugins with
`file_per_package`, which emits one `<dotted.package>.rs` per plugin, and writing
the small `pub mod` tree in `crates/wire/src/lib.rs` by hand. That keeps the whole
generation path on the BSR and keeps the handwritten cost visible.

`buf.gen.yaml`:

```yaml
version: v2
plugins:
  - remote: buf.build/anthropics/buffa:v0.9.2
    out: generated/buffa
    opt: [views=true, json=true, file_per_package=true]
  - remote: buf.build/connectrpc/rust:v0.9.0
    out: generated/connect
    opt: [buffa_module=crate::proto, file_per_package]
```

Note the option syntax differs: the buffa plugin accepts `file_per_package=true`,
the connect plugin accepts the bare `file_per_package`. The connect plugin also
rejects `file_per_package=true` as an unknown option.

Exact commands:

```
cd spikes/connectrpc

# buf, unmodified from nixpkgs (version 1.73.0), no local plugins
nix shell nixpkgs#buf --command buf --version
nix shell nixpkgs#buf --command buf generate
nix shell nixpkgs#buf --command buf lint

# drift check: regenerate into a temp copy, diff, exit non-zero on staleness
bash scripts/regenerate.sh --check        # prints "drift check: no diff"

# build and run through devenv
devenv shell -- cargo build
devenv shell -- cargo run --bin spike
devenv shell -- cargo run --bin stall

# gates
devenv shell -- cargo fmt --all
devenv shell -- cargo fmt --all -- --check
devenv shell -- cargo clippy --workspace --all-targets -- -D warnings

# fmt on the generated files directly, since cargo fmt does not follow include!
devenv shell -- rustfmt --check generated/buffa/metis.spike.v1.rs
devenv shell -- rustfmt --check generated/connect/metis.spike.v1.rs

# Windows compile-only check: rustup toolchain matching the devenv nightly
nix shell nixpkgs#rustup --command rustup toolchain install nightly-2026-10-12 \
  --profile minimal --target x86_64-pc-windows-msvc
devenv shell -- bash -c 'export RUSTUP_TOOLCHAIN=nightly-2026-10-12; \
  export PATH="$HOME/.rustup/toolchains/nightly-2026-10-12-x86_64-unknown-linux-gnu/bin:$PATH"; \
  cargo check --target x86_64-pc-windows-msvc --all-targets'

# static musl build and inspection
nix shell nixpkgs#rustup --command rustup target add \
  --toolchain nightly-2026-10-12 x86_64-unknown-linux-musl
devenv shell -- bash -c 'export RUSTUP_TOOLCHAIN=nightly-2026-10-12; \
  export PATH="$HOME/.rustup/toolchains/nightly-2026-10-12-x86_64-unknown-linux-gnu/bin:$PATH"; \
  cargo build --target x86_64-unknown-linux-musl'
devenv shell -- bash -c 'file target/x86_64-unknown-linux-musl/debug/spike; \
  ldd target/x86_64-unknown-linux-musl/debug/spike; \
  readelf -d target/x86_64-unknown-linux-musl/debug/spike | grep NEEDED || true'
./target/x86_64-unknown-linux-musl/debug/spike
```

No JVM is needed on this path. The ephemeral JDK plus pinned
`openapi-generator-cli-7.10.0.jar` from the OpenAPI spike disappears entirely.

## Q1. Server streaming over a Unix socket, end to end

Result: works, `PASS`. An accept loop over `tokio::net::UnixListener` drives the
Connect service, the generated client connects over
`Http2Connection::lazy_with_connector`, both RPCs succeed, and the
server-streaming call delivers three message events plus three heartbeats in
order. A second call with cursor `2` resumes at message `3`, which is the
resumable-cursor semantics the effort needs.

One correction to the research. There is no `Server::serve_connection` in
connectrpc 0.9.1; grepping the published crate's `src/server.rs` shows only
private helpers. `Server::bind` and `Server::from_listener` accept TCP listeners
only, and `serve` takes a `SocketAddr`, so the built-in server cannot bind a Unix
socket. The documented escape hatch is `ConnectRpcService::new(router)` wrapped
in `hyper_util::service::TowerToHyperService` and handed to hyper's auto
connection builder, which auto-detects the HTTP/2 preface versus HTTP/1.1. That
is exactly what `crates/wire/src/lib.rs::serve_uds` does, in about 21 code lines,
which is comparable to the OpenAPI spike's 29-line `serve_uds`. Client and
server both speak h2c, so the client's HTTP/2-only `Http2Connection` is satisfied.

The client side uses the exact constructor named in the task,
`Http2Connection::lazy_with_connector`, with a `tower::service_fn` closure that
opens a `UnixStream` and returns `TokioIo<UnixStream>`. The resulting connection
is converted with `.shared(64)` because `ClientTransport` is implemented for
`SharedHttp2Connection`, not for `Http2Connection` directly.

## Q2. Handwritten line count

Counted with the OpenAPI spike's rule: non-blank lines that do not start with
`//` after trimming leading whitespace. Generated code is excluded.

| File | Handwritten code lines |
| --- | --- |
| `crates/wire/src/bin/spike.rs` | 155 |
| `crates/wire/src/lib.rs` | 100 |
| Core total (echo plus events contract) | 255 |
| `crates/wire/src/bin/stall.rs` (Q6 diagnostic, not contract glue) | 118 |

File-by-file breakdown of the 255:

| Section | File | Lines |
| --- | --- | --- |
| Module tree mounting both generated trees | `lib.rs` | 23 |
| Transport imports | `lib.rs` | 9 |
| `serve_uds` accept loop | `lib.rs` | 21 |
| `uds_transport` connector over `lazy_with_connector` | `lib.rs` | 12 |
| `local_authority` helper | `lib.rs` | 3 |
| `serve_named_pipe` and `named_pipe_transport` (Windows, compile-only) | `lib.rs` | 32 |
| Imports and cfg wiring | `spike.rs` | 16 |
| `SpikeService` impl plus event helpers | `spike.rs` | 53 |
| Unix `main` driver | `spike.rs` | 44 |
| `collect` stream drain | `spike.rs` | 22 |
| `wait_for_socket` | `spike.rs` | 10 |
| Windows `main` (compile-only) | `spike.rs` | 10 |

Comparison with the measured OpenAPI baseline:

| Path | Handwritten code lines |
| --- | --- |
| OpenAPI, both sides generated (A) | 326 |
| OpenAPI, client-only plus handwritten server (B) | 229 |
| OpenAPI, typify models, both HTTP layers handwritten (C) | 276 |
| OpenAPI SSE path, entirely handwritten | 125 |
| Connect, all handwritten Rust including the Windows path | 255 |
| Connect, Unix path only (drop the 42 Windows lines) | 213 |

The headline number is 255, which sits between the OpenAPI client-only path
(229) and the both-sides-generated path (326). Unlike the OpenAPI spike, there is
no separate shared crate; the 255 is the whole handwritten surface for this
contract. The Windows named pipe is compile-only and accounts for 42 of the 255
lines, so the exercised Unix path is 213 lines, below the client-only OpenAPI
path. If the Q6 diagnostic is included as effort code, the total is 373 lines;
it is reported separately because it is not part of the echo or events contract.

Two differences in what the count covers, stated so the comparison is honest.
Generated code is 2092 code lines here (buffa 1655, connect 437), versus 1488 for
OpenAPI alternative A; the Connect output bundles views, JSON serde impls and
registration that the OpenAPI output did not. The handwritten side on Connect
also has no DTO-conversion file, because both halves share one message set,
which is the drift problem the OpenAPI spike paid 51 lines for.

## Q3. fmt and clippy on generated output

`cargo fmt --all -- --check` exits 0 for the workspace. That result is
misleading on its own: the generated trees are mounted with `include!`, and
rustfmt does not follow `include!`, so `cargo fmt` never sees them. Run rustfmt
directly on the generated files to get the real answer.

Under the repository `rustfmt.toml` (edition 2024, `max_width = 100`), rustfmt
reports 95 diff hunks against raw output: 74 in the buffa file and 21 in the
connect file. Under a narrower default-shaped config (`max_width = 80`,
edition 2021) it reports 131 (99 plus 32). So raw `buf generate` output does not
pass the repository fmt gate. The diffs are ordinary reformats (collapsing
multi-line signatures that fit, reflowing a `concat!`), not semantic changes, so
a `cargo fmt` or `rustfmt` post-generation step on the generated directories, or
an exclusion for them, is the practical fix. There is no allow attribute that
fixes a fmt failure.

`cargo clippy --workspace --all-targets -- -D warnings` fails on raw output with
three errors, and the spike makes it pass by adding two module-scope allows at
the `include!` mount points:

- `generated/buffa/metis.spike.v1.rs`: a hand-written `impl Default for EventKind`
  that clippy wants derived (`clippy::derivable_impls`). Allow it, or add
  `#[derive(Default)]` plus `#[default]` in a normalization step.
- `generated/connect/metis.spike.v1.rs`: two single-arm `match method` fallbacks
  in the unimplemented dispatcher (`clippy::match_single_binding`). Allow it.

A third allow is needed on the consumer side, not on generated code: the
generated service trait uses an explicit RPITIT signature, so an `async fn`
implementation refines it and the compiler emits `refining_impl_trait` under
`#[warn(refining_impl_trait)]`. With `-D warnings` this is an error, so every
`impl SpikeService` needs `#[allow(refining_impl_trait)]` (or an exact
hand-written signature). That is real friction on the server-implementation
side and is worth knowing before adopting.

With the two generated-code allows and the one consumer allow, clippy exits 0.
There is no third-party future-incompatibility warning on this dependency set,
unlike the OpenAPI spike's `proc-macro-error2` note. The generated code also
passes `buf lint` (STANDARD) once `RPC_RESPONSE_STANDARD_NAME` is excepted,
because the streaming response message is deliberately named `Event` to mirror
the OpenAPI schema rather than `StreamEventsResponse`.

## Q4. Windows named pipe, compile-only

Result: compiles for `x86_64-pc-windows-msvc`. The spike provides
`serve_named_pipe` (a `NamedPipeServer` accept loop over the same
`ConnectRpcService` and hyper auto builder) and `named_pipe_transport` (a
`lazy_with_connector` that opens a `NamedPipeClient` and returns
`TokioIo<NamedPipeClient>`). The crate's Windows `main` wires the same
`SpikeService` into both, so
`cargo check --target x86_64-pc-windows-msvc --all-targets` type checks both
ends of the pipe. It exits 0.

This is a check, not a link and not a run. There is no Windows host here, so,
exactly as the OpenAPI spike recorded for `reqwest::ClientBuilder::windows_named_pipe`,
the runtime validity of the named-pipe path is unverified. The compile-only
caveat carries over unchanged. To build it, the spike used rustup's
`nightly-2026-10-12` toolchain (the same rustc commit as the devenv nightly) with
the `x86_64-pc-windows-msvc` target added, run inside the devenv shell so host
build scripts have `cc`. No MSVC linker is needed for `cargo check`.

Note that tokio's named-pipe types are behind `#[cfg(windows)]`, so this code is
inert on the Linux verification path and does not affect the Unix results.

## Q5. Static linking for musl

Result: yes, the plaintext build links statically. With
`x86_64-unknown-linux-musl` added to the same rustup toolchain,
`cargo build --target x86_64-unknown-linux-musl` succeeds with no musl-gcc and
no `openssl-sys`. Inspection of `target/x86_64-unknown-linux-musl/debug/spike`:

```
file:   ELF 64-bit LSB pie executable, x86-64, static-pie linked, with debug_info, not stripped
ldd:    statically linked
readelf -d: no NEEDED entries
```

The static binary also runs and prints `[result] PASS`, so this is not just a
link-time artifact. The build is clean because the spike turns TLS and
compression off: `connectrpc` is used with `default-features = false, features =
["client", "server", "json"]`, which keeps `zstd` and `gzip` out. `rustls` and
its `aws-lc-rs` provider, the one dependency the research flagged for static
linking, are not pulled at all on the plaintext path. If TLS is later enabled on
the Connect stack, `aws-lc-rs` compiles C and assembly and a fully static musl
build would need re-verification. The spike did not add a release profile, so the
measured binary is a 62 MB debug build; static linking itself is what Q5 asked
about and it holds.

## Q6. What the transport does to a stalled subscriber

Result: backpressure, then an observable disconnect when the client drops the
stream, measured with `cargo run --bin stall`. The handler produces an unbounded
stream of 16 KiB events into a bounded channel and counts production; the client
reads one message, stops polling for 2.5 seconds, then drops the stream.

Three runs produced the same shape:

```
[stall] first_message_received=true
[stall] produced: after_first=23 after_0.5s_stall=138 after_2.5s_stall=138 after_drop_1s=138
[stall] server_sender_saw_disconnect=true
[result] PASS
```

Reading: a subscriber that stops reading does not cause unbounded buffering. The
producer plateaus at 138 events (about 2.2 MiB in flight across the channel and
the HTTP/2 flow-control window) within half a second and stays there. The
transport applies backpressure; it does not grow memory without bound and it does
not kill the stream on its own. When the client drops the stream, the server's
response stream is dropped, the handler's send into the channel fails, and the
producer records the disconnect and stops. The disconnect is a plain drop of the
response stream: the handler gets no error and no callback, so a slow-consumer
policy that needs to resync must detect it from a failed send or a dropped
receiver and must build its own resync marker. Connect does not deliver a
terminal error to a server handler when the client goes away.

This is the effort's `slow-consumer policy currently assumes a disconnect with a
resync requirement` question answered at the transport level. The policy still
has to define the resync contract itself, because the transport only guarantees
the disconnect signal.

## fmt and clippy posture, summary

| Gate | Result | Normalization needed |
| --- | --- | --- |
| `cargo fmt --all -- --check` | passes | none, but generated code is invisible to it |
| `rustfmt --check` on generated files (repo config) | fails, 95 hunks | reformat generated dirs, or exclude them |
| `cargo clippy --workspace --all-targets -- -D warnings` | passes | 2 generated-code allows plus 1 consumer allow on the trait impl |
| `buf lint` (STANDARD) | passes | except `RPC_RESPONSE_STANDARD_NAME` |

The generated-code allows are `#![allow(clippy::derivable_impls)]` on the buffa
tree and `#![allow(clippy::match_single_binding)]` on the connect tree, both at
the `include!` mount point in `crates/wire/src/lib.rs`. The consumer allow is
`#[allow(refining_impl_trait)]` on the `impl SpikeService` block.

## Remaining uncertainty

- Windows named pipes are compile-only. Their behavior on a real Windows host,
  and the exact pipe name and security model, are unverified here.
- The static musl result is for a plaintext, no-TLS build. Enabling `client-tls`
  or `server-tls` pulls `rustls` with `aws-lc-rs`, and a static musl build then
  needs a fresh check that the spike did not run.
- The BSR remote plugin version for connect is 0.9.0 while the runtime is 0.9.1.
  The hard coupling the research documents (0.9 codegen against 0.9 runtime) is
  respected, and the spike built and ran, but a future 0.9.1 codegen plugin is not
  yet on the BSR, so pinning the plugin and the runtime in lockstep is a process
  thing to set up.
- Q6 sampled one client, a bounded channel and default HTTP/2 windows. The
  plateau size (138 events) is a property of those buffers, not a protocol
  constant; the qualitative result (bounded backpressure plus disconnect on drop)
  is what generalizes.
- The line count covers this one small contract. It does not include the `.proto`
  file, the Cargo manifests, or the shell script, matching the OpenAPI spike's
  Rust-only rule.

## Conclusiveness

Conclusive. Every question has a working artifact and a measured answer. The
Connect path works end to end over a Unix socket, compiles for Windows, links
statically for musl and runs, and its handwritten cost is measured with the same
rule as the OpenAPI baseline. The only research correction is that
`Server::serve_connection` does not exist in 0.9.1, replaced by the documented
`ConnectRpcService` plus hyper accept loop. The spike is kept as reference under
`spikes/connectrpc/`. The path choice this unblocks is the user's to make.
