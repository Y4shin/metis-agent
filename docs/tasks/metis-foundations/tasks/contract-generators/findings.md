---
type: findings
title: Contract generators, generated code placement, and the CI staleness check
status: stable
---

# Research: contract generators for `crates/api-contracts`

## Question investigated

Which generators produce the Rust code in `crates/api-contracts` from
`contracts/openapi.yaml` and `contracts/asyncapi.yaml`, where does generated
code live, and what exact CI command proves the checked-in generated code is
current? Sub-questions: is there a usable OpenAPI to Rust generator and a usable
AsyncAPI to Rust generator, or do AsyncAPI channels and messages get handwritten
types over generated schemas; what is the generated versus handwritten boundary
inside `crates/api-contracts`; and what regeneration plus staleness command goes
into CI.

Hard requirements carried into the evaluation: generated types or client must
work over Unix domain sockets and Windows named pipes (pluggable connector, or
standalone generated types plus handwritten transport); the realtime design is
already fixed as "AsyncAPI payloads reuse the OpenAPI schemas" plus "a
subscription over HTTP is a `text/event-stream` response"; and the tooling must
express shared schema references and an SSE response.

## Local repository facts consulted

| Path | Fact used | Notes |
| --- | --- | --- |
| `contracts/openapi.yaml` | Declares `openapi: 3.1.0`, `paths: {}` (placeholder) | `/home/patric/Projects/metis-agent/contracts/openapi.yaml` |
| `contracts/asyncapi.yaml` | Declares `asyncapi: 3.0.0`, `channels: {}`, `operations: {}` (placeholder) | `/home/patric/Projects/metis-agent/contracts/asyncapi.yaml` |
| `crates/api-contracts/Cargo.toml` | Empty `[dependencies]`, `[lints] workspace = true` | No serde yet |
| `crates/api-contracts/src/lib.rs` | Empty file | no generated layout exists yet |
| `Cargo.toml` | Workspace members include `crates/api-contracts`; `[workspace.lints.clippy] all = "warn"` | Generated code must satisfy this |
| `.github/workflows/ci.yml` | Steps run `devenv shell -- cargo fmt --check`, `devenv shell -- cargo clippy --all-targets -- -D warnings`, `devenv shell -- cargo test` | A new step must follow the same `devenv shell --` pattern |
| `devenv.nix` | `languages.rust` (nightly) and `packages = [ pkgs.git pkgs.nodejs ]`; no JVM package | So `cargo` and `node` are provided, `java` is not |
| `docs/rough-plan.md`, section `api-contracts` | "Keep handwritten behavior out of generated code. Verify generator capabilities before assuming OpenAPI and AsyncAPI can generate complete interoperable client/server implementations." | Also "Generated clients may require custom connectors." |
| `docs/tasks/metis-foundations/map.md` | "OpenAPI is the complete functional contract; AsyncAPI references the same payload schemas." and "A subscription's HTTP form is a server-sent events response: the snapshot is one SSE event, the cursor is the SSE `id`, resume uses `Last-Event-ID`..." | Also: client crate is handwritten and UI-independent |

Toolchain check limitation: this run had no shell execution tool, so it could
not run `cargo --version`, `node --version`, or `java -version`, and could not
run any generator. The toolchain statement above is read from `devenv.nix`, not
observed. No scratch directory was created and no generation run was reproduced;
see "Reproduction disclosure" at the end.

## Sources, with quoted passages

1. progenitor README (`https://github.com/oxidecomputer/progenitor`, fetched via
   `https://raw.githubusercontent.com/oxidecomputer/progenitor/main/README.md`):
   - "Progenitor is a Rust crate for generating opinionated clients from API
     descriptions specified in the OpenAPI 3.0.x specification."
   - Client injection point: "let client = Client::new_with_client(baseurl,
     client_with_custom_defaults);" where `client_with_custom_defaults` is a
     `reqwest::ClientBuilder::new()...build()` value.
   - Output modes: `generate_api!` macro, `build.rs` plus `include!`, or
     "cargo progenitor -i INPUT -o OUTPUT -n CRATE -v VERSION" producing a
     stand-alone crate.
   - "if there are any websocket channel endpoints" adds `base64` and `rand`
     dependencies (Dropshot-style WebSocket channel extension support).
2. progenitor issue #762 "Support OpenAPI 3.1.0"
   (`https://github.com/oxidecomputer/progenitor/issues/762`): state `open`,
   47 👍, `created_at 2024-04-04`, `updated_at 2025-10-18`, `closed_at: null`;
   body: "OpenAPI version 3.1.0 came with some breaking changes, notably, the
   introduction of a `null` type and removal of the `nullable` attribute."
3. progenitor issue #1268 "Support for OpenAPI `v3.1.0`"
   (`https://github.com/oxidecomputer/progenitor/issues/1268`): reported error
   text "error: generation error for assets/openapi.json: unexpected or
   unhandled format in the OpenAPI document invalid version: 3.1.0".
