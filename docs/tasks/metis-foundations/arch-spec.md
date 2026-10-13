---
type: arch spec
title: arch-spec, metis-foundations
status: stable
---

# arch-spec, metis-foundations

Effort-wide architecture spec, shared by every ticket chain in this effort. It
covers the pending frontier in depth and states the interface contracts the
later tickets inherit. Later levels get their own sections added by the
spec-reconciler when they become pending.

## Source of truth for the target state

- Architecture: `docs/rough-plan.md`
- Effort destination, decisions, fog: `docs/tasks/metis-foundations/map.md`
- Effort spec: `docs/tasks/metis-foundations/spec.md`
- Contract decisions: `docs/adr/0001-protobuf-connect-instead-of-openapi.md`
- Plugin ABI decisions: `docs/tasks/metis-foundations/tasks/plugin-abi-layout/task.md`
  (D1 to D15 plus the two spike amendment lists)
- Prior art in tree: `spikes/connectrpc/`, `spikes/plugin-abi-mechanics/`,
  `spikes/plugin-authoring-trait/`, `spikes/openapi-codegen/`
- Frontier: `tw_frontier metis-foundations`

## Shared conventions

- Ticket branches: landing branch `task/<ticket-slug>` off the previous ticket's
  landing branch (`main` for the first), working branch `ticket/<ticket-slug>`.
  Chains land in dependency-level order.
- Toolchain: devenv only (`devenv.nix`, `devenv.lock`). Rust nightly through
  rust-overlay, edition 2024, `resolver = "3"`. Add a tool by adding it to
  `devenv.nix` and running it as `devenv shell -- <cmd>`, matching the existing
  CI style.
- Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, all through `devenv shell --`. Every crate carries
  `[lints] workspace = true`.
- Rustfmt: `crates/` and `contracts/` config stays as committed
  (`edition = "2024"`, `max_width = 100`).
- Generated code is mounted, never edited; handwritten behaviour lives outside
  the generated trees. Generated trees are not excluded from fmt, clippy, or
  test, they carry the measured allow set.
- Dependency posture: pin the pre-1.0 protocol pair exactly (`connectrpc` and
  `buffa` move together). Keep `plugin-contract` free of async runtimes and of
  serde in its C ABI surface.
- Prose and comments follow `AGENTS.md`: no em-dashes.
- Names fixed by this spec: crate `plugin-contract`, crate `plugin-fixture`,
  proto package `metis.v1`, service `DaemonService`, entry symbol
  `metis_plugin_entry_v1`, feature gate `authoring`.

---

## Pending ticket: `contract-pipeline`

Builds the contract pipeline end to end: one `.proto`, pinned BSR generation
into committed Rust, the regeneration command, and the CI gates.

### Exports

- `contracts/proto/metis/v1/daemon.proto`: `syntax = "proto3"`, package
  `metis.v1`, `service DaemonService` carrying one ordinary RPC for now:
  `rpc Echo(EchoRequest) returns (EchoResponse)`. Messages:
  `EchoRequest { string message = 1; }` and
  `EchoResponse { string message = 1; string received_at = 2; }`, where
  `received_at` is an RFC 3339 string, mirroring the spike contract. A later
  ticket (`event-stream`) appends `StreamEvents` and its messages; this ticket
  reserves the shape without declaring them.
- `contracts/buf.yaml`: module root `proto`, `lint` STANDARD with no exception
  yet (the `RPC_RESPONSE_STANDARD_NAME` exception arrives with the streaming
  `Event` message, and is recorded then), `breaking` FILE.
- `contracts/buf.gen.yaml`: `version: v2`, `clean: true`, two BSR remote plugins
  pinned by tag:
  - `remote: buf.build/anthropics/buffa:v0.9.2`, out
    `../crates/api-contracts/generated/buffa`, opts `views=true`, `json=true`,
    `file_per_package=true`
  - `remote: buf.build/connectrpc/rust:v0.9.0`, out
    `../crates/api-contracts/generated/connect`, opts
    `buffa_module=crate::proto`, `file_per_package` (bare option, the plugin
    rejects `=true`; measured)
- Committed generated trees:
  `crates/api-contracts/generated/buffa/metis.v1.rs` (messages and views) and
  `crates/api-contracts/generated/connect/metis.v1.rs` (service trait, register
  helper, generated client).
