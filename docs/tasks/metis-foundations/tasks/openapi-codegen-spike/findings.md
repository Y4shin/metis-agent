---
type: findings
title: OpenAPI codegen spike, spec-first generation versus handwritten HTTP layers
status: stable
---

# OpenAPI codegen spike

Prototype for `docs/tasks/metis-foundations/tasks/openapi-codegen-spike/task.md`.
All source lives under `spikes/openapi-codegen/` with its own `[workspace]` table,
its own lockfile and its own target directory. It is not a member of the root
workspace and nothing outside the spike directory and this file was changed.

## Verdict

Conclusive. The `rust-axum` server generator emits a crate that compiles and whose
router can be served over a Unix domain socket through hyper. The `rust` client
generator emits a client that accepts a custom transport with no wrapper. Both
halves of the pipeline were built, run, linted and formatted. The friction is
concentrated in three places: the generator cannot stream `text/event-stream`, the
raw output does not pass `cargo fmt --check` or `cargo clippy -D warnings` without
help, and the raw client does not build in this devenv without a TLS feature
change.

## The spike

`spikes/openapi-codegen/spec/openapi.yaml` (owned by the spike) has one ordinary
JSON request, `POST /v1/echo`, and one SSE endpoint, `GET /v1/events` with an
optional `Last-Event-ID` header. The spike workspace holds:

- `generated/server/`: `rust-axum` output (`metis-spike-server`).
- `generated/client/`: `rust` output, `library=reqwest` (`metis-spike-client`).
- `crates/shared/`: glue every alternative needs (UDS serving, custom transports,
  hand-written SSE client).
- `crates/wire/`: the primary alternative (both sides generated) plus `bin/spike`.
- `crates/alt-client-only/`: alternative B, generated client with a handwritten
  axum server.
- `crates/alt-typify/` and `crates/typify-gen/`: alternative C, typify models with
  both HTTP layers handwritten.
- `scripts/regenerate.sh`: pinned generation plus the drift check.

`crates/wire/src/bin/spike.rs` serves the generated router over a Unix socket and
drives it with the generated client; `alt-client-only` and `alt-typify` do the
same against their own servers. All three print `[result] PASS`.

## Environment and toolchain cost

Java is not in `devenv.nix`, and no `devenv.nix` change was needed or left in
place. The working route was an ephemeral JVM:

```
nix shell nixpkgs#jdk21 --command java -version
# openjdk version "21.0.12.1" 2026-08-18, OpenJDK 64-Bit Server VM
```

The generator is pinned to one explicit version, `7.10.0`, by downloading
`openapi-generator-cli-7.10.0.jar` from Maven Central and checking its sha256:

```
https://repo1.maven.org/maven2/org/openapitools/openapi-generator-cli/7.10.0/openapi-generator-cli-7.10.0.jar
sha256 615e014705af34861e789e0b2a11075d3c80db134f881e937265479a4a83996e
```

Measured generation cost on this machine (JVM and jar already cached):

| Step | Wall time |
| --- | --- |
| JVM start (`generate version`) | ~130 ms |
| `rust-axum` server generation | ~830 ms |
| `rust` client generation | ~810 ms |
| Full `regenerate.sh` (both generators + normalizations) | 1.6 to 1.8 s |

A pinned `npx @openapitools/openapi-generator-cli@<version>` was not used because
the npm wrapper versions do not line up with generator versions; the jar is the
explicit pin.

Commands used:

```
cd spikes/openapi-codegen
nix shell nixpkgs#jdk21 --command bash scripts/regenerate.sh          # regenerate
nix shell nixpkgs#jdk21 --command bash scripts/regenerate.sh --check  # drift check
devenv shell -- cargo build
devenv shell -- cargo run --bin spike
devenv shell -- cargo clippy --workspace --all-targets -- -D warnings
devenv shell -- cargo fmt --all -- --check
```

## Q1. Does the `rust-axum` crate compile, and can it be served over a Unix socket?

Yes on both counts. The generated crate is an axum 0.7 library exposing
`server::new(api_impl) -> Router` plus an `async_trait` `apis::default::Default`
trait. The crate compiles unmodified (warnings only). Serving over a Unix socket
is about 29 lines of glue shared by every alternative
(`crates/shared/src/lib.rs`): bind a `tokio::net::UnixListener`, wrap each
`UnixStream` in `hyper_util::rt::TokioIo`, and drive
`hyper::server::conn::http1::Builder::new().serve_connection(io, service)` where
`service` is `service_fn(|req| router.clone().oneshot(req.map(axum::body::Body::new)))`.
No generator change is needed to leave TCP.

