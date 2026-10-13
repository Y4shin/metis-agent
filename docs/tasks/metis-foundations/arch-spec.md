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
  (`edition = "2024"`, `max_width = 100`). The generated-tree post-step adds
  `normalize_comments=true` as a command-line config only, so the committed
  files stay as they are (see the `contract-pipeline` section).
- Generated code is mounted, never edited; handwritten behaviour lives outside
  the generated trees. Generated trees are not excluded from fmt, clippy, or
  test, they carry the measured allow set (`clippy::match_single_binding` is
  load-bearing today, `clippy::derivable_impls` stays dormant until the
  contract declares its first enum).
- Dependency posture: pin the pre-1.0 protocol pair exactly (`connectrpc` and
  `buffa` move together). Keep `plugin-contract` free of async runtimes and of
  serde in its C ABI surface.
- Prose and comments follow `AGENTS.md`: no em-dashes.
- Names fixed by this spec: crate `plugin-contract`, crate `plugin-fixture`,
  proto package `metis.v1`, service `DaemonService`, entry symbol
  `metis_plugin_entry_v1`, feature gate `authoring`.

---

## Landed ticket: `contract-pipeline`

Landed first in the effort (merge `adea161` on `task/contract-pipeline`); the
surfaces below are the measured ones. Builds the contract pipeline end to end:
one `.proto`, pinned BSR generation into committed Rust, the regeneration
command, and the CI gates.

Two file locations below are superseded by `contract-gate-hardening`, which
moves the buf configuration to the repository root: read that pending section
before copying a path out of this one.

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
  `Event` message, and is recorded then), `breaking` FILE. As landed, this
  file's `breaking` block governs nothing in CI: the `buf breaking` step runs
  from the repository root, where buf reads its built-in defaults, because from
  `contracts/` the `.git#branch=main` target cannot resolve at all (measured:
  exit 1, "does not appear to be a git repository"). The gate's category
  happens to agree with the declared one only because buf's default is FILE.
  `contract-gate-hardening` moves this configuration to a root `buf.yaml` and
  deletes this file.
- `contracts/buf.gen.yaml`: `version: v2`, `clean: true`, two BSR remote plugins
  pinned by tag:
  - `remote: buf.build/anthropics/buffa:v0.9.2`, out
    `../crates/api-contracts/generated/buffa`, opts `views=true`, `json=true`,
    `file_per_package=true`
  - `remote: buf.build/connectrpc/rust:v0.9.0`, out
    `../crates/api-contracts/generated/connect`, opts
    `buffa_module=crate::proto`, `file_per_package` (bare option, the plugin
    rejects `=true`; measured)

  As landed, generation runs from the repository root against this template
  file. `contract-gate-hardening` moves the file to the root with the same
  pins, options, and output trees, so later chains should take the location
  from that pending section.
- Committed generated trees:
  `crates/api-contracts/generated/buffa/metis.v1.rs` (messages and views) and
  `crates/api-contracts/generated/connect/metis.v1.rs` (service trait, register
  helper, generated client).
- `crates/api-contracts/src/lib.rs`: `pub mod connect; pub mod proto;` plus
  crate docs. `src/proto.rs` declares the nested `metis::v1` module tree and
  mounts the buffa tree with `include!` and
  `#![allow(clippy::derivable_impls)]`. `src/connect.rs` does the same for the
  connect tree with `#![allow(clippy::match_single_binding)]`. The mount point
  is `crate::proto`, which is exactly what `buffa_module` names. Both allows are
  present as planned, but only `clippy::match_single_binding` is load-bearing
  (removing it fails clippy with three errors); `clippy::derivable_impls` stays
  dormant until the contract declares its first enum, which `event-stream` adds
  as `EventKind`.
- `crates/api-contracts/Cargo.toml` dependencies: `connectrpc = "=0.9.1"` with
  `default-features = false, features = ["client", "server", "json"]`,
  `buffa = "=0.9.2"`, `serde = "1"`, `http = "1"`, `http-body = "1"`. The
  connect `json` feature stays on because the generated buffa types carry JSON
  impls and the spike measured that combination; whether the JSON codec is
  served to outside clients remains the map's fog.
- `scripts/regenerate-contracts.sh` with an optional `--check`: regenerate with
  `buf generate`, apply the rustfmt post-step to both generated trees, and in
  `--check` mode regenerate into a temporary directory, format it, and diff
  against the committed trees, exiting non-zero on any difference. The
  post-step is `rustfmt --edition 2024 --config-path rustfmt.toml --config
  normalize_comments=true`, one option beyond the repository config. Without
  it, buffa's multi-line `/** */` doc comments keep the item indentation,
  rustdoc reads the prose as an indented Markdown code block, and the doctests
  it collects cannot compile, so `cargo test` fails. The user confirmed
  `normalize_comments=true` as the precedent every later generated tree
  inherits.