4. progenitor PR #1107 (merged 2025-03-30, referenced from the issue search):
   "Because minor spec updates are considered to be fully compatibly (according
   to semver) it makes more sense to check we're within the OAS 3.0.x version
   realm ... using `.starts_with("3.0.")` seemed a fairly low overhead
   solution." So the accepted version gate is 3.0.x.
5. openapi-generator generated metadata, `docs/generators/rust.md`
   (`https://github.com/OpenAPITools/openapi-generator/blob/master/docs/generators/rust.md`):
   "generator name | rust", "generator stability | STABLE", "generator type |
   CLIENT", "helpTxt | Generates a Rust client library (beta).", and `library`
   values "hyper", "hyper0x", "reqwest", "reqwest-trait".
6. openapi-generator generated metadata, `docs/generators/rust-server.md`:
   "generator name | rust-server", "generator type | SERVER", "helpTxt |
   Generates a Rust Hyper/Tower server library. Also generates a matching Hyper
   client library within the same crate that implements the same trait."
7. openapi-generator README
   (`https://github.com/OpenAPITools/openapi-generator`): "OpenAPI Spec
   compatibility: 1.0, 1.1, 1.2, 2.0, 3.0, 3.1 (beta support)"; for the CLI jar:
   "you can grab it directly from Maven.org (Java 11 runtime at a minimum)"; for
   the npm/pip wrappers: "(`java` executable is needed to run)".
8. openapi-generator generated async reqwest client sample
   (`samples/client/petstore/rust/reqwest/petstore-async/src/apis/configuration.rs`):
   "pub struct Configuration { pub base_path: String, ... pub client:
   reqwest::Client, ... }". Both the blocking and async sample configurations
   expose the `reqwest::Client` as a public field, so a caller-supplied client
   can be injected.
9. reqwest CHANGELOG
   (`https://github.com/seanmonstar/reqwest/blob/master/CHANGELOG.md`):
   - "## v0.12.23 - Add `ClientBuilder::unix_socket(path)` option that will
     force all requests over that Unix Domain Socket."
   - "## v0.12.27 - Add `ClientBuilder::windows_named_pipe(name)` option that
     will force all requests over that Windows Named Piper." (the changelog has
     the typo "Piper"; the release page for v0.12.27 says "Windows Named Pipe").
10. reqwest `ClientBuilder` documentation source
    (`https://docs.rs/reqwest/latest/src/reqwest/blocking/client.rs.html`, and
    the async equivalent): "Set that all connections will use this Unix socket.
    If a request URI uses the `https` scheme, TLS will still be used over the
    Unix socket. This option is not compatible with any of the TCP or Proxy
    options." The fields are `unix_socket: Option<Arc<std::path::Path>>` under
    `#[cfg(unix)]` and `windows_named_pipe: Option<Arc<std::ffi::OsStr>>` under
    `#[cfg(target_os = "windows")]`.
11. reqwest issue #39 "Is there a way to use a unix socket?"
    (`https://github.com/seanmonstar/reqwest/issues/39`): "state: CLOSED
    (COMPLETED)", "closed: 2025-08-08", "Closed by pull requests - #2624". The
    old objection in that thread ("there didn't seem to be any hooks in
    ClientBuilder where I could shove a custom Transport") is now partly moot
    for sockets and pipes, though the hook is client-wide, not per request.
12. asyncapi/generator README
    (`https://github.com/asyncapi/generator`): the official template table lists
    `@asyncapi/nodejs-template`, `@asyncapi/nodejs-ws-template`,
    `@asyncapi/java-template`, `@asyncapi/java-spring-template`,
    `@asyncapi/java-spring-cloud-stream-template`,
    `@asyncapi/python-paho-template`, `@asyncapi/html-template`,
    `@asyncapi/markdown-template`, `@asyncapi/ts-nats-template`,
    `@asyncapi/go-watermill-template`, `@asyncapi/dotnet-nats-template`,
    `@asyncapi/php-template`, `@asyncapi/dotnet-rabbitmq-template`. There is no
    Rust template, and most listed templates are marked "(archived)".
13. Modelina README (`https://github.com/asyncapi/modelina`): "We support the
    following AsyncAPI versions: 2.0.0 -> 3.0.0, which generates models for all
    the defined message payloads."; "We support the following OpenAPI versions:
    Swagger 2.0, OpenAPI 3.0 and 3.1 ... which generates models for all the
    defined request and response payloads."; Rust row: "Struct/tuple and enum
    generation: generation of `implement Default`, generate serde macros...";
    "Requirements - NodeJS >= 18"; licence Apache-2.0; versioning policy: "any
    changes that change the generated outcome are not allowed as it's a breaking
    change for the consumer of the generated models."
14. Modelina `AsyncAPIInputProcessor.ts`
    (`https://github.com/asyncapi/modelina/blob/master/src/processors/AsyncAPIInputProcessor.ts`):
    "static supportedVersions = [ '2.0.0', '2.1.0', '2.2.0', '2.3.0', '2.4.0',
    '2.5.0', '2.6.0', '3.0.0' ];" and the traversal
    `for (const channel of doc.channels()) { for (const operation of
    channel.operations()) { ... operation.reply() ... operation.messages() } }`,
    which is the AsyncAPI 3 parser API shape.