## Q2. `text/event-stream`: what is generated and what stays handwritten?

Almost nothing is generated, and the generated part is unusable. For
`text/event-stream` the `rust-axum` generator emits:

```rust
pub enum StreamEventsResponse {
    Status200_AStreamOfEvents (String)
}
```

and turns it into a fully buffered body, `response.body(Body::from(body_content))`.
The trait method returns a `String`, so the generated router cannot stream at all.
The generated Rust client is worse: `stream_events` calls `resp.text().await` and
`serde_json::from_str::<models::Event>(...)`, which buffers the whole stream and
then tries to parse the concatenated frames as one JSON object.

Everything therefore stays handwritten:

- The stream itself, chunking and `Content-Type`: the SSE server is a separate
  `axum::response::sse::Sse` handler (50 code lines in
  `crates/wire/src/sse_server.rs`). Because axum panics when the same route is
  registered twice, the handler is mounted as a `middleware::from_fn` layer that
  short-circuits `GET /v1/events` before the generated buffered handler runs. The
  generated `stream_events` implementation is dead code.
- `Last-Event-ID` resume: read the header, parse the cursor, start the frame
  iterator after it (server), and send the header on reconnect (client).
- Heartbeat: emit `Event::default().comment(...)` (or a `KeepAlive`) and ignore
  comment frames on the client.
- Framing, `id:`/`event:`/`data:` parsing and JSON decode: 75 code lines in
  `crates/shared/src/sse_client.rs`.

The only reuse from the generator on this path is the DTO: the `data:` payload is
the same JSON the document describes, so the client can still deserialize it with
`serde`. In total 125 code lines of SSE stay handwritten (50 server, 75 client).

## Q3. Does the generated client accept a custom transport?

Yes, with no wrapper. The generated `Configuration` has a public
`pub client: reqwest::Client` field, so the transport is injected by building the
client differently:

```rust
let configuration = Configuration {
    client: reqwest::Client::builder().unix_socket(socket).build()?,
    ..Configuration::default()
};
```

Confirmed against reqwest 0.12.28 source: `ClientBuilder::unix_socket` is
`#[cfg(unix)]` and `ClientBuilder::windows_named_pipe` is
`#[cfg(target_os = "windows")]`, both taking a sealed provider implemented for
`String`, `&str`, `Path`, `PathBuf` and similar. On this Linux host `unix_socket`
was compiled and used (`crates/shared/src/transport.rs`);
`windows_named_pipe` is behind `#[cfg(windows)]` and was compiled on Linux, so it
is compile-only here and its validity on Windows is not verified.

## Q4. How do the two generated DTO sets interact?

They are two independent sets of types that only agree in the JSON they
serialize. In-process conversion is needed only when one binary holds both, which
is what the `spike` binary does for testing; over the wire no conversion is
needed because JSON is the contract. The structural gap is small but not
mechanical:

- The server generator renders `format: date-time` as
  `chrono::DateTime<chrono::Utc>`; the client generator has no chrono and renders
  it as `String`. Conversion needs `to_rfc3339` / `parse_from_rfc3339`.
- The server generator falls back to `String` for an inline enum; the client
  generator emits a real `Kind` enum. Conversion needs a two-way match.
- The orphan rule blocks `impl From<ServerX> for ClientX` in a third crate because
  neither trait nor self type is local, so conversions must be free functions or a
  local extension trait.

Measured boundary cost in `crates/wire/src/convert.rs`: 51 code lines for eight
conversion functions plus two enum mappers covering all four schema types.
Alternatives B and C avoid this entirely by having a single DTO set.

## Q5. Toolchain cost and the fmt/clippy results

Covered above for the JVM and the pin. Raw generator output is not clean:

`cargo fmt --check` on the generated crates fails. Under the repository
`rustfmt.toml` (edition 2024, max_width 100) the `rust-axum` crate produces 62
diff hunks and the client 28, and it also fails under plain `rustfmt --edition
2021`. The templates leave trailing whitespace, over-indented blocks and trailing
commas. The practical fix is a `cargo fmt` post-generation step or excluding
generated directories from the fmt gate, not an allow attribute.

`cargo clippy --all-targets -- -D warnings` fails on raw output too:

- `metis-spike-server`: 9 errors, seven `unreachable_code` in the generated
  `FromStr` impls in `models.rs` and two `mismatched_lifetime_syntaxes` in
  `types.rs`.
- `metis-spike-client`: 2 errors, `clippy::empty_docs` for a bare `///` and
  `clippy::derivable_impls` for a hand-written `impl Default for Kind`.

Two module-level allows per crate make both pass: server
`#![allow(unreachable_code)]` and `#![allow(mismatched_lifetime_syntaxes)]`,
client `#![allow(clippy::empty_docs)]` and
`#![allow(clippy::derivable_impls)]`. Generated crates do not opt into the
workspace lints by themselves; `scripts/regenerate.sh` appends
`[lints] workspace = true` so the workspace posture actually applies. With the
normalizations in place, `devenv shell -- cargo clippy --workspace --all-targets --
-D warnings` exits 0 (the only remaining warning is a third-party
`proc-macro-error2` future-incompatibility note). The handwritten crates pass
`cargo fmt --check`; the two generated crates still fail it, as recorded above.

One extra normalization is required for repeatable generation: the `rust-axum`
README embeds a build timestamp (`- Build date: ...`), so the script strips that
line to keep the output byte-stable.

There is also a TLS friction point. The raw generated client declares
`reqwest = { version = "^0.12", features = ["json", "multipart"] }`, which pulls
native-tls, and building it in this devenv fails in `openssl-sys` because OpenSSL
is absent. The spike switches reqwest to
`default-features = false, features = ["json", "multipart", "rustls-tls", "stream"]`.
A repository that adopts generated clients should decide between `pkgs.openssl` in
the devenv and rustls; the spike used rustls.

## Q6. The exact drift-check command

```
cd spikes/openapi-codegen
nix shell nixpkgs#jdk21 --command bash scripts/regenerate.sh --check
```

It regenerates into a temporary directory, applies the same four normalizations,
`diff -ru`s the result against `generated/` and exits non-zero on any difference
(verified by injecting a line: exit 1, then clean: `drift check: no diff`). A
git-based equivalent that does not need the temp copy is
`... regenerate.sh && git diff --exit-code -- spikes/openapi-codegen/generated`.

The four normalizations are deterministic text edits after generation: reqwest to
rustls, `[lints] workspace = true` in both generated manifests, the four allow
attributes, and stripping the README build date.

## Handwritten glue per alternative

Code lines count non-blank, non-`//` lines. Generated code is shown for contrast.
`shared` is the glue every alternative needs (`serve_uds` 29, `sse_client` 75,
`transport` 15 = 119 code lines); it is listed once.

| Alternative | Generated code lines | Alternative-specific handwritten | Plus shared | Total handwritten |
| --- | --- | --- | --- | --- |
| A: `openapi-generator` both sides | 1488 (server 1161, client 327) | 207 | 119 | 326 |
| B: client only, handwritten axum server | 327 (client) | 110 | 119 | 229 |
| C: typify models, both HTTP layers handwritten | 102 (typify models) | 157 (46 shim + 111) | 119 | 276 |

Breakdown of A's 207: `convert.rs` 51, `sse_server.rs` 50, `bin/spike.rs` 103
(ApiImpl plus driver), `lib.rs` 3. Breakdown of C's 157: `typify-gen` shim 46
(extract `components.schemas` and rewrite `$ref` to JSON Schema) plus
`alt-typify` 111 (server and client). The alternative-specific number for B and C
contains their own driver loop, which is roughly the same size in each, so the
comparison is fair in magnitude.

Notes on the comparison:

- All three alternatives were built and run; none is estimated. A second binary
  was not built for B's "conformance test"; parity is checked by driving the
  handwritten server with the generated client and asserting the round-trip, not
  by validating responses against a JSON Schema.
- B is the smallest in total code because it generates the client and handwrites
  only about a hundred lines of server. C is smaller than A only because the
  spec is tiny; its shim grows with the schema and typify produces no paths,
  parameters or HTTP code at all.
- A's 1488 generated lines buy parameter structs, validators, header handling and
  a client for every operation, none of which B or C get. The generated server's
  own weight (1161 lines for two endpoints) is mostly model and type boilerplate.

## Conclusiveness

Conclusive: every question is answered with a working artifact and measured
numbers. The spike is kept as reference under `spikes/openapi-codegen/`. The
decision this unblocks (spec-first versus code-first) is the parent's to take.