- `crates/api-contracts/src/lib.rs`: `pub mod connect; pub mod proto;` plus
  crate docs. `src/proto.rs` declares the nested `metis::v1` module tree and
  mounts the buffa tree with `include!` and
  `#![allow(clippy::derivable_impls)]`. `src/connect.rs` does the same for the
  connect tree with `#![allow(clippy::match_single_binding)]`. The mount point
  is `crate::proto`, which is exactly what `buffa_module` names.
- `crates/api-contracts/Cargo.toml` dependencies: `connectrpc = "=0.9.1"` with
  `default-features = false, features = ["client", "server", "json"]`,
  `buffa = "=0.9.2"`, `serde = "1"`, `http = "1"`, `http-body = "1"`. The
  connect `json` feature stays on because the generated buffa types carry JSON
  impls and the spike measured that combination; whether the JSON codec is
  served to outside clients remains the map's fog.
- `scripts/regenerate-contracts.sh` with an optional `--check`: regenerate with
  `buf generate`, apply the rustfmt post-step to both generated trees, and in
  `--check` mode regenerate into a temporary directory, format it, and diff
  against the committed trees, exiting non-zero on any difference.
- `contracts/README.md` documenting the one developer-facing regeneration
  command and what the CI gates assert.
- CI (`.github/workflows/ci.yml`) additions, all in the existing
  `devenv shell --` style: `actions/checkout` gains `fetch-depth: 0` (needed by
  `buf breaking`), then `buf lint` (from `contracts/`),
  `buf breaking --against '.git#branch=main'`, and
  `bash scripts/regenerate-contracts.sh --check`.
- `devenv.nix`: add `pkgs.buf` so `buf` is on the shell path (pinned by
  `devenv.lock`).

### Existing abstractions to use

- `spikes/connectrpc/buf.yaml`, `buf.gen.yaml`, `scripts/regenerate.sh`, and the
  `include!` mount pattern in `spikes/connectrpc/crates/wire/src/lib.rs` are the
  measured prior art. Copy the shape, not the spike paths.
- `docs/testing.md` owns the run commands and CI conventions; the contract gates
  join the existing CI job rather than a second workflow.
- `crates/api-contracts` already exists as an empty workspace member with the
  workspace lint posture. Fill it, do not reshape the workspace.

### Do NOT reimplement

- Do not hand-write generated message, client, or server code, and do not add a
  `build.rs` generation path. This ticket is the committed-generation path
  (`buf generate` into the tree), which is what makes the regeneration check
  meaningful.
- Do not use the local-only `protoc-gen-buffa-packaging` plugin. `file_per_package`
  plus the handwritten module tree replaces it (measured).
- Do not install `protoc`, a JVM, or local plugin binaries. BSR remote plugins
  only.
- Do not exclude generated trees from `cargo fmt` or `clippy`. The measured
  footprint is two module-level allows.
- Do not re-declare the buffa messages by hand in `src/proto.rs`; the module
  tree is handwritten, the types are included.
- Do not put handwritten behaviour in `generated/`.

### Seams

Each seam is a public boundary the worker tests; nothing deeper.

1. **Message seam.** `api_contracts::proto::metis::v1::{EchoRequest, EchoResponse}`
   round-trip through the buffa message API the generated types implement.
2. **Service seam.** The generated `DaemonService` trait is implementable by a
   dummy struct (with the measured `#[allow(refining_impl_trait)]`), and the
   generated register helper builds a `Router`. This is a compile-and-construct
   check, not a served call; the served path belongs to
   `local-transport-rpc-skeleton`.
3. **Script seam.** `bash scripts/regenerate-contracts.sh --check` exits 0 on a
   clean tree and non-zero after a hand edit to a generated file.
4. **Gate seam.** `buf lint` and `buf breaking --against '.git#branch=main'`
   exit 0 on the committed contract, and the CI job runs both.

Test types: an integration test at `crates/api-contracts/tests/contract.rs` for
seams 1 and 2; the two scripts and the two buf commands for seams 3 and 4.

### Interface contract

- The message namespace for the whole effort is `api_contracts::proto::metis::v1`;
  the service namespace is `api_contracts::connect::metis::v1`. Later tickets add
  RPCs and messages under the same package and regenerate through the same
  script.
- The generated client is what `client` wraps and what
  `local-transport-rpc-skeleton` injects a transport into. This ticket does not
  export the transport module.