- Generation and the drift check resolve the two pinned plugins from the BSR
  over the network on every run, so the gates depend on the network being
  reachable and on anonymous rate limits: `resource_exhausted: too many
  requests` was reproduced here and can fail a pull request for a non-code
  reason. CI pins `BUF_TOKEN` from `secrets.BUF_TOKEN` for the two BSR-backed
  steps (`contract-gate-hardening`).
- `contracts/README.md` documenting the one developer-facing regeneration
  command and what the CI gates assert.
- CI (`.github/workflows/ci.yml`) additions, all in the existing
  `devenv shell --` style: `actions/checkout` gains `fetch-depth: 0` (needed by
  `buf breaking`), then `buf lint` (from `contracts/`, the one buf command that
  reads the declared configuration),
  `buf breaking --against '.git#branch=main'` from the repository root (buf's
  built-in defaults, see this ticket's `buf.yaml` entry), and
  `bash scripts/regenerate-contracts.sh --check`. The two BSR-backed steps take
  `BUF_TOKEN` from `secrets.BUF_TOKEN`.
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
   exit 0 on the committed contract, and the CI job runs both. The breaking
   gate only has a comparison input once `main` itself carries the contract,
   which the landing merge established: on a working branch whose base predates
   the proto, every change to the declared contract is an addition on the
   comparison's input side, so the gate cannot fail retroactively there
   (measured: the CI command exits 0 with a breaking field rename in the
   working tree, while `--against '.git#branch=<branch>'` exits 100).

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
  surface is not. The mode that governs CI is the one in the root `buf.yaml`
  once `contract-gate-hardening` lands; until then it is buf's built-in
  default, which happens to be FILE. A later contract ticket may not weaken the
  breaking gate by editing the configuration surface alone, because the same
  file feeds `buf lint`. Regeneration keeps going through
  `scripts/regenerate-contracts.sh` after the move.

---

## Landed ticket: `plugin-abi-core`

Landed on `task/plugin-abi-core` (merge `8ff64fa`, on top of the landed
`contract-pipeline`). The ABI surface below matches the plan name for name; the
corrections landed in tooling and test support. Builds the plugin boundary
itself: the primitive ABI, the generated header, and a fixture plugin driven by
a harness.

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
  `crates/plugin-contract/include/metis_plugin_abi.h`. The `[export] include`
  list carries every ABI type, and an `exclude` list names the six
  `crate::payload` constants (`METIS_PAYLOAD_VERSION_MASK`,
  `METIS_PAYLOAD_CODEC_BIT`, `METIS_PAYLOAD_RESERVED_MASK`,
  `METIS_PAYLOAD_FORMAT_VERSION`, `METIS_CODEC_JSON`,
  `METIS_CODEC_MESSAGEPACK`), because cbindgen emits every public constant of
  the crate regardless of the include list: the first generated header leaked
  them. Any new public constant in `plugin-contract` must be excluded by name
  too.
- `scripts/regenerate-abi-header.sh` with an optional `--check`, mirroring the
  contracts script: regenerate the header from the crate, and in `--check` mode
  regenerate into a temporary file and diff, exiting non-zero on drift.
- `crates/plugin-fixture/`, one workspace member, test support:
  - `src/lib.rs` a `cdylib` plus `rlib` fixture implementing the v1 table with
    four capabilities: `tool.echo` (inline), `tool.pending` (pending, then a
    `progress` callback, then a completion through the host `complete`
    callback), `tool.cancellable` (suppresses its completion after a cancel),
    and `tool.panic`, plus an allocation ledger that reports `allocs`, `frees`,
    `live`, `violations`, `suppressed`. The `rlib` exists only so the sibling
    harness binary shares the payload shapes instead of restating the JSON; the
    loaded `cdylib` is unchanged and still exports exactly one `metis_*`
    symbol. The late-completion-after-cancel path is `tool.pending` itself,
    which never honours a cancel, so no separate `tool.late` name exists: the
    host's cancel succeeds, the plugin's completion still arrives, and the host
    drops it and still releases its buffer.
  - `src/bin/abi-harness.rs` the harness: it builds the fixture first with
    `env!("CARGO")` (`cargo build -p plugin-fixture --lib`), then loads the
    library placed next to `current_exe()`, prints one `[ok]`/`[fail]` line per
    check, prints a `[result] PASS`/`FAIL` line, and exits non-zero on any
    failure. Rebuilding from the harness is the spike's stale-library lesson
    encoded rather than documented.
- `Cargo.toml` and `devenv.nix` additions: workspace member
  `crates/plugin-fixture`; workspace dependencies for `libloading = "0.9"`,
  `serde = "1"` (with `derive`), `serde_json = "1"`, and the `cc`
  build-dependency for `plugin-contract`; `pkgs.rust-cbindgen` in `devenv.nix`.
  The attribute is `rust-cbindgen`, not `cbindgen`: the pinned nixpkgs has no
  `cbindgen` attribute, and `rust-cbindgen` installs the `cbindgen` binary at
  0.29.4, the version the mechanics spike measured.
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
  dependency graph, so cbindgen emits only constants (measured). The include
  list is not sufficient on its own: every public constant outside `crate::abi`
  needs a matching `exclude` entry, because cbindgen emits public constants
  regardless of the include list (measured).
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
   library. The pending capability is `tool.pending` and the cancel path is
   `tool.cancellable`.
3. **Envelope seam.** `crate::payload` round-trips a header, rejects a bad
   format version or set reserved bit with `InvalidArgument`, and rejects an
   unknown codec with `Unsupported`. Exercised as a unit test and across the
   boundary from the harness.
4. **Ownership seam.** The fixture ledger and the harness counter agree,
   `allocs == frees`, `live == 0`, `violations == 0`, including a completion the
   harness drops and still releases.
5. **Panic seam.** A panic inside an exported adapter and a panic inside a host
   callback each become `MetisStatus::Internal`, and the instance still serves
   the next call. The host-callback half is harness-owned rather than
   fixture-owned: the harness panics on purpose inside its own `complete`
   callback for one designated request, catches it in that frame, and records
   the request as `internal`. A panic inside a Rust `extern "C"` callback
   cannot be caught by the plugin that called it, because unwinding out of an
   `extern "C"` frame aborts, so only the callback's own frame can convert it.
   Every host must wrap every callback it installs the same way; that is an
   obligation on `plugin-runtime` (metis-plugins), not a fixture detail.
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
  reads descriptor payloads, not the table. The predicate is named here as the
  contract but is not exported today: this ticket landed `metis_read_prefix`
  only, and `plugin-compatibility` adds the helper in-crate, which is additive
  and changes no layout. A per-capability version gate check was written
  against the fixture and removed before landing, because the fixture
  advertises no version a host would refuse, so the check could not fail;
  gating is `plugin-compatibility`'s subject.
- `plugin-authoring-trait` lives in the same crate behind a feature gate named
  `authoring` and imports only `crate::abi` and `crate::payload`. It supplies
  `table::<P: Plugin + Default>()` returning a `const MetisPluginTableV1`, and
  the host adapter. The trait adapter always returns `Pending` (D15) while the
  ABI keeps its inline path for direct implementers. Every public item it adds
  is subject to the cbindgen rule above: a new public constant lands in the
  header unless it is excluded, so an item meant for Rust authors only must be
  excluded by name.
- `plugin-runtime` (metis-plugins) and `metis-plugin-sdk` consume these types;
  this effort hands them the ABI and the layout guarantees, not a loader.
  `plugin-runtime` must wrap every host callback it installs so a panic is
  converted inside the callback's own frame (seam 5).
- The fixture and the harness are test support. Nothing outside
  `crates/plugin-fixture` depends on them, and the fixture's payload types are
  deliberately not exported from `plugin-contract`.

---

## Pending ticket: `contract-gate-hardening`

Hardens the contract gates `contract-pipeline` landed: one configuration that
every buf command reads, and authentication for the two BSR-backed gates. It
supersedes the `contracts/buf.yaml` and `contracts/buf.gen.yaml` locations in
the `contract-pipeline` section above.

### Exports

- A root `buf.yaml` declaring the module at `contracts/proto`, `lint` STANDARD
  with still no exception, and `breaking` FILE. `contracts/buf.yaml` is deleted.
- A root `buf.gen.yaml` carrying the same `version: v2`, `clean: true`, two BSR
  plugin pins, options, and output trees that `contracts/buf.gen.yaml` carries
  today, so the committed trees under `crates/api-contracts/generated/` are
  reproduced byte for byte.
- `buf lint`, `buf breaking`, and generation all run from the repository root
  and read that one configuration; `buf lint` stops reading a directory-local
  configuration surface.
- CI supplies `BUF_TOKEN` from `secrets.BUF_TOKEN` to the steps that resolve
  plugins (generation and the drift check). An absent or empty secret must leave
  both gates working unauthenticated, and the form buf needs is measured rather
  than assumed, because an empty variable may not equal an unset one.
- `contracts/README.md` and `docs/testing.md` updated to the new locations, with
  the documented developer command unchanged or shorter.
- `scripts/regenerate-contracts.sh` keeps its behaviour, including the
  `normalize_comments=true` rustfmt post-step, and still exits 0 with `--check`
  on a clean tree.

### Existing abstractions to use

- `contracts/buf.yaml`, `contracts/buf.gen.yaml`, and
  `scripts/regenerate-contracts.sh` as landed: this ticket moves configuration,
  it does not rewrite the pipeline.
- `docs/testing.md` owns the run commands; `contracts/README.md` owns the
  developer-facing regeneration command.
- CI keeps its existing `devenv shell --` style steps and adds the token only to
  the steps that resolve plugins.

### Do NOT reimplement

- Do not add a second regeneration script or a second generation path.
- Do not change the two plugin pins, their options, or the committed tree
  layout: the drift check must still reproduce the committed output.
- Do not weaken the root `lint` policy or the `breaking` category to make the
  configuration move easier.
- Do not exclude the generated trees from `cargo fmt`, `clippy`, or `test`.

### Seams

1. **Configuration seam.** One deliberate edit to the root `breaking` block
   changes the verdict of `buf breaking` run from the repository root, which
   proves the gate reads that file. `buf lint` reads the same file.
2. **Token seam.** The steps that resolve plugins receive `BUF_TOKEN` from
   `secrets.BUF_TOKEN`, and both BSR-backed gates still work when the secret is
   absent or empty.
3. **Reproduction seam.** `bash scripts/regenerate-contracts.sh --check` exits 0
   on a clean tree from the new root layout.

Test types: the two buf commands and the deliberate-edit probe for seams 1 and
2; the script for seam 3. No new Rust test.

### Interface contract

- After this ticket the repository has exactly one buf configuration surface, at
  the root, and every buf command reads it. A later contract ticket that changes
  lint or breaking policy edits that one file, so a declared policy and the
  verdict of the gate cannot drift apart again.
- The Rust gates, the `plugin-abi-core` header drift check, and the ABI harness
  are unaffected: this ticket changes where buf reads its configuration and how
  CI authenticates, not what is generated.

---

## Downstream interface contracts (outline)

These tickets are not pending yet. Their sections are added when they reach the
frontier; the commitments below are what the two landed tickets must already
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
  regenerates through `scripts/regenerate-contracts.sh`. `EventKind` is the
  contract's first enum, which is what makes the buffa mount's dormant
  `clippy::derivable_impls` allow load-bearing.
- `windows-named-pipe`, `tcp-listener`, `tls-and-client-trust`,
  `reverse-proxy-mode`: extend the same `transport` module; no second service
  implementation.
- `local-identity-and-acl`, `pairing-and-tokens`, `remote-pairing`: add request
  metadata and RPCs under `metis.v1`; credentials travel as
  `Authorization` metadata on every call, including the stream.
- `plugin-authoring-trait`: the `authoring` feature in `plugin-contract`, with
  the settled cancellation token and always-pending adapter, plus the
  descriptor/dispatch consistency check and the lifecycle and `emit_event` gaps
  the authoring spike recorded. It extends the crate `plugin-abi-core` landed,
  so its new public items must respect the cbindgen `exclude` rule.
- `plugin-compatibility`: prefix reading and per-capability version gating on
  top of the unchanged layout. It adds the `size`-based presence predicate
  in-crate, since `plugin-contract` exports `metis_read_prefix` but not the
  predicate.

## Architecture notes

- The two landed tickets were independent (no `blocked_by` edge between them),
  so they ran in level 0 sequentially on the shared repo cwd, in the order
  `tw_dependency_levels` returned, and `plugin-abi-core` landed on
  `contract-pipeline`'s landing branch.
- `contract-gate-hardening` is `blocked_by` `contract-pipeline` and edits the
  pipeline that ticket landed, so it lands on the same chain.
- Both tickets touch the workspace root (`Cargo.toml`, `devenv.nix`,
  `.github/workflows/ci.yml`) and `crates/`, which is why they cannot run in
  parallel and why the second chain lands on the first chain's landing branch.
- `contract-pipeline` adds the first committed generated tree in the repository,
  so the fmt and clippy allowances it establishes, and the
  `normalize_comments=true` rustfmt post-step, are the precedent every later
  generated tree follows.
- `plugin-abi-core` is the first place the repository compiles C. The `cc`
  build-dependency plus `-std=c11` is the measured minimum; the C compiler is
  already in the devenv toolchain.
- The plugin fixture adds a workspace member that exists only to prove the
  boundary. That is deliberate and is the shape the effort spec's plugin ABI
  seam describes. The harness is the owner of the host-callback panic probe and
  of the library rebuild, so no consumer of the fixture can load a stale or
  unwrapped library.