15. `docs/usage.md` in Modelina
    (`https://github.com/asyncapi/modelina/blob/master/docs/usage.md`): the
    AsyncAPI section's examples are all titled "AsyncAPI 2.x", and the known
    AsyncAPI limitation recorded there is polymorphism: "Current version of
    Modelina does not generate the expected inheritance of models, instead it's
    current behavior is to merge the schemas together."
16. Typify README (`https://github.com/oxidecomputer/typify`): "Typify compiles
    JSON Schema documents into Rust types."; usage modes: "using the `cargo
    typify` command", "via the macro `import_types!("types.json")`", "via a
    builder interface to generate Rust types in `build.rs` or `xtask`", "via the
    builder functions to generate persistent files e.g. when building API
    bindings"; formatting advice: "rustfmt_wrapper::rustfmt(typespace.to_stream().to_string())?"
    and prettyplease for build scripts; known weak spots: anyOf ("this is one of
    the weaker areas of code generation"), bounded numbers not enforced.
17. Typify crate metadata (`https://crates.io/api/v1/crates/typify`):
    description "JSON schema to rust type code generator", license "Apache-2.0",
    `default_version` / `max_stable_version` 0.8.0, `newest_version`
    0.10.0-alpha.2 (published 2026-10-05).
18. cargo-typify README
    (`https://github.com/oxidecomputer/typify/blob/main/cargo-typify/README.md`):
    "cargo typify my_types.json"; the worked example schema declares `"$schema":
    "http://json-schema.org/draft-07/schema#"`; options include `--output`
    (with `-` for stdout), `--no-builder`, `--additional-derive`.
19. paperclip README (`https://github.com/paperclip-rs/paperclip`): "Paperclip
    offers tooling for the OpenAPI specification. Once complete, it will
    provide: Code generation for efficient, type-safe, compile-time checked HTTP
    APIs (server, client and CLI) in Rust."; "It's currently under active
    development and may not be ready for production use just yet."; licence
    "Apache License, Version 2.0 ... MIT ... at your option". Context:
    issue #308 "Stepping down as a maintainer" (2021, still open) and issue #519
    "Is this crate still actively developed?" (opened 2024-02-03, unanswered in
    the fetched index); crates.io shows the most recent entries at 0.9.5
    (2025-03-10) and 0.9.6 (2025-06-18).
20. Community AsyncAPI Rust templates (only alternatives to Modelsina on that
    side): `kanekoshoyu/asyncapi-rust-ws-template` (MIT, 2 stars, "AsyncAPI
    Template for Generating Rust WebSocket Client", npm weekly downloads 1182)
    and `@ioka-technologies/asyncapi-rust-server-template` (MIT, weekly
    downloads 332, "Rust server side template for the AsyncAPI code generator").
    Neither is an AsyncAPI Initiative official template.
21. OpenAPI Specification v3.1.0 (`https://spec.openapis.org/oas/v3.1.0.html`):
    - Responses Object: "content Map[`string`, Media Type Object] A map
      containing descriptions of potential response payloads. The key is a media
      type or media type range, see [RFC7231 Appendix D]". So an SSE response is
      an ordinary `text/event-stream` entry; there is no dedicated SSE object.
    - Schema dialect: "The `$schema` keyword MAY be present in any root Schema
      Object, and if present MUST be used to determine which dialect should be
      used when processing the schema. This allows use of Schema Objects which
      comply with other drafts of JSON Schema than the default Draft 2020-12
      support. Tooling MUST support the OAS dialect schema id, and MAY support
      additional values of `$schema`."
    - Root object: "jsonSchemaDialect | string | The default value for the
      `$schema` keyword within Schema Objects contained within this OAS
      document."
    - External resources: "When a Schema Object is referenced from an external
      resource which is not an OAS document (e.g. a bare JSON Schema resource),
      then the value of the `$schema` keyword for schemas within that resource
      MUST follow JSON Schema rules".
22. AsyncAPI Specification v3.0.0
    (`https://github.com/asyncapi/spec/blob/v3.0.0/spec/asyncapi.md`):
    - File structure: "An AsyncAPI document MAY be made up of a single document
      or be divided into multiple, connected parts at the discretion of the
      author. In the latter case, Reference Objects are used."
    - Message Object: "payload | Multi Format Schema Object | Schema Object |
      Reference Object | Definition of the message payload. If this is a Schema
      Object, then the `schemaFormat` will be assumed to be
      `application/vnd.aai.asyncapi+json;version=asyncapi`".
    - Multi Format Schema Object MUST-support table: "AsyncAPI 3.0.0 Schema
      Object" (`application/vnd.aai.asyncapi;version=3.0.0`, `+json`, `+yaml`)
      and "[JSON Schema Draft 07] `application/schema+json;version=draft-07`,
      `application/schema+yaml;version=draft-07`". Everything else, including
      2020-12, is only RECOMMENDED or custom ("Custom values are allowed but
      their implementation is OPTIONAL").
    - "When using Reference Objects within the schema, the `schemaFormat` of the
      referenced resource MUST match the `schemaFormat` of the schema containing
      the reference."
23. Git documentation for `git diff --exit-code`
    (`https://git-scm.com/docs/git-diff`): "Make the program exit with codes
    similar to `diff`(1). That is, it exits with 1 if there were differences and
    0 means no differences."

## Candidate table

| Candidate | Spec input | Rust artifacts produced | Custom connector / local IPC | Toolchain cost in CI | Licence | Maintenance | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `openapi-generator` `-g rust` | OAS 2.0/3.0, "3.1 (beta support)" | Client + models as a whole crate; `library` = `hyper`, `hyper0x`, `reqwest`, `reqwest-trait` | reqwest variants: the generated `Configuration` exposes `pub client: reqwest::Client`, and reqwest >= 0.12.23 (UDS) / >= 0.12.27 (named pipe) covers both transports; `hyper` variants build their own connector | JVM (Java 11 minimum) plus generator jar/npm/pip wrapper; not present in `devenv.nix` | Apache-2.0 | Very active, but the Rust client is documented as "beta" | Rejected |
| `openapi-generator` `-g rust-server` | same | Hyper/Tower server library plus a matching Hyper client in the same crate | Hyper-based, no reqwest connector injection | JVM | Apache-2.0 | STABLE generator label | Rejected (server stubs are not wanted; generated client cannot take the IPC transport) |
| `progenitor` | OpenAPI 3.0.x only | Client + types (`generate_api!`, `build.rs`, or standalone crate) | `Client::new_with_client(baseurl, reqwest::Client)` allows injecting a reqwest client, so UDS/named pipes are reachable through reqwest >= 0.12.27 | Rust only | MPL-2.0 | Active (0.15.1, Oct 2026), but 3.1 open since 2024 | Rejected: hard version blocker |
| `paperclip` | OpenAPI (version not pinned in README) | Rust server/client/CLI codegen, described as not yet complete | n/a (codegen incomplete) | Rust only | Apache-2.0 OR MIT | Last changelog entries 0.9.5 (2025-03) and 0.9.6 (2025-06); maintainer stepped down (2021), "still actively developed?" unanswered | Rejected |
| `asyncapi/generator` | AsyncAPI | No official Rust template (Node/Java/Python/Go/TS/.NET/PHP/docs only; most archived) | n/a | Node plus template package | Apache-2.0 | Active project, wrong language coverage | Rejected |
| community AsyncAPI Rust templates | AsyncAPI | Rust WebSocket client template or Rust server template | Assume TCP WebSocket; not our IPC transports | Node plus unpinned third-party template | MIT | Small, single-maintainer, low downloads | Rejected |
| `modelina` | AsyncAPI 2.0.0 to 3.0.0, OpenAPI 3.0/3.1, JSON Schema drafts 4/6/7 | Models only (structs/enums, serde derives) | Not applicable: no client, so the transport requirement is satisfied by handwritten code | Node >= 18 (already in `devenv.nix`) | Apache-2.0 | Active, strong output-stability policy | Viable, but models-only; kept as the documented alternative |
| `typify` (+ `cargo typify`, or as an xtask library) | JSON Schema (README example is draft-07) | Models only | Not applicable: types are transport-free by construction | Rust only, pinned by `Cargo.lock` when used as an xtask dependency | Apache-2.0 | Active stable 0.8.0, 0.10.0-alpha.2 in flight | **Recommended** |
| Models-only plus handwritten channels/transport | n/a | Models from `contracts/schemas/`, everything else handwritten in `api-contracts`/`client` | Handwritten transport over `tokio::net::UnixStream` / Windows named pipe and reqwest's `unix_socket`/`windows_named_pipe` for plain HTTP | Rust only | n/a | n/a | **Recommended strategy** |

## Findings

1. **Claim:** There is no usable AsyncAPI to Rust generator. The AsyncAPI
   Initiative's generator ships no Rust template, and the only Rust templates
   found are third-party, single-maintainer packages with low adoption.
   **Sources:** [asyncapi/generator
   README](https://github.com/asyncapi/generator),
   [asyncapi-rust-ws-template](https://www.npmjs.com/package/asyncapi-rust-ws-template),
   [@ioka-technologies/asyncapi-rust-server-template](https://www.npmjs.com/package/@ioka-technologies/asyncapi-rust-server-template).
   **Support:** direct evidence. **Confidence:** high. **Consequence:** the
   AsyncAPI document's channels and messages are handwritten behavior over
   generated payload types. This is a decision input for the realtime design
   task, exactly as the task file anticipated.

2. **Claim:** Modelina is the only credible Node-based option for AsyncAPI 3.0
   models, but it is models-only and would duplicate the model set if the models
   are already generated from the shared schemas. **Sources:** [Modelina
   README](https://github.com/asyncapi/modelina),
   [AsyncAPIInputProcessor.ts](https://github.com/asyncapi/modelina/blob/master/src/processors/AsyncAPIInputProcessor.ts),
   [npm registry entry](https://registry.npmjs.org/@asyncapi/modelina).
   **Support:** direct evidence for the version list `[..., '3.0.0']`, the
   `channel.operations()` / `operation.reply()` traversal, the Rust output row,
   and the Apache-2.0 licence; interpretation for "would duplicate the model
   set", which follows from the map's rule that both documents reference the
   same payload schemas. **Confidence:** high for the capability, medium for the
   duplication argument.

3. **Claim:** Progenitor cannot consume `contracts/openapi.yaml` as written
   (`openapi: 3.1.0`); the version gate accepts only 3.0.x and the 3.1 request
   has been open since April 2024. **Sources:** [issue
   #762](https://github.com/oxidecomputer/progenitor/issues/762), [issue
   #1268](https://github.com/oxidecomputer/progenitor/issues/1268), [PR
   #1107](https://github.com/oxidecomputer/progenitor/pull/1107) (merged
   2025-03-30, `starts_with("3.0.")`), [progenitor
   README](https://github.com/oxidecomputer/progenitor) ("OpenAPI 3.0.x
   specification"). **Support:** direct evidence (quoted error message
   "invalid version: 3.1.0" and the open issue state with 47 reactions).
   **Confidence:** high. **Caveat:** evidence is from the issue tracker and the
   version-check PR; a pinned 0.15.1 parser was not compiled against the
   contract by this run.

4. **Claim:** openapi-generator does accept 3.1 documents, but only as "beta
   support", its Rust client generator is described as "(beta)", it requires a
   JVM in CI, and it emits a whole crate rather than a module inside an existing
   workspace crate. **Sources:** [openapi-generator
   README](https://github.com/OpenAPITools/openapi-generator) ("3.1 (beta
   support)", "Java 11 runtime at a minimum"), [docs/generators/rust.md](https://github.com/OpenAPITools/openapi-generator/blob/master/docs/generators/rust.md)
   ("Generates a Rust client library (beta)."). **Support:** direct evidence for
   the quotes; interpretation for the "whole crate" friction (the generator's
   own samples are complete crates with `Cargo.toml`, and `rust-server` adds a
   server library plus a matching client). **Confidence:** high for the quotes,
   medium for the workflow-friction judgement.

5. **Claim:** The "generated clients cannot do local IPC" objection is now
   weaker than the earlier design notes assumed: reqwest gained
   `ClientBuilder::unix_socket(path)` in 0.12.23 and
   `ClientBuilder::windows_named_pipe(name)` in 0.12.27, and the reqwest-based
   generated clients expose their `reqwest::Client` so it can be replaced. The
   limitation is that the choice is client-wide, not per request, and is
   "not compatible with any of the TCP or Proxy options", so one client instance
   serves exactly one transport. **Sources:** [reqwest
   CHANGELOG](https://github.com/seanmonstar/reqwest/blob/master/CHANGELOG.md),
   [reqwest ClientBuilder source](https://docs.rs/reqwest/latest/src/reqwest/blocking/client.rs.html),
   [reqwest issue #39](https://github.com/seanmonstar/reqwest/issues/39),
   [openapi-generator async sample
   configuration.rs](https://github.com/OpenAPITools/openapi-generator/blob/master/samples/client/petstore/rust/reqwest/petstore-async/src/apis/configuration.rs).
   **Support:** direct evidence for the API quotes; interpretation for "one
   client instance serves exactly one transport". **Confidence:** high for UDS
   and the injection point, medium for Windows named pipes because the
   `cfg(target_os = "windows")` path cannot be exercised on this Linux host and
   the changelog release note for 0.12.27 is the only authority consulted.

6. **Claim:** An SSE response needs no generator support at all: OpenAPI 3.1
   expresses it as an ordinary media type key in the response `content` map, and
   no candidate generator produces typed SSE framing. **Sources:** [OAS
   3.1.0](https://spec.openapis.org/oas/v3.1.0.html) ("The key is a media type
   or media type range"), [map.md](docs/tasks/metis-foundations/map.md) (SSE is
   the HTTP form of a subscription). **Support:** direct evidence for the media
   type rule; researcher inference that generated code would surface the body as
   bytes or a string and that `id`/`event`/`data` framing plus `Last-Event-ID`
   handling stays handwritten. **Confidence:** high for the spec reading, medium
   for the generated-body claim, which was not reproduced.

7. **Claim:** Shared schema references across the two documents constrain the
   schema dialect. AsyncAPI 3.0 implementations MUST support only the AsyncAPI
   3.0 schema format and JSON Schema draft-07; 2020-12 would be a custom value
   whose implementation is optional. OpenAPI 3.1 defaults to the OAS 2020-12
   dialect but permits a different dialect through `jsonSchemaDialect` or a
   per-schema `$schema`, and requires external bare JSON Schema resources to
   carry their own `$schema`. **Sources:** [AsyncAPI
   3.0.0](https://github.com/asyncapi/spec/blob/v3.0.0/spec/asyncapi.md)
   (Multi Format Schema Object table), [OAS
   3.1.0](https://spec.openapis.org/oas/v3.1.0.html) (`$schema`,
   `jsonSchemaDialect`, external resources). **Support:** direct evidence.
   **Confidence:** high. **Consequence:** author `contracts/schemas/*.json` as
   JSON Schema draft-07 so both documents reference the same files with maximal
   tool support. Note that this is my recommendation, not a spec requirement.

8. **Claim:** Typify is the best-fitting generator for the models-only
   strategy: Rust-native, Apache-2.0, usable as a library from an xtask so the
   generator version is pinned by `Cargo.lock`, formats its output with rustfmt,
   and its documented example is a draft-07 JSON Schema. **Sources:** [Typify
   README](https://github.com/oxidecomputer/typify), [crates.io typify
   metadata](https://crates.io/api/v1/crates/typify), [cargo-typify
   README](https://github.com/oxidecomputer/typify/blob/main/cargo-typify/README.md).
   **Support:** direct evidence for the usage modes, licence, and formatting
   guidance; interpretation for "best-fitting", derived from the Rust-only
   toolchain constraint and the models-only requirement. **Confidence:** medium
   to high. **Caveat:** typify is a JSON Schema compiler, not an OpenAPI or
   AsyncAPI consumer; the argument only works if the payloads live in
   `contracts/schemas/` and both documents `$ref` them.

9. **Claim:** Paperclip is the wrong direction and is not a safe dependency for
   this decision: it is server-code-first spec generation whose client codegen
   is described as not yet complete, with a 2021 maintainer step-down and an
   unanswered 2024 "still actively developed?" issue. **Sources:** [paperclip
   README](https://github.com/paperclip-rs/paperclip), [issue
   #519](https://github.com/paperclip-rs/paperclip/issues/519), [issue
   #308](https://github.com/paperclip-rs/paperclip/issues/308), [crates.io
   paperclip](https://crates.io/crates/paperclip). **Support:** direct evidence
   for the README sentences and issue titles; interpretation for "unsafe
   dependency". **Confidence:** high.

10. **Claim:** The output location and boundary should make the generated set
    mechanically identifiable. **Sources:** local
    [rough-plan.md](docs/rough-plan.md) ("Keep handwritten behavior out of
    generated code"), local [map.md](docs/tasks/metis-foundations/map.md),
    local [ci.yml](.github/workflows/ci.yml), local
    [Cargo.toml](Cargo.toml) (`[workspace.lints.clippy] all = "warn"`).
    **Support:** researcher recommendation, constrained by local files.
    **Confidence:** high for the constraint (clippy with `-D warnings` plus
    `cargo fmt --check` already run in CI, so generated code must be
    rustfmt-clean and clippy-clean or carry module-level allows), medium for the
    exact paths.

## Recommendation and decision input

**Generate payload models only; handwrite the channels, the SSE handling, and
the transport.**

1. Per document:
   - `contracts/openapi.yaml`: no generator consumes it. It stays the complete
     functional contract, and its payloads `$ref` into `contracts/schemas/`.
   - `contracts/asyncapi.yaml`: no generator consumes it either. Its messages
     reference the same `contracts/schemas/` files, and its channels, subscribe
     semantics, and SSE mapping are handwritten.
   - `contracts/schemas/*.json`: the generated source. Author them as JSON
     Schema draft-07 (see finding 7) and compile them with **typify** from a
     workspace xtask.
2. Why not a document-consuming generator: progenitor is a hard no on 3.1
   (finding 3); openapi-generator is viable but beta, JVM-dependent, and emits a
   whole crate (finding 4); the AsyncAPI side has no Rust generator at all
   (finding 1); paperclip is unfinished (finding 9). Because
   `docs/rough-plan.md` already says the client is handwritten and the map
   already fixes SSE plus IPC transports, the client-side code that a generated
   client would replace has to be written by hand anyway, and a generated client
   would only add a second transport stack that cannot serve the WebSocket and
   SSE paths over named pipes.
3. Keep Modelina (finding 2) as the documented alternative if the team prefers
   document-driven generation or if typify's draft-07 path proves inadequate:
   same models-only shape, Node already present in `devenv.nix`, Apache-2.0,
   and a stated policy that output-changing changes require a major version.
4. Placement:
   - Source of truth: `contracts/openapi.yaml`, `contracts/asyncapi.yaml`,
     `contracts/schemas/`.
   - Generated: `crates/api-contracts/src/generated/` (one file per top-level
     schema plus `generated/mod.rs`), every file starting with a marker comment
     such as `// @generated by cargo xtask contracts gen. Do not edit.` The
     marker is what the staleness check and a lint can key on.
   - Handwritten: `crates/api-contracts/src/lib.rs` (module declarations,
     re-exports, crate docs, clippy allows) and all of `crates/client` and
     `crates/daemon`. Nothing under `src/generated/` is ever edited by hand.
   - Generated code needs `serde` (and whatever typify reports through
     `TypeSpace::uses_chrono`/`uses_uuid`/`uses_regress`) as real dependencies of
     `crates/api-contracts`. Add a module-level allow list if typify output trips
     the workspace's `clippy::all` warnings under `-D warnings`.
5. Regeneration command (developer-facing):
   `cargo run -p xtask -- contracts gen` (a small workspace xtask crate that
   depends on `typify` and `rustfmt-wrapper`; it loads each
   `contracts/schemas/*.json` into a `TypeSpace` in a fixed order, renders one
   file per schema, formats with rustfmt, and writes
   `crates/api-contracts/src/generated/`).
6. CI staleness check, following the existing workflow's `devenv shell --`
   convention:
   ```yaml
   - name: Contracts generated code is current
     run: devenv shell -- cargo run -p xtask -- contracts gen --check
   ```
   `--check` regenerates into `target/contracts-gen/`, byte-compares against the
   committed files, prints a unified diff of the drift, and exits non-zero on
   any difference. The equivalent diff-based form, which is the fallback if
   `--check` is not implemented first, is:
   ```yaml
   - name: Contracts generated code is current
     run: |
       devenv shell -- cargo run -p xtask -- contracts gen
       git diff --exit-code -- crates/api-contracts/src/generated
   ```
   It fails because `git diff --exit-code` "exits with 1 if there were
   differences and 0 means no differences"
   ([git-diff](https://git-scm.com/docs/git-diff)). The existing
   `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` steps
   then also police the freshly generated code, and `cargo test` compiles it.
7. Because no tool reads the two documents for types, add a second, cheap xtask
   lint (`cargo run -p xtask -- contracts lint`) that parses both documents and
   fails unless every request body, every response body schema, and every
   AsyncAPI message payload is a `$ref` into `contracts/schemas/` with the
   expected `schemaFormat`. Without it, inlining a schema in `openapi.yaml`
   would silently leave the generated types stale while the check still passes.
   This is researcher inference from the models-only strategy, not an external
   requirement. Optional and additive: lint the documents themselves
   (`npx @stoplight/spectral-cli lint contracts/openapi.yaml`,
   `npx @asyncapi/cli validate contracts/asyncapi.yaml`) since Node is already
   in the devenv; not verified in this run.

## Impact on dependent tasks

- `api-contracts` implementation tickets: the crate gains a real dependency set
  (`serde`, plus typify-reported extras) and a `src/generated/` tree with a
  marker convention; the only hand-written file is `src/lib.rs`.
- `client` implementation: confirms the handwritten design. Plain HTTP can use
  `reqwest` with `ClientBuilder::unix_socket(path)` on Unix and
  `ClientBuilder::windows_named_pipe(name)` on Windows (one client per
  transport, never both); SSE can consume `bytes_stream()`; the WebSocket
  upgrade and frames need a stream-level client over the same socket or pipe.
  The transport abstraction has to be below the HTTP layer, not inside a
  generated client.
- Walking skeleton: unaffected in shape, but there is no generated
  `subscribe` method to call; the client method that opens the SSE response is
  handwritten.
- CI task: add the `contracts gen --check` step in the `devenv shell --` style,
  and accept that generated code participates in fmt, clippy, and test.
- Realtime design task: record that AsyncAPI has no usable Rust generator, so
  the AsyncAPI document is documentation plus lint target, not a code source;
  keep its payloads as draft-07 `$ref`s into `contracts/schemas/`; express the
  subscription's HTTP form as a `text/event-stream` response entry.
- Plugin contract work is untouched by this decision.

## Contradictions and disputed evidence

- Modelina's README claims AsyncAPI "2.0.0 -> 3.0.0" input support, and the
  processor source lists `'3.0.0'` and walks the AsyncAPI 3 parser shape, while
  `docs/usage.md` still shows only "AsyncAPI 2.x" examples. The code-level
  evidence is stronger; treat the docs as stale, but treat 3.0 support as
  verified from source, not from a generation run.
- Modelina's package metadata records `engines: { node: ">=14" }` on older
  entries while the current README says "NodeJS >= 18". Use 18 as the floor.
- The reqwest changelog spells the 0.12.27 option as "Windows Named Piper"
  while the release page says "Windows Named Pipe". Cosmetic, but quote the
  release page when citing it.

## Missing evidence

- No generation run and no compile: this run had no shell, so no candidate was
  reproduced against a minimal contract and nothing was compiled. All generator
  claims rest on upstream documentation, samples, and issue trackers.
- `cargo`, `node`, and `java` availability was read from `devenv.nix`, not
  executed: `devenv shell -- cargo --version`, `node --version`, and
  `java -version` were never run, and the report does not claim otherwise.
- Windows behaviour is unverified. Nothing here was tested on Windows; the named
  pipe path is cited only from reqwest's release notes and source.
- Typify's support for cross-file `$ref` between schema files, and for JSON
  Schema 2020-12, is not documented in the README or the pages consulted; only a
  draft-07 example and internal `$ref` handling appear. Verify before relying on
  cross-file references, or keep each schema file self-contained with `$defs`.
- Whether the pinned `typify` version can process the eventual real contracts
  (subscription envelopes, oneOf error payloads, `format: date-time`) is
  untested. Typify's anyOf handling and unenforced numeric bounds are documented
  weak spots.
- openapi-generator's ability to emit models only (`--global-property models`)
  and to resolve `$ref`s into `contracts/schemas/` at a pinned release was not
  verified; it matters only if the team revisits that candidate.
- Whether `@asyncapi/generator` itself accepts AsyncAPI 3.0 input was not
  verified; it does not change the recommendation, because there is no official
  Rust template.

## Reproduction disclosure

No scratch directory was created. No generator was installed or run, no code was
compiled, and no contract was modified. The task file's request to test
generation against a minimal contract is therefore not satisfied by this run;
that verification belongs to the `api-contracts` implementation ticket, which
should generate the real `contracts/schemas/` payloads with a pinned typify and
confirm the compile inside `devenv shell`.

## Sources

- Kept: progenitor README and issue tracker (README, #762, #1268, PR #1107) -
  decisive evidence that OpenAPI 3.1.0 is rejected and that the client takes an
  injected reqwest client.
- Kept: openapi-generator README plus `docs/generators/rust.md` and
  `rust-server.md`, and the generated async `configuration.rs` sample - exact
  "3.1 (beta support)" and "Rust client library (beta)" wording, Java 11
  requirement, and the injection point.
- Kept: reqwest CHANGELOG v0.12.23 and v0.12.27, docs.rs source for
  `ClientBuilder`, and issue #39 - the transport finding that changes the
  calculus for generated clients.
- Kept: asyncapi/generator README template table - proves the absence of an
  official Rust template.
- Kept: Modelina README, `AsyncAPIInputProcessor.ts`, npm registry entry -
  versions supported, Rust output scope, licence, Node requirement.
- Kept: typify README, crates.io metadata, cargo-typify README - the
  recommended generator, its licence, and its usage modes.
- Kept: paperclip README, issues #519 and #308, crates.io entry - rejection
  reasons.
- Kept: OpenAPI 3.1.0 specification and AsyncAPI 3.0.0 specification - media
  type keys, dialect rules, `schemaFormat` rules.
- Kept: git-diff documentation - the exact failure semantics of
  `--exit-code`.
- Deprioritized: code-community write-ups and search summaries that repeated
  generator feature lists; superseded by the primary READMEs, generator
  metadata files, and issue trackers above.

## Later amendment

Superseded in conclusion by `protobuf-connect-research` and `connectrpc-spike`:
the contract is Protobuf with ConnectRPC, so no JSON-schema or OpenAPI generator
is used. The findings below remain the evidence for why the OpenAPI generator
path was not taken, and the measured baseline the Protobuf path was compared
against.

1. In the `api-contracts` ticket, create the xtask, generate the first real
   payload models with a pinned `typify`, and confirm `cargo build`, `cargo fmt
   --check`, and `cargo clippy --all-targets -- -D warnings` all pass inside
   `devenv shell`. This is the only remaining reproduction the task asked for.
2. In the realtime design task, decide the shared schema dialect (draft-07
   recommended) and record the `schemaFormat` string for AsyncAPI message
   payloads.
3. In the client ticket, prove over a Unix socket, inside the devenv, that one
   `reqwest` client with `unix_socket()` can issue an ordinary request and read
   a `text/event-stream` body, and that a stream-level WebSocket client can use
   the same socket. Leave the Windows named pipe path as a compile-only check
   until a Windows runner exists.