- Any server implementation of `DaemonService` must carry
  `#[allow(refining_impl_trait)]` on the `impl` block, because the generated
  trait uses an explicit RPITIT signature and the workspace denies warnings.
  This applies to `local-transport-rpc-skeleton` and to the daemon effort.
- `buf breaking` in FILE mode is the compatibility gate for later contract
  additions: adding RPCs and messages is allowed, changing or removing the echo
  surface is not.

---

## Pending ticket: `plugin-abi-core`

Builds the plugin boundary itself: the primitive ABI, the generated header, and
a fixture plugin driven by a harness.

### Exports

- `crates/plugin-contract/` library, module `abi` (the only cbindgen surface):
  - `pub const METIS_ABI_MAJOR: u32 = 1;`, `pub const METIS_ABI_MINOR: u32 = 0;`,
    `pub const METIS_PLUGIN_ENTRY_V1: &[u8] = b"metis_plugin_entry_v1\0";`
  - `#[repr(C)] pub enum MetisStatus` with the frozen members `Ok = 0`,
    `InvalidArgument = 1`, `NotFound = 2`, `PermissionDenied = 3`,
    `Cancelled = 4`, `Internal = 5`, `Unsupported = 6`, `Pending = 7`
    (D7 as amended by the mechanics spike).
  - `#[repr(C)] pub struct MetisBuffer { ptr, len, cap, owner, reserved }` with
    the `METIS_BUFFER_OWNER_{NONE,HOST,PLUGIN}` constants (D6).
  - `#[repr(C)] pub struct MetisTablePrefix { size, abi_major, abi_minor }` and
    `pub unsafe fn metis_read_prefix<'a>(table: *const MetisPluginTableV1)
    -> Option<&'a MetisTablePrefix>`, which checks the reported `size` before the
    fields past `size` are read (D8).
  - The function-pointer type aliases `MetisCreateInstanceFn`,
    `MetisDestroyInstanceFn`, `MetisListCapabilitiesFn`, `MetisInvokeFn`,
    `MetisCancelFn`, `MetisReleaseBufferFn`, `MetisShutdownFn`,
    `MetisCompleteFn`, `MetisProgressFn`, `MetisEmitEventFn`, `MetisLogFn`.
  - `#[repr(C)] pub struct MetisPluginTableV1 { size, abi_major, abi_minor,
    create_instance, destroy_instance, list_capabilities, invoke, cancel,
    release_buffer, shutdown }`, 72 bytes on 64-bit, `size` first, presence
    decided by `size` and never by a null pointer (D1, D8).
  - `#[repr(C)] pub struct MetisHostApiV1 { size, user_data, complete, progress,
    emit_event, log }` (D1 plus the `user_data` amendment).
  - `pub type MetisPluginEntryV1Fn = unsafe extern "C" fn() -> *const
    MetisPluginTableV1;` (argument-free, frozen within the major version).
- `crates/plugin-contract/src/abi/layout.rs`: Rust const assertions of the size,
  alignment, and every field offset of the four layout types, written in terms of
  `size_of::<usize>()` so they hold per target (D13 amendment).
- `crates/plugin-contract/c/layout_static_asserts.c`: includes the committed
  header and static-asserts the same numbers in pointer-size terms with
  `_Static_assert` (`-std=c11`). Compiled by `crates/plugin-contract/build.rs`
  through the `cc` build-dependency on every build, so CI's existing build and
  test steps compile it.
- `crates/plugin-contract/src/payload.rs`, outside the C ABI surface: the D4
  header constants (`METIS_PAYLOAD_VERSION_MASK`, `METIS_PAYLOAD_CODEC_BIT`,
  `METIS_PAYLOAD_RESERVED_MASK`, `METIS_PAYLOAD_FORMAT_VERSION`,
  `METIS_CODEC_JSON`, `METIS_CODEC_MESSAGEPACK`), the header accessors, and
  `metis_validate_payload_header` returning `InvalidArgument` for a bad format
  version or non-zero reserved bits and `Unsupported` for a codec this build does
  not know, plus encode and body-strip helpers.
- `crates/plugin-contract/cbindgen.toml` and the committed generated header
  `crates/plugin-contract/include/metis_plugin_abi.h`.
- `scripts/regenerate-abi-header.sh` with an optional `--check`, mirroring the
  contracts script: regenerate the header from the crate, and in `--check` mode
  regenerate into a temporary file and diff, exiting non-zero on drift.
- `crates/plugin-fixture/`, one workspace member, test support:
  - `src/lib.rs` a `cdylib` fixture implementing the v1 table with
    `tool.echo` (inline), a pending capability completed through the host
    `complete` callback with a `progress` callback, a cancel path, a
    late-completion-after-cancel path, a panic path, and an allocation ledger
    that reports `allocs`, `frees`, `live`, `violations`, `suppressed`.
  - `src/bin/abi-harness.rs` the harness: it builds the fixture first with
    `env!("CARGO")` (`cargo build -p plugin-fixture --lib`), then loads the
    library placed next to `current_exe()`, prints one `[ok]`/`[fail]` line per
    check, prints a `[result] PASS`/`FAIL` line, and exits non-zero on any
    failure. Rebuilding from the harness is the spike's stale-library lesson
    encoded rather than documented.
- `Cargo.toml` and `devenv.nix` additions: workspace member
  `crates/plugin-fixture`; workspace dependencies for `libloading = "0.9"`,
  `serde = "1"`, `serde_json = "1"`, and the `cc` build-dependency for
  `plugin-contract`; `pkgs.cbindgen` in `devenv.nix`.
- CI: one step running the harness
  (`devenv shell -- cargo run -p plugin-fixture --bin abi-harness`) and one
  running `bash scripts/regenerate-abi-header.sh --check`.

### Existing abstractions to use

- `spikes/plugin-abi-mechanics/crates/abi/` is the measured shape of every type
  above, including `cbindgen.toml`, `build.rs`, `c/layout_static_asserts.c`,
  `src/layout.rs`, and `tests/c_layout.rs`. Port the shape into
  `crates/plugin-contract`; do not depend on the spike crates.
- `spikes/plugin-abi-mechanics/crates/plugin` and `crates/host` are the prior art
  for the fixture and the harness, including the PASS/FAIL line pattern.
- `spikes/plugin-abi-mechanics/crates/header-gen` is the prior art for the
  header regeneration and `--check` diff.
- `libloading` 0.9.0 and `cbindgen` 0.29 are the versions the spikes measured.
- `env!("CARGO")` plus `std::process::Command` for the fixture rebuild inside the
  harness.
- The effort's plugin ABI seam in `docs/testing.md`'s spirit: a real shared
  library loaded through `plugin-contract`, no mocks, no re-implementation of the
  ABI in the harness.

### Do NOT reimplement

- Do not re-litigate D1 to D15 or the spike amendments. They are settled.
- Do not write table fields as `Option<NamedFnAlias>`. cbindgen 0.29.4 renders
  that as an opaque struct value and the header stops compiling (measured).
- Do not let any entry point take `MetisPluginTableV1` in a parameter. cbindgen
  then emits the function typedef before the struct and C scopes the struct tag
  to the prototype (measured).
- Do not skip the `[export] include` list in `cbindgen.toml`: a crate whose ABI
  surface is a table of function pointers exports no functions to seed the
  dependency graph, so cbindgen emits only constants (measured).
- Do not hand-write the header, and do not edit it to fix a compile error. Fix
  the Rust source.
- Do not add a cross-boundary allocator, a shared heap, or a `free` call from the
  host into plugin memory. Each side releases only what it allocated, including a
  payload the receiver discards.
- Do not put serde types or payload structs inside the `abi` module; keep the
  cbindgen surface primitive.
- Do not use Rust `String`, `Vec`, trait objects, futures, or channel types at
  the boundary, and do not add an async runtime to `plugin-contract`.
- Do not leave the header drift check or the C compile check out of CI.
- Do not make the harness a `#[test]` that loads a library built by an earlier
  unrelated command without rebuilding it.

### Seams

1. **Entry symbol seam.** The fixture cdylib defines exactly one dynamic symbol,
   `metis_plugin_entry_v1`, and no other `metis_*` symbol. Asserted with
   `nm -D --defined-only` or the equivalent in CI.
2. **Table seam.** The harness resolves the symbol, reads `size`, `abi_major`,
   and `abi_minor`, and drives instance creation, capability listing, invoke
   (inline and pending), cancel, buffer release, and shutdown from the real
   library.
3. **Envelope seam.** `crate::payload` round-trips a header, rejects a bad
   format version or set reserved bit with `InvalidArgument`, and rejects an
   unknown codec with `Unsupported`. Exercised as a unit test and across the
   boundary from the harness.
4. **Ownership seam.** The fixture ledger and the harness counter agree,
   `allocs == frees`, `live == 0`, `violations == 0`, including a completion the
   harness drops and still releases.
5. **Panic seam.** A panic inside an exported adapter and a panic inside a host
   callback each become `MetisStatus::Internal`, and the instance still serves
   the next call.
6. **Layout seam.** The Rust const assertions and the C translation unit
   static-assert the same numbers in pointer-size terms, and both compile on
   every build.
7. **Header seam.** `bash scripts/regenerate-abi-header.sh --check` exits 0 on a
   clean tree and non-zero after a hand edit to the header.

Test types: unit tests for seam 3; the harness binary for seams 1, 2, 4, and 5;
compile-time assertions for seam 6; the script for seam 7.

### Interface contract

- `plugin-compatibility` reads only the prefix. It uses `metis_read_prefix` and a
  `size`-based presence predicate, never a direct field read past the prefix it
  knows, and it appends nothing to the layout. Per-capability version gating
  reads descriptor payloads, not the table.
- `plugin-authoring-trait` lives in the same crate behind a feature gate named
  `authoring` and imports only `crate::abi` and `crate::payload`. It supplies
  `table::<P: Plugin + Default>()` returning a `const MetisPluginTableV1`, and
  the host adapter. The trait adapter always returns `Pending` (D15) while the
  ABI keeps its inline path for direct implementers.
- `plugin-runtime` (metis-plugins) and `metis-plugin-sdk` consume these types;
  this effort hands them the ABI and the layout guarantees, not a loader.
- The fixture and the harness are test support. Nothing outside
  `crates/plugin-fixture` depends on them, and the fixture's payload types are
  deliberately not exported from `plugin-contract`.

---

## Downstream interface contracts (outline)

These tickets are not pending yet. Their sections are added when they reach the
frontier; the commitments below are what the two landing tickets must already
support.

- `local-transport-rpc-skeleton`: `crates/api-contracts` gains a handwritten
  `transport` module (Unix socket serve loop plus connector) shared by the
  skeleton, `client`, and the daemon effort; the skeleton service implements
  `DaemonService` with the `refining_impl_trait` allow.
- `event-stream`, `stream-resume-and-expiry`, `cancellation-and-slow-consumers`:
  append `StreamEvents` and `Cancel` to `metis.v1`, with `last_event_id` in the
  request, the snapshot as the stream's first message, heartbeats as ordinary
  messages, and correlation ids in request metadata. The
  `RPC_RESPONSE_STANDARD_NAME` lint exception is added here, and the contract
  regenerates through `scripts/regenerate-contracts.sh`.
- `windows-named-pipe`, `tcp-listener`, `tls-and-client-trust`,
  `reverse-proxy-mode`: extend the same `transport` module; no second service
  implementation.
- `local-identity-and-acl`, `pairing-and-tokens`, `remote-pairing`: add request
  metadata and RPCs under `metis.v1`; credentials travel as
  `Authorization` metadata on every call, including the stream.
- `plugin-authoring-trait`: the `authoring` feature in `plugin-contract`, with
  the settled cancellation token and always-pending adapter, plus the
  descriptor/dispatch consistency check and the lifecycle and `emit_event` gaps
  the authoring spike recorded.
- `plugin-compatibility`: prefix reading and per-capability version gating on
  top of the unchanged layout.

## Architecture notes

- The pending tickets are independent (no `blocked_by` edge between them), so
  they run in level 0 sequentially on the shared repo cwd, in the order
  `tw_dependency_levels` returns.
- Both tickets touch the workspace root (`Cargo.toml`, `devenv.nix`,
  `.github/workflows/ci.yml`) and `crates/`, which is why they cannot run in
  parallel and why the second chain lands on the first chain's landing branch.
- `contract-pipeline` adds the first committed generated tree in the repository,
  so the fmt and clippy allowances it establishes are the precedent every later
  generated tree follows.
- `plugin-abi-core` is the first place the repository compiles C. The `cc`
  build-dependency plus `-std=c11` is the measured minimum; the C compiler is
  already in the devenv toolchain.
- The plugin fixture adds a workspace member that exists only to prove the
  boundary. That is deliberate and is the shape the effort spec's plugin ABI
  seam describes.
