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
- State decisions: `docs/adr/0003-daemon-state-in-sqlite.md`
- Remote transport decisions: `docs/adr/0004-no-native-tls-reverse-proxy.md`
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

The buf configuration this ticket first landed under `contracts/` now lives at
the repository root: `contract-gate-hardening` moved it there, and the paths
below are the current ones. `buf lint`, `buf breaking`, and `buf generate` all
run from the repository root and read the root `buf.yaml`.

### Exports

- `contracts/proto/metis/v1/daemon.proto`: `syntax = "proto3"`, package
  `metis.v1`, `service DaemonService` carrying one ordinary RPC for now:
  `rpc Echo(EchoRequest) returns (EchoResponse)`. Messages:
  `EchoRequest { string message = 1; }` and
  `EchoResponse { string message = 1; string received_at = 2; }`, where
  `received_at` is an RFC 3339 string, mirroring the spike contract. A later
  ticket (`event-stream`) appends `StreamEvents` and its messages; this ticket
  reserves the shape without declaring them.
- Root `buf.yaml`, moved there by `contract-gate-hardening` from the
  `contracts/buf.yaml` this ticket landed (the old file is gone): module at
  `contracts/proto`, so `buf lint` covers the contract alone, `lint` STANDARD
  with no exception yet (the `RPC_RESPONSE_STANDARD_NAME` exception arrives with
  the streaming `Event` message, and is recorded then), `breaking` FILE. Every
  buf command runs from the repository root and reads this one file, so the
  declared `breaking` category is the one the gate applies. `buf breaking`
  requires `--against-config buf.yaml` from the root: without it the working
  side resolves the module at `contracts/proto` while the branch side resolves
  whole-tree paths, and the command exits 100 with spurious deleted-file
  reports (measured).
- Root `buf.gen.yaml`, the same template `contract-gate-hardening` moved from
  `contracts/buf.gen.yaml` with the same pins, options, and output trees:
  `version: v2`, `clean: true`, two BSR remote plugins pinned by tag:
  - `remote: buf.build/anthropics/buffa:v0.9.2`, out
    `crates/api-contracts/generated/buffa`, opts `views=true`, `json=true`,
    `file_per_package=true`
  - `remote: buf.build/connectrpc/rust:v0.9.0`, out
    `crates/api-contracts/generated/connect`, opts
    `buffa_module=crate::proto`, `file_per_package` (bare option, the plugin
    rejects `=true`; measured)

  Generation runs from the repository root, where the template and the module
  both live, and reproduces the committed trees byte for byte.
- Committed generated trees:
  `crates/api-contracts/generated/buffa/metis.v1.rs` (messages and views) and
  `crates/api-contracts/generated/connect/metis.v1.rs` (service trait, register
  helper, generated client).
- `crates/api-contracts/src/lib.rs`: `pub mod connect; pub mod proto;` plus
  crate docs, which name the root `buf.gen.yaml` as the plugin template (the
  doc comment used to name `contracts/buf.gen.yaml`; `contract-gate-hardening`
  updated it, comment only). `src/proto.rs` declares the nested `metis::v1`
  module tree and mounts the buffa tree with `include!` and
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
  over the network on every run, so that step depends on the network being
  reachable and on anonymous rate limits: `resource_exhausted: too many
  requests` was reproduced here and can fail a pull request for a non-code
  reason. `contract-gate-hardening` gives the one plugin-resolving CI step
  `BUF_TOKEN` from `secrets.BUF_TOKEN`, and unsets the variable when the secret
  is empty. `buf lint` and `buf breaking` resolve no plugins, so they carry no
  token.
- `contracts/README.md` documenting the one developer-facing regeneration
  command and what the CI gates assert.
- CI (`.github/workflows/ci.yml`) additions, all in the existing
  `devenv shell --` style: `actions/checkout` gains `fetch-depth: 0` (needed by
  `buf breaking`), then `buf lint`,
  `buf breaking --against '.git#branch=main' --against-config buf.yaml`, and
  `bash scripts/regenerate-contracts.sh --check`, all from the repository root.
  `contract-gate-hardening` added the `--against-config` flag and gave the
  regeneration check step `BUF_TOKEN` from `secrets.BUF_TOKEN`, unset when the
  secret is empty; the other two buf steps resolve no plugins and carry no
  token.
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
4. **Gate seam.** `buf lint` and
   `buf breaking --against '.git#branch=main' --against-config buf.yaml`, both
   run from the repository root, exit 0 on the committed contract, and the CI
   job runs both. The breaking gate only has a comparison input once `main`
   itself carries the contract,
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
- The generated client is what `client` wraps; `local-transport-rpc-skeleton`
  builds the local transport inside the wrapper's own constructors rather than
  taking one from the caller, and its landed section records that. This ticket
  does not export the transport module.
- Any server implementation of `DaemonService` must carry
  `#[allow(refining_impl_trait)]` on the `impl` block, because the generated
  trait uses an explicit RPITIT signature and the workspace denies warnings.
  This applies to `local-transport-rpc-skeleton` and to the daemon effort.
- `buf breaking` in FILE mode is the compatibility gate for later contract
  additions: adding RPCs and messages is allowed, changing or removing the echo
  surface is not. The mode that governs CI is the `breaking` block of the root
  `buf.yaml`, which the gate reads on both sides of the comparison because it
  runs with `--against-config buf.yaml`. A later contract ticket may not weaken
  the breaking gate by editing the configuration surface alone, because the
  same file feeds `buf lint`. Regeneration keeps going through
  `scripts/regenerate-contracts.sh`.

---

## Landed ticket: `plugin-abi-core` (superseded in part)

Superseded in part by `plugin-contract-v1`, which revised both tables, deleted
`invoke`, `MetisInvokeFn` and the host `complete` callback, and removed the
fixture ledger's `suppressed` field. What follows is the record of what this
ticket landed, including the four-capability fixture, the inline `invoke`
result, and the 72-byte table that revision replaced; the current surface is in
the landed `plugin-contract-v1` section.

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

## Landed ticket: `contract-gate-hardening`

Landed on `task/contract-gate-hardening` as the merge commit `3612e0f`, parents
`08a5be8` (the landing branch, which already carried the first attempt's two
checkpoints behind the preservation merge) and `81c6207` (the working branch);
the landing tree is byte-identical to `81c6207` and the working branch was
deleted after the merge. Hardens the contract gates `contract-pipeline` landed:
one configuration that every buf command reads, and authentication for the one
plugin-resolving gate. It moved the `contracts/buf.yaml` and
`contracts/buf.gen.yaml` locations named in the section above to the repository
root.

### Exports

- Root `buf.yaml` declaring the module at `contracts/proto`, `lint` STANDARD
  with still no exception, and `breaking` FILE; `contracts/buf.yaml` is gone.
  `buf ls-files` from the repository root lists exactly
  `contracts/proto/metis/v1/daemon.proto`, so the module comes from the root
  file and lint covers the contract alone.
- Root `buf.gen.yaml` carrying the same `version: v2`, `clean: true`, two BSR
  plugin pins, options, and output trees that `contracts/buf.gen.yaml` carried,
  so the committed trees under `crates/api-contracts/generated/` are reproduced
  byte for byte.
- `buf lint`, `buf breaking`, and generation all run from the repository root
  and read that one configuration; `buf lint` no longer reads a directory-local
  configuration surface.
- `buf breaking --against '.git#branch=main' --against-config buf.yaml`: the
  flag is required, not optional. Without it the working side resolves the
  module at `contracts/proto` while the branch side resolves whole-tree paths,
  and the command exits 100 with spurious
  `Previously present file "contracts/proto/metis/v1/daemon.proto" was deleted`
  reports (measured; with the flag the same command exits 0). CI,
  `contracts/README.md`, and `docs/testing.md` all carry the flag, and a later
  contract ticket that copies the command line keeps it.
- CI supplies `BUF_TOKEN` from `secrets.BUF_TOKEN` to exactly one step, the
  plugin-resolving `Contract regeneration check`, and runs
  `if [ -z "${BUF_TOKEN:-}" ]; then unset BUF_TOKEN; fi` before it, so buf is
  either given a real token or no variable at all. Measured on buf 1.73.0: both
  an unset and an empty `BUF_TOKEN` reach the BSR anonymously, a non-empty token
  the BSR cannot verify is refused before any request is sent, and `buf lint`
  and `buf breaking` resolve no plugins and pass even under a token buf refuses.
- `contracts/README.md` and `docs/testing.md` updated to the new locations, with
  the documented developer command unchanged.
- `scripts/regenerate-contracts.sh` keeps its behaviour, including the
  `normalize_comments=true` rustfmt post-step, and still exits 0 with `--check`
  on a clean tree; it now `cd`s to the repository root and rewrites the scratch
  template path, and nothing else changed.
- `crates/api-contracts/src/lib.rs`: the crate doc comment that pointed
  generated code at `contracts/buf.gen.yaml` now names the root `buf.gen.yaml`
  (comment only, no API change).
- The ticket's acceptance probe lives at `scripts/probe-contract-gates.sh`, one
  section per seam below, restoring every file it touches through a `trap` and
  reporting a rate-limited probe as skipped rather than as a pass. It was
  written inside the ticket's transient `.work/` channel and moved to `scripts/`
  at finalization, so the evidence survives without putting scaffolding in the
  ticket's uncertainty channel. `docs/testing.md` documents it as an on-demand
  probe; it is not wired into CI.

### Existing abstractions to use

- `contracts/buf.yaml`, `contracts/buf.gen.yaml`, and
  `scripts/regenerate-contracts.sh` as landed: this ticket moved configuration,
  it did not rewrite the pipeline. The two plugin pins, their options, the
  output tree layout, `clean: true`, and the rustfmt post-step are unchanged,
  and no second generation path was added.
- `docs/testing.md` owns the run commands; `contracts/README.md` owns the
  developer-facing regeneration command. Both were updated in place.
- CI kept its existing `devenv shell --` style steps and added the token to the
  one step that resolves plugins.

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
   proves the gate reads that file. `buf lint` reads the same file. Exercised by
   `scripts/probe-contract-gates.sh`, which restores the edit.
2. **Token seam.** The one step that resolves plugins receives `BUF_TOKEN` from
   `secrets.BUF_TOKEN`, and the gate still works when the secret is absent or
   empty, because the empty form is unset before buf runs.
3. **Reproduction seam.** `bash scripts/regenerate-contracts.sh --check` exits 0
   on a clean tree from the new root layout.

Test types: the two buf commands and the deliberate-edit probe for seams 1 and
2; the script for seam 3. No new Rust test. All of it lives in
`scripts/probe-contract-gates.sh`, which runs on demand: `docs/testing.md`
documents it, CI does not run it, and the network-only probes are skipped under
anonymous BSR rate limiting, so a rate-limited run exercises the drift logic
only through the offline stub-backed twin and the clean-tree passes.

### Interface contract

- The repository has exactly one buf configuration surface, at the root, and
  every buf command reads it. A later contract ticket that changes lint or
  breaking policy edits that one file, so a declared policy and the verdict of
  the gate cannot drift apart again.
- Any later ticket that runs `buf breaking` from the root must pass
  `--against-config buf.yaml`. Without it the two sides of the comparison
  resolve different path roots and the command fails on an unchanged contract.
- The Rust gates, the `plugin-abi-core` header drift check, and the ABI harness
  are unaffected: this ticket changed where buf reads its configuration and how
  CI authenticates, not what is generated.

---

## Landed ticket: `local-transport-rpc-skeleton`

Landed on `task/local-transport-rpc-skeleton` as the merge commit `d886567`,
parents `c4daca7` (the landing branch, which was level with `main`) and
`3db4066` (the working branch `ticket/local-transport-rpc-skeleton`); the landing
tree is byte-identical to `3db4066` and the working branch was deleted after the
merge. Because the landed tree is the tree the verify worker tested, no gate was
re-run at landing. Builds the first complete path between two processes: the
handwritten transport, the client wrapper, a minimal service, an integration
test, and an example, so the contract and its generated stubs are proven over a
real local socket instead of a dummy implementation.

The surfaces below are the measured ones. Four divergences from the plan are
folded in: `serve_stream` takes the router through `Into<Arc<Router>>`, two
additive public functions (`resolve_endpoint()` and `Client::with_endpoint()`),
a `Client` with no public transport injection point, and the fixed endpoint
literals with the `/tmp` conflict they open against `local-identity-and-acl`.

### Exports

- `crates/api-contracts/src/transport.rs`, mounted as `pub mod transport;` in
  `src/lib.rs` (crate docs updated to name it): the one handwritten transport in
  the repository, the module the skeleton, `client`, and the daemon effort
  share.
  - `pub async fn serve_stream<S>(stream: S, router: impl Into<Arc<Router>>) ->
    io::Result<()>`: the accept-loop body, generic over any asynchronous
    bidirectional stream. It wraps the stream with `TokioIo`, builds the service
    with `ConnectRpcService::from_arc(router.into())`, and drives a `hyper_util`
    auto (HTTP/1.1 or HTTP/2) connection. The router arrives through
    `Into<Arc<Router>>` rather than as a bare `Router`, because
    `connectrpc::Router` is not `Clone` in 0.9.1 and an accept loop cannot hand
    the same router to each accepted stream; a one-shot caller passing a bare
    `Router` still compiles, which is the shape the duplex test uses. This is the
    function `windows-named-pipe`, `tcp-listener`, and `reverse-proxy-mode`
    reuse, so a later transport adds a listener front-end and no second serve
    loop, building the router once and cloning the `Arc` per accepted stream.
    connectrpc 0.9.1 has no `Server::serve_connection`, so this push path is the
    measured route.
  - `pub async fn serve_uds(path: &Path, router: Router) -> io::Result<()>`: the
    Unix front-end. It removes a stale socket file, binds
    `tokio::net::UnixListener`, accepts in a loop, and hands each stream to
    `serve_stream`. It keeps the bare `Router` parameter and wraps it in an `Arc`
    itself, so one router serves the whole accept loop. The socket path is the
    seam the integration test drives. It has no shutdown path that unlinks its
    socket, so a stopped listener leaves the socket file behind (see the
    residual limitation in the interface contract below).
  - `pub fn uds_transport(path: impl Into<PathBuf>) -> connectrpc::client::Http2Connection`:
    the connector front-end, built with `Http2Connection::lazy_with_connector`
    over a `tower::service_fn` that opens a `UnixStream`, so a later transport
    supplies its own connector against the same generated client shape.
  - `pub fn local_authority() -> http::Uri`: `http://localhost`, the
    `:authority` the local transport uses (the server does not validate it).
- `crates/api-contracts/Cargo.toml`: the transport's dependencies at the spike's
  measured set and features, declared in the root `Cargo.toml` workspace
  dependencies: `tokio = "1"`,
  `tower = { version = "0.5", features = ["util"] }`,
  `hyper = { version = "1", features = ["server", "http1", "http2"] }`, and
  `hyper-util = { version = "0.1", features = ["tokio", "server-auto", "service"] }`.
  The `futures` crate is not needed yet: the echo path carries no stream. The
  pinned `connectrpc` also moved into the root `[workspace.dependencies]` with
  the same pin and features, so the pin is no longer duplicated now that a
  second crate names it and the effective dependency graph is unchanged.
  `tokio`'s features are narrowed to `io-util`, `macros`, `net`, `rt`, and
  `time`, the features the listener, the connector, the skeleton wait, and the
  tests actually name; `rt-multi-thread` is deliberately absent. `client` gained
  `tokio` as a normal dependency plus `http`, `hyper-util`, and `tower` as
  dev-dependencies. `crates/api-contracts/Cargo.toml` says
  `connectrpc.workspace = true`; `docs/testing.md` and `docs/dev-env.md` carry
  the local-transport run commands.
- `crates/client/src/lib.rs`, on the crate `api-contracts` is already wired
  into: the wrapper that hides the transport.
  - `pub struct Client`, holding the generated `DaemonServiceClient` over the
    transport it builds itself. The field is private and there is no public
    transport injection point: the transport is constructed inside `local()` and
    `with_endpoint()`, so a later transport extends `client` with its own
    constructor rather than injecting from outside. Its API names no transport
    and no generated stub type: a caller constructs it and calls a method.
  - `pub async fn local() -> Result<Client, ClientError>` and
    `pub async fn Client::with_endpoint(endpoint: impl Into<PathBuf>) ->
    Result<Self, ClientError>`, both `#[cfg(unix)]`: `local()` resolves the
    endpoint and delegates to `with_endpoint`, which builds the local transport
    and connects. `with_endpoint` is the explicit endpoint constructor the plan
    named but did not export; it is async and returns a `Result` so it can
    become an eager connect without an API break. On non-Unix there is no local
    constructor until `windows-named-pipe` lands.
  - `pub async fn echo(&self, message: &str) -> Result<String, ClientError>`,
    performing the echo RPC over the local transport and returning the
    response's `message`.
  - `pub fn default_endpoint() -> PathBuf` and
    `pub fn resolve_endpoint() -> PathBuf`: the platform default, which is
    `/tmp/metis-daemon.sock` on Unix, and the single resolver, which returns the
    environment override when it is set and non-empty and the default otherwise.
    The override variable is `METIS_ENDPOINT`, an empty value is treated as
    unset, and `const ENDPOINT_ENV` is private to the crate. `resolve_endpoint()`
    is additive beyond the plan's export list; it is what makes the wrapper and
    the example honour the override identically. Both literals are fixed here,
    and the daemon effort binds the same default. Because the variable name is
    private, the daemon effort restates the literal `METIS_ENDPOINT` unless the
    constant is exported (the map carries that obligation as fog, together with
    the `/tmp` conflict below). Superseded by `local-identity-and-acl`: both
    functions now return `Result<PathBuf, EndpointError>`, and the default is
    `$XDG_RUNTIME_DIR/metis/daemon.sock`, else `$TMPDIR/metis/daemon.sock`, else
    `EndpointError::NoPrivateBase`.
  - `pub enum ClientError`, the wrapper's typed error over the transport and
    protocol failures: `#[non_exhaustive]`, with the `Call(ConnectError)`
    variant, `Display`, `Error::source`, and `From<ConnectError>`. The
    credential-specific variants arrive with `local-identity-and-acl`.
  - The skeleton service in `crates/client/src/skeleton.rs`: the minimal
    `DaemonService` implementation the test and the example drive, carrying
    `#[allow(refining_impl_trait)]`, echoing the request, and stamping
    `received_at` as an RFC 3339 UTC string through a hand-written days-to-civil
    routine (no date dependency). It is test support rather than product API and
    lives beside them in `crates/client` as one shared implementation, but it
    landed as an unconditional `pub mod skeleton`, so it is reachable through
    `client`'s public API and is the only reason `tokio` is a normal dependency
    of `client`; the spec's intent is not compiler-enforced, and feature-gating
    it until the daemon effort removes it is an open, non-blocking item. The
    module also carries the additive helpers `router()` (the router the
    transport serves) and `wait_for_socket()` (a bounded poll before
    connecting). This effort owns `client` and not `daemon`, so the daemon
    effort replaces this service and keeps the transport module.
- `crates/client/tests/echo.rs`, four tests: the duplex loop seam, the served
  path through the wrapper on a temporary socket directory with a `Drop` cleanup
  guard, the platform-default literal, and the environment override plus
  `Client::local()`. `crates/client/examples/echo.rs` serves the same skeleton on
  the resolved endpoint, makes the same call, prints `[echo]` and
  `[result] PASS`, and exits non-zero on failure; it pins
  `#[tokio::main(flavor = "current_thread")]` because `rt-multi-thread` is
  deliberately absent from the narrowed workspace `tokio` features, and its
  non-Unix `main` prints a FAIL line and exits 1, so the example is honest on a
  platform whose front-end has not landed.
- CI: no new step. The socket test joins the existing `cargo test` step on Linux,
  and the existing all-targets gates already compile the example.

### Existing abstractions to use

- `spikes/connectrpc/crates/wire/src/lib.rs`: the measured accept loop (the
  `ConnectRpcService` plus `hyper_util` auto builder shape, landed here with
  `ConnectRpcService::from_arc` rather than the spike's `::new` because the
  0.9.1 `Router` is not `Clone`; that is the one deliberate difference from the
  spike), `Http2Connection::lazy_with_connector`, `local_authority()`, and the
  compile-only Windows named-pipe pair the later transport tickets extend. Port
  the shape, not the spike paths.
- `spikes/connectrpc/crates/wire/src/bin/spike.rs`: the serve-then-call driver
  shape, including waiting for the socket before connecting and printing one
  result line.
- The landed contract surfaces: `DaemonService`, `DaemonServiceExt::register`,
  `DaemonServiceClient`, and `DAEMON_SERVICE_SERVICE_NAME`, already exercised by
  `crates/api-contracts/tests/contract.rs`.
- `crates/api-contracts/src/{lib,proto,connect}.rs`: the generated trees stay
  mounted and untouched, and the transport is a sibling handwritten module.
- `docs/testing.md`: the run commands and the CI convention; the socket test
  joins the existing test step rather than a second workflow.

### Do NOT reimplement

- Do not hand-write messages, the client stub, or a register helper, and do not
  define a second echo request or response type: the generated `metis.v1` types
  are the wire types.
- Do not add a second accept loop, a second connector, or a wrapper around
  `connectrpc::Server`: `serve_stream` and `uds_transport` are the one
  implementation every later transport extends.
- Do not put handwritten behaviour in `generated/`, and do not edit generated
  code to make the transport fit.
- Do not grow a product daemon surface: the skeleton service exists to prove the
  path, and the configurable, ACL-configured listener belongs to the daemon
  effort.
- Do not pin `tokio`, `hyper`, `hyper-util`, or `tower` exactly: only the pre-1.0
  protocol pair is pinned exactly.
- Do not let `client` depend on Tauri or a terminal UI crate, and do not expose
  the transport, `Http2Connection`, or a generated stub type in the wrapper's
  public API.
- Do not mock the transport in the served test or point it at a fixed socket
  path: the test binds its own temporary directory and removes it.

### Seams

1. **Loop seam.** `serve_stream` serves a router over any asynchronous
   bidirectional stream: a test drives a `tokio::io::duplex` pair through the
   loop and the connector shape, which is what proves no Unix-socket assumption
   leaked into it. The socket test below drives the real path.
2. **Served path seam.** Against a real socket: the service serves on a
   temporary path, the wrapper calls echo through it, the response carries the
   echoed message and a `received_at` stamp, and the test removes the socket
   directory through a `Drop` guard, so cleanup happens on the success path and
   on a panic alike.
3. **Endpoint seam.** `default_endpoint()` yields the platform default and
   `resolve_endpoint()` applies the override, which redirects the connection:
   the test sets the variable to its temporary socket, asserts the call lands
   there through `Client::local()`, and restores the previous value on drop. The
   test mutates the process environment with `unsafe { std::env::set_var }`
   (edition 2024), which is safe today only because it is the sole reader and
   writer of `METIS_ENDPOINT` in that test binary.
4. **Wrapper seam.** A caller that names no transport gets the echo result: the
   test constructs the client through the local constructor and calls `echo`,
   with no `uds_transport`, `Http2Connection`, or generated stub type anywhere in
   the test's path.
5. **Example seam.** `cargo run -p client --example echo` serves the skeleton on
   the resolved endpoint, performs the same call, prints `[echo]` and
   `[result] PASS`, and exits non-zero on failure (a non-Unix build prints a
   FAIL line and exits 1), so the path is runnable by hand as well as asserted.

Test types: `crates/client/tests/echo.rs` for seams 1, 2, 3, and 4, run by the
existing `cargo test` step on Linux; the example binary for seam 5, compiled by
the existing all-targets gates.

### Interface contract

- The transport module is the only place that knows how to serve and connect a
  local stream. `client` calls it, and `windows-named-pipe`, `tcp-listener`,
  and `reverse-proxy-mode` extend it with new bind and
  connector front-ends over the same `serve_stream` and no second service
  implementation. `serve_stream` takes the router through `Into<Arc<Router>>`,
  so each of those front-ends builds the router once and clones the `Arc` per
  accepted stream; a one-shot caller still passes a bare `Router`.
- The wrapper is the only UI-independent way to reach the daemon: its API names
  no transport, a later transport changes no caller's code, and the crate's
  manifest gains the runtime and transport crates only, never Tauri or a
  terminal UI crate. `Client` holds its transport privately with no public
  injection point, so every new transport extends `client` with its own
  constructor rather than being supplied from outside; on non-Unix there is no
  local constructor until `windows-named-pipe` lands, because `local()` and
  `with_endpoint()` are `#[cfg(unix)]`.
- The skeleton service is test support. The daemon effort replaces it with the
  real service, keeps the transport module, and carries the same
  `#[allow(refining_impl_trait)]` on every `DaemonService` implementation that
  the landed contract requires.
- This ticket serves the echo RPC only. The event stream, its snapshot, and the
  cursor arrive with `event-stream` over this same transport, which is what
  leaves the walking skeleton's stream half to that ticket.
- `api-contracts` gains async transport dependencies; `plugin-contract` stays
  free of async runtimes. The transport compiles on Unix, and the Windows and
  TCP front-ends arrive with their own tickets as the only per-platform code.
- The endpoint contract is one platform-derived default plus one environment
  override, resolved identically by the wrapper and the example through
  `resolve_endpoint()`. The fixed literals are `default_endpoint()` =
  `/tmp/metis-daemon.sock` on Unix and the override variable `METIS_ENDPOINT`,
  with an empty value treated as unset. The daemon effort binds the same default
  and restates the variable name, because `const ENDPOINT_ENV` is private and
  exporting it would remove that drift risk. A later transport reuses the rule
  (a TCP/TLS endpoint is a different front-end, not a different resolution
  rule). Superseded in part by `local-identity-and-acl`: the `/tmp` literal is
  gone, `default_endpoint()` and `resolve_endpoint()` return a `Result`, and the
  default resolves `$XDG_RUNTIME_DIR/metis/daemon.sock`, else
  `$TMPDIR/metis/daemon.sock`, else `EndpointError::NoPrivateBase`, because
  `/tmp` cannot carry the ACL.
- Open conflict for `local-identity-and-acl`, recorded rather than resolved
  here: `/tmp` is world-writable and sticky, so the socket path cannot satisfy
  that ticket's requirement that the daemon own the socket's directory with a
  configured `uid:gid` and mode 700 or 770, and `serve_uds` unlinks whatever
  sits at the path before binding, so another local user can pre-create the path
  between daemon starts. That ticket must revisit the default (for example a
  per-user runtime directory such as `$XDG_RUNTIME_DIR/metis/daemon.sock` with a
  documented fallback) and decide whether its ACL tests set the
  `METIS_ENDPOINT` override. The map carries the same item as fog. Closed by
  `local-identity-and-acl`, which took the suggested rule: the default is
  `$XDG_RUNTIME_DIR/metis/daemon.sock`, then `$TMPDIR/metis/daemon.sock`, then a
  typed `EndpointError::NoPrivateBase` refusal; its ACL test serves its own
  temporary socket base; and a pre-created path that is a symlink, is not a
  directory, is owned by another user, or is wider than the rule is now refused
  rather than reused.
- Residual limitation recorded rather than hidden: `serve_uds` has no shutdown
  path that unlinks its socket, so a stopped listener leaves the socket file
  behind. The integration test removes its temporary directory through a `Drop`
  guard; the example leaves the file on its endpoint.

---

## Landed ticket: `plugin-authoring-trait` (superseded in part)

Superseded in part by `plugin-contract-v1`, and completely in the places this
section names as the surface: the four-arm vocabulary, the four associated
`Shape` types, the `PluginError`-to-completion rule, and the crate's own
executor (`authoring/executor.rs`, with its bounded pool and its thread-per-call
fallback) are deleted, along with the host-side pending-slot machinery in
`authoring/host_adapter.rs`. One capability-name-routed `call`, a boxed-future
helper, and a host that polls are what replaced them, and
`crates/plugin-contract/tests/authoring.rs` was rewritten with them. The section
is kept as the record of how the surface was first built, so a later reader can
see what the revision removed and why. Everything below the trait that is still
true is named in the landed `plugin-contract-v1` section.

It builds the Rust author surface
on top of the landed primitives: the trait, both adapters, the
cancelled-completion rule, the lifecycle and event routes, and the
descriptor/dispatch check, with the fixture rewritten through the trait and
still driven by the same harness.

### Exports

- `crates/plugin-contract/Cargo.toml`: a `[features]` table with `authoring`, off
  by default, carrying the feature's optional `serde` (with `derive`) and
  `serde_json` dependencies. With the feature off the crate keeps the empty
  dependency list `plugin-abi-core` measured, which is what lets `plugin-runtime`
  depend on the primitives and the payload envelope alone.
- `crates/plugin-contract/src/authoring/`, behind `#[cfg(feature = "authoring")]`
  and re-exported from `src/lib.rs` under the same gate:
  - `Plugin`: `Default + Send + Sync + 'static` (the bounds the spike measured,
    because the adapter moves every capability future onto a worker), one async
    method per capability taking the deserialised request and the per-request
    context, the capability declaration the adapter lists and dispatches on, an
    instance lifecycle pair, and a cancellation method. The capability methods
    are parameterised by the plugin's own serde request and response types, in
    the shape the fixture's `payload` module already models, so no payload type
    enters the crate.
  - `Call`, the per-request context: the ABI request id, the cancellation token
    (D15) with the `is_cancelled()` poll convenience kept, and the host routes
    (`progress`, `emit_event`, `log`), so an author never sees a `MetisBuffer`,
    a payload header, or a callback pointer.
  - `PluginError`: the frozen status space minus `Ok` and `Pending`, with
    `InvalidArgument`, `NotFound`, `PermissionDenied`, `Unsupported`, and
    `Internal` carrying a detail string and `Cancelled` carrying control flow,
    mapped onto `MetisStatus` inside the adapter.
  - `pub const fn table::<P: Plugin + Default>() -> MetisPluginTableV1`, the
    export adapter: the whole plugin side of the translation as one `const fn`
    that the plugin instantiates into a static. That is the spike's 6-line
    wiring, and the reason no macro is justified at this size.
  - The host adapter: `MetisHostApiV1` to an awaitable call, the shape the
    spike's consumer used, so a host registers its pending record before calling
    and awaits a completion the callback resolves.
  - The crate's own minimal executor (`block_on` and `spawn`): one worker thread
    per pending call with a `park`/`unpark` waker, which is what lets `invoke`
    return `Pending` with no async runtime in the dependency list.
- The four behaviours the spike left open, added on top of its shape:
  - a lifecycle pair, `started` and `stopped`, run by the adapter from
    `create_instance` and `shutdown`. `started` can fail with a `PluginError`
    and that status is what `create_instance` returns, so `Default` construction
    stays infallible while initialisation can still fail loudly.
  - an `emit_event` route through the author surface: a topic and a payload go
    out, the D4 envelope and a plugin-owned buffer are built inside the adapter,
    the host `emit_event` callback is called, and the receiver releases the
    buffer. The spike emitted zero events, so this is the missing half of the
    host table's contract.
  - the descriptor/dispatch consistency check: the declared descriptors and the
    dispatch arms are separate declarations, and `create_instance` fails with a
    non-`Ok` status and a log line naming the mismatch when they do not line up,
    so a missing arm cannot survive to a call-time `NotFound`. The test drives
    the failure.
  - the cancelled-completion rule as documented contract: a capability that
    returns `Cancelled` after a successful cancel suppresses its `complete`
    callback (the fixture's `suppressed` ledger counts it, and no buffer is
    allocated to release), while `Cancelled` returned without a successful
    cancel still completes, so the host is never parked on a completion that
    will not arrive. D12's other half stays the host's: a completion that races
    the cancel is dropped and its payload released.
- `crates/plugin-fixture/src/lib.rs`: the fixture rewritten through the trait,
  with `plugin-contract`'s `authoring` feature on, keeping the four capability
  names (`tool.echo`, `tool.pending`, `tool.cancellable`, `tool.panic`), the
  allocation ledger, and the single exported `metis_*` symbol.
- `crates/plugin-fixture/src/bin/abi-harness.rs`: the check list changes where
  the always-pending shape moves an expectation, and the harness keeps loading
  the real library. The `[ok]`/`[fail]` pattern, the `[result] PASS`/`FAIL`
  line, the rebuild-first step, and the bounded wait are unchanged.
- New public constants: any constant the feature adds anywhere in the crate
  needs a matching `exclude` entry in `cbindgen.toml`, and
  `bash scripts/regenerate-abi-header.sh --check` must still exit 0. The
  authoring types are absent from the cbindgen `[export] include` list and the
  script parses the crate with default features, so the header does not grow.
- `Cargo.toml` and `devenv.nix`: the fixture's dependency on the `authoring`
  feature is the only manifest change, and no new tool is needed.

### Existing abstractions to use

- `spikes/plugin-authoring-trait/crates/authoring/`: the measured trait, `Call`,
  `PluginError`, `table::<P>()`, the host adapter, and the minimal executor.
  Port the shape into `crate::authoring`; do not depend on the spike crates.
- `spikes/plugin-authoring-trait/crates/plugin/` and `crates/consumer/`: prior
  art for the plugin implementation and the consumer that loads and drives it.
- The landed `crate::abi` and `crate::payload`: the buffers, the statuses,
  `metis_read_prefix`, the envelope header, the encode helper, and the release
  rule all exist, and this ticket adds no ABI type, no payload rule, and no
  layout change.
- `crates/plugin-fixture/src/lib.rs` and `src/bin/abi-harness.rs`: the fixture
  being rewritten and the host that must keep loading it for real.
- `docs/tasks/metis-foundations/tasks/plugin-abi-layout/task.md`: D3, D5, D7,
  D11, D12, and D15 plus the authoring spike's gap list are the settled input,
  and the `plugin-abi-core` section above states the crate-level obligations
  this ticket inherits.

### Do NOT reimplement

- Do not re-litigate D1 to D15 or the spike amendments, including "the adapter
  always returns `Pending`": no inline fast path in the trait adapter, because
  D11's inline shape stays the ABI's direct-implementer path.
- Do not add a macro: the spike measured about 90 translation lines, so a macro
  is not justified at this size.
- Do not add an async runtime: no `tokio` and no executor crate in
  `plugin-contract`. The crate drives the futures with its own minimal executor,
  and the SDK owns the real one.
- Do not move payload types, serde types, or the fixture's request and response
  shapes into `crate::abi`, and do not let the trait module import an ABI type.
- Do not make construction fallible or configuration-carrying: `Default` stays
  the construction shape, and `started` is where initialisation reports failure.
- Do not let the author surface carry a raw pointer, a `MetisBuffer`, a status
  enum, or a callback pointer: those belong to the adapter.
- Do not remove the harness's rebuild-first step, its real library load, or its
  bounded wait, and do not keep a second, ABI-direct implementation of the
  fixture alongside the trait one.

### Seams

1. **Always-pending seam.** Through the real library: every capability of the
   rewritten fixture returns `Pending` from `invoke` and delivers its result
   through the host `complete` callback, including the capability that used to
   finish inline, and the instance keeps serving the next call.
2. **Cancellation seam.** A call parked on I/O observes cancellation promptly:
   the test starts a capability that waits on the cancellation token, calls the
   table's `cancel`, and asserts the call ends promptly. `tool.cancellable`
   suppresses its completion and the ledger's `suppressed` counter moves, while
   the harness drops the late completion of the capability that ignores
   cancellation (`tool.pending`) and still releases its buffer.
3. **Lifecycle seam.** `started` runs from `create_instance` and `stopped` runs
   from `shutdown`, which joins every worker before returning, so no callback
   arrives after it (D12); a plugin whose `started` fails yields a failing
   `create_instance` and no instance.
4. **Event seam.** A plugin-originated event reaches the host: a capability and
   the lifecycle hook emit a topic and a payload through the author surface, the
   harness's `emit_event` callback records the event, and the harness releases
   the buffer, so the ownership ledger still balances.
5. **Descriptor seam.** `create_instance` checks the declared descriptors against
   the dispatch arms and fails when they disagree, so a missing arm is a loud
   creation failure and not a call-time `NotFound`; a unit test under the feature
   drives a deliberately inconsistent plugin through the exported adapter.
6. **Cancelled-completion seam.** The documented rule holds: `Cancelled` after a
   successful cancel completes nothing, `Cancelled` without a successful cancel
   still completes, and both leave the buffer ledger balanced.
7. **Feature seam.** With the feature off the crate's dependency list stays empty
   and the runtime path is unchanged; the trait module names no ABI type, and the
   header drift check still exits 0 with the authoring module in the tree.

Test types: the harness binary for seams 1, 2, 3, 4, and 6; unit tests under the
feature for seams 5 and 7; the `plugin-abi-core` symbol, layout, envelope, and
ownership checks keep running unchanged.

### Interface contract

- `plugin-runtime` (metis-plugins) consumes `plugin-contract` with the feature
  off and never depends on the author surface. `metis-plugin-sdk` re-exports the
  trait, `Call`, `PluginError`, `table`, the host adapter, and the minimal
  executor, and owns the real executor.
- The author-facing contract in one place: `Default` construction,
  `started`/`stopped` lifecycle, a cancellation token on the per-request context
  for prompt cancellation, always-pending completion through the host callback,
  `Cancelled` as control flow only after a successful cancel, capability
  descriptors checked against dispatch arms at creation, and plugin-originated
  events routed through the same surface.
- The ABI is untouched: no new table entry, no layout change, no new ABI type,
  no new payload rule, the header regenerates with no diff, and any new public
  constant carries an `exclude` entry.
- The fixture's rewrite moves two harness expectations off the `invoke` return:
  the inline capability completes as `Ok`, and the panicking capability
  completes as `Internal`, because the worker catches the panic after `invoke`
  has already returned `Pending`. A direct ABI implementer keeps both on the
  `invoke` return, the inline `Ok` with a payload and `Internal` from the
  exported adapter.
- Consequence recorded rather than hidden: once the fixture implements the trait
  only, nothing in the tree exercises the ABI's inline `invoke` result. The
  inline shape stays part of the ABI for direct implementers, and a later
  fixture or host that needs it covered has to add an implementer of its own.

---

## Landed ticket: `plugin-compatibility`

Landed on `task/plugin-compatibility` as the merge commit `c9227a4`, with the
landing record and this section's update in `c1d95d2`. It was blocked by
`plugin-abi-core` and `plugin-contract-v1`, both landed. Version tolerance on
top of the revised v1 layout: additive change inside the major, per-capability
version gating, unknown payload fields, and the refusal of a plugin built
against the retired pre-release table, whose typed error landed with
`plugin-contract-v1`.

The surfaces below are the measured ones. The implementation was verified on the
working branch `ticket/plugin-compatibility` at HEAD `602b348`, nine `wip:`
commits off `main`, where `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, the workspace `cargo test`,
`bash scripts/regenerate-abi-header.sh --check`, and
`cargo run -p plugin-fixture --bin abi-harness` (`[result] PASS (73 checks)`)
were all green. The divergences from the plan are folded in below: the minor
rule is one-directional in code, the fixture advertises eight capabilities, the
major seam is driven by a second compiled library, the faulting-page probes are
Linux only, and the gating policy lives in the harness as well as in the
runtime.

### Exports

- `crates/plugin-contract/src/abi/version.rs`: `metis_read_table` now compares
  the minor additively, `if prefix.abi_minor < METIS_ABI_MINOR`, so a newer
  minor is accepted and a lower one is still refused. No signature changed and
  no variant was added; `MinorMismatch` keeps its `known` and `found` fields and
  its doc states the one-directional rule. The check order is now documented in
  that file as part of the contract: major first, then minor, then the first
  entry the reported `size` does not cover. `plugin-runtime` inherits the
  relaxation through the shared `MetisAbiError` with no code change; only its
  `CallError::Version` doc text moved.
- The size-based presence predicate landed by `plugin-contract-v1`, next to
  `metis_read_prefix`, is consumed here rather than exported. It answers whether
  the table's reported `size` covers a given entry
  (`size >= offset_of!(entry) + size_of::<*const c_void>()`, written so it holds
  per target), and presence is decided by it and never by a null-pointer test.
  It serves both the plugin table and the host table, and it adds no field and no
  table entry. The header carries the same rule as slot constants, because
  cbindgen renders neither a `const fn` nor a computed `size_of` expression
  (both measured).
- The typed version error has already landed in `plugin-contract-v1` and is
  consumed here rather than added: `MetisAbiError` in
  `crates/plugin-contract/src/abi/version.rs`, with `Unreadable`,
  `MajorMismatch { expected, found }`, `MinorMismatch { known, found }` and
  `EntryMissing { entry }`, returned by the landed `metis_read_table`. The
  variant this ticket relaxed to additive tolerance is `MinorMismatch`, and the
  size cases it exercises are `EntryMissing` and `Unreadable`. It is Rust-only,
  absent from the generated header, and `plugin-runtime` inherits it instead of
  inventing its own (`CallError::Version(MetisAbiError)`). The symbol half of the
  major check (a plugin exports `metis_plugin_entry_v<major>`, so a v2 plugin
  cannot resolve through the v1 name) is the same fact at resolution time, and
  the probe for it
  stays in the harness until `plugin-runtime` owns loading.
- The per-capability gate, demonstrated in the fixture and the harness: a host
  refuses a capability whose descriptor version it does not support with
  `Unsupported` for that capability alone. The plugin still loads, still lists
  its other capabilities, and still serves them. The fixture declares eight
  capabilities, not the seven `plugin-contract-v1` landed: it gained `tool.newer`
  at descriptor version 2, plus the public constants `NEWER_CAPABILITY` and
  `NEWER_CAPABILITY_VERSION` and the private `CAPABILITY_VERSION`. Its public
  `CAPABILITIES` array is therefore `[&str; 8]`, and a consumer reads that array
  rather than its length. The cbindgen rule is satisfied trivially here: the
  constants live in `plugin-fixture`, which cbindgen never processes, and the
  generated header contains no `NEWER_` symbol. The harness applies its own gate
  policy, `HOST_CAPABILITY_VERSION = 1` in `capability_gate`, and
  `crates/plugin-runtime/tests/registry.rs` moved its "every descriptor is
  version 1" assertion to expect the gated version for that one name.
- `crates/plugin-fixture/src/payload.rs`: the unknown-field rule stays intact.
  No payload type sets `serde(deny_unknown_fields)`, and a new field is optional
  with a default, so a body carrying a field this build does not know decodes
  and the call succeeds. The three tests that assert it live in that module:
  `a_request_with_a_field_this_build_does_not_know_decodes`,
  `a_request_without_a_field_that_has_a_default_decodes`, and
  `a_response_with_a_field_this_build_does_not_know_decodes`.
- `crates/plugin-fixture/src/bin/abi-harness.rs`: new checks for the seams
  below, and the harness baseline moved from 62 to 73 checks. The old
  `symbol.wrong_major_absent` check is gone, because it looked up a symbol the
  fixture never exported and could not fail; four checks replaced it
  (`symbol.other_major_library_present`, `symbol.other_major_library_loaded`,
  `symbol.other_major_exactly_one_symbol`, `symbol.other_major_typed_error`),
  and they load a real other-major library, report
  `MajorMismatch { expected: 1, found: 2 }`, and see exactly one exported
  symbol, `metis_plugin_entry_v2`. `HostState::invoke` was split into a typed
  body encoder plus `invoke_body`, which takes raw bytes, so the unknown-field
  check can send a body this build has no type for. The fixture library is not
  replaced, the harness keeps its rebuild-first step and its real library load,
  and no second copy of the ABI appears anywhere.
- `crates/plugin-fixture/c/other-major-plugin.c` and
  `scripts/build-c-plugin.sh`: a 19-line C library whose only export is
  `metis_plugin_entry_v2`, built into `target/c-plugin/libother-major.so` with
  gcc only, because the header is already through both compilers. The harness's
  rebuild-first step builds it, so no CI step was added. The file is beyond the
  plan's export list: the major seam needs a case that can fail, and this is the
  real other-major library the four `symbol.other_major_*` checks drive.
- This ticket adds no layout: the revised table `plugin-contract-v1` landed is
  the one it reads, so `MetisPluginTableV1` keeps its eight entries and its 80
  bytes on 64-bit, and the Rust const assertions and the C translation unit
  changed by comment only, correcting text that still described the exact-minor
  rule. The harness still reports `table.size_matches_layout: size=80` and
  `symbol.exactly_one_metis_symbol`. A new capability descriptor field is a
  payload change, not a table change, and the cbindgen rule did not bite: the
  two new public constants are in `plugin-fixture`, which cbindgen never
  processes.
- `crates/plugin-contract/include/metis_plugin_abi.h`: regenerated, and it
  changed by doc comments only, in the `METIS_ABI_MINOR` block and the
  `abi_minor` field doc, because the Rust docs were rewritten for the relaxed
  rule. `bash scripts/regenerate-abi-header.sh --check` exits 0. Any branch
  carrying the pre-relaxation header must regenerate it.
- `docs/testing.md` documents the compatibility checks in the harness and the
  second C library. The plan did not ask for that doc update.

### Existing abstractions to use

- `metis_read_prefix` and `MetisTablePrefix` as landed: the prefix read is the
  entry point, and the predicate is the other half of the same idea, a consumer
  that reads only what the reported `size` promises.
- `crates/plugin-fixture/src/lib.rs`: the fixture whose shorter table and gated
  capability are exercised, with the capability list it advertises.
- `crates/plugin-fixture/src/bin/abi-harness.rs`: the check pattern
  (`Checks::new()`, one `[ok]`/`[fail]` line per check, a `[result]` line, a
  non-zero exit on failure), the timeout-bounded wait for a completion, and the
  rebuild-the-library-first step are the prior art for the new checks.
- `spikes/plugin-abi-mechanics/`: prior art for the prefix rules of D8 (size
  first, append-only entries, presence by size) on both tables.
- `crate::payload`: the envelope already owns the header rules, so unknown-field
  tolerance is a property of the payload types and not a new envelope rule.

### Do NOT reimplement

- Do not append a field or an entry to `MetisPluginTableV1` or
  `MetisHostApiV1`: this ticket is additive in code, not in layout, and the Rust
  const assertions and the C static asserts keep their numbers, with comment
  corrections only.
- Do not decide presence from a null pointer, and do not read a field past the
  `size` the plugin reported.
- Do not add a second capability descriptor type or a second payload shape: the
  fixture's payload module stays the one place the payload shapes live, and the
  serialiser stays outside `crate::abi`.
- Do not re-test the envelope's failure modes or re-state its header rules;
  `crate::payload` owns them.
- Do not add a loader to `plugin-contract`: the symbol probe stays harness-owned
  until `plugin-runtime` owns library loading.
- Do not weaken the harness: the rebuild-first step, the real library load, the
  bounded wait, and the `[ok]`/`[fail]` plus `[result]` contract stay.
- Do not add a gating check the fixture cannot make fail: the landed effort
  removed exactly such a check because the fixture advertised no version a host
  would refuse. This ticket closed that gap with `tool.newer` and the harness's
  own gate policy, and the same rule is why `symbol.wrong_major_absent` was
  replaced by four checks against a real other-major library.

### Seams

1. **Prefix seam.** A table whose reported `size` covers only the prefix this
   build knows is read through `metis_read_prefix` and the predicate: the entries
   inside the reported size are read with their real values, the entry outside it
   is refused with the typed error, and no field is reached past the size. Two
   mechanisms assert it. In `plugin-contract`'s tests, `ExtendedTableV1` carries
   an appended entry and a canary counter, and
   `abi.rs::an_appended_entry_and_a_newer_minor_still_interoperate` drives it, so
   the canary check runs on every target. In the harness,
   `table.appended_entry_not_read`, `table.uncovered_entry_not_read`, and
   `table.stale_minor_entries_not_read` place the table in an `mmap`ed page whose
   uncovered tail is `PROT_NONE` through a local `mprotect` binding, so a read
   past the reported size faults instead of returning a canary value. Those three
   checks are Linux only; on another target they print a note and skip rather
   than fail. The harness reports the shortened read as
   `table.appended_entry_not_read: the reader took this build's 80 bytes of the
   88-byte table it was shown and read no further`.
2. **Major seam.** A differing ABI major fails before any table is interpreted:
   the `metis_plugin_entry_v2` lookup fails against a real other-major library,
   the harness reports `MajorMismatch { expected: 1, found: 2 }` rather than a
   raw loader error, and nothing in the table is read. The retired pre-release
   v1 table's minor-0 refusal is the same gate in the other direction: additive
   tolerance leaves minor 0 refused, and the harness and
   `abi.rs::a_retired_minor_is_refused_before_any_table_entry_is_read` assert it
   by patching the live table's prefix rather than by compiling a second reject
   plugin. The symbol half holds by construction, because the probe goes through
   `metis_plugin_entry_v1`.
3. **Gating seam.** One capability whose descriptor version the host does not
   support is refused with `Unsupported` for that capability alone, while
   `list_capabilities` still lists the rest and the other capabilities still
   serve, asserted against the real fixture through the harness checks
   `gate.declared_versions`, `gate.version_refused_for_one_capability`,
   `gate.refusal_is_the_hosts`, and `gate.rest_of_plugin_still_serves` (eight
   capabilities, only `tool.newer` refused). The gate policy the harness applies
   is its own constant; `plugin-runtime::Registry::gate` keeps its unit test
   `the_version_gate_is_per_capability`, which drives synthetic names rather
   than the real fixture.
4. **Unknown-field seam.** A request body carrying a field this build does not
   know decodes and the call succeeds, asserted across the boundary by the
   harness check `payload.unknown_field_tolerated` and on the payload types by
   the three unit tests the exports list names.
5. **Additivity seam.** The layout numbers are unchanged and still pass: the Rust
   const assertions in `src/abi/layout.rs`, the C translation unit compiled on
   every build, and the comparison in `crates/plugin-contract/tests/c_layout.rs`.
   `src/abi/mod.rs` and `c/layout_static_asserts.c` changed by comment only. The
   header changed by doc comments only and regenerates with no drift, and the
   entry-symbol check still sees exactly one `metis_*` symbol in the fixture.

Test types: the harness binary for seams 1, 2, 3, and 4 (73 checks, and every
check runs on Linux); unit tests in `plugin-contract` for the predicate, the
typed error, and the payload tolerance, including `abi.rs` for the appended
entry, newer minor, other major, and retired minor cases; the existing layout,
C, and header checks for seam 5.

### Interface contract

- The compatibility surface is exactly this sequence: read the prefix with
  `metis_read_prefix`, decide presence with the size predicate, refuse a
  differing major with the typed error before interpreting anything, compare the
  minor additively (`prefix.abi_minor < METIS_ABI_MINOR`, so a lower minor is
  refused and a newer one is accepted), and gate each capability by its own
  descriptor version. Nothing else moved. The exact-minor refusal
  `metis_read_table` shipped is relaxed in that one comparison, in
  `crates/plugin-contract/src/abi/version.rs`, with no signature change and no
  new variant.
- `plugin-runtime` (metis-plugins) applies the same sequence at load time and
  per capability. The fixture and the harness here are the measured
  demonstration; the loader itself stays in that effort.
- The layout stays append-only and unchanged. The predicate is what a later
  ticket uses when it appends the first new table entry, and at that point the
  assertions in `src/abi/layout.rs` and `c/layout_static_asserts.c` are extended
  with the new entry.
- Descriptor gating is per capability and never fails the whole plugin: a
  refused capability is `Unsupported` for that name, the plugin keeps its other
  capabilities, and unknown payload fields stay ignorable (D5, D10).
- The fixture and the harness are the ones `plugin-contract-v1` landed, extended
  in place: eight capabilities through the routed trait, the eighth of them
  `tool.newer` at descriptor version 2, the C section, and the `[ok]`/`[fail]`
  plus `[result]` check printing. The earlier ordering note against
  `plugin-authoring-trait` is spent: both of those chains have landed, and
  `plugin-contract-v1` revised the fixture on top of them. `plugin-runtime`'s
  only change is one doc comment on `CallError::Version` and the
  descriptor-version expectation in `crates/plugin-runtime/tests/registry.rs`;
  its loader behaviour is unchanged.

### Residual, recorded rather than resolved

- The per-capability gate policy exists in two places: the harness's own
  `HOST_CAPABILITY_VERSION` constant and `plugin-runtime::Registry::gate`. The
  runtime applies its gate at load time in the metis-plugins effort, and the
  fixture's `tool.newer` is the capability that effort can reuse.
- The retired pre-release v1 case is simulated by patching the live table's
  prefix, not by a separately compiled minor-0 plugin, so no check loads such a
  binary.
- The three faulting-page prefix checks are Linux only, so a non-Linux host
  loses that canary; the Rust canary test still asserts the rule there.
- `scripts/build-c-plugin.sh` hard-requires `gcc` on the PATH, not `cc`. A shell
  with clang only (the default PATH outside `devenv shell`) makes the harness
  fail `cplugin.compiled_by_gcc_and_clang`. The CI gates run through
  `devenv shell`, where gcc is present.

---

## Landed ticket: `tcp-listener`

Landed on `task/tcp-listener` as the merge commit `c2e8684`, parents `c63a56e`
(the landing branch, created at this ticket's arch spec commit, which is also
`main`'s tip and carries every earlier landing merge) and `1d93f42` (the working
branch `ticket/tcp-listener`, four `wip:` commits off `main` at `c63a56e`, one
per seam, deleted after the merge); the docs commit `6bbf8a7` carries the ticket
note and the committed deviation report. The merge was clean, so the landing tree
is the tree the verify worker gated, and no gate was re-run in full at landing.
The gates were green on `1d93f42` through `devenv shell --`: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, the workspace `cargo test` (29
targets, no failure), the three `api-contracts` configuration unit tests, and
`cargo test -p client --test remote` (3 tests, run three extra times with no
flake).

Builds the optional remote transport: the same router reachable over TCP when
the operator names a bind address, and no listener at all when they do not.

The surfaces below are the measured ones. The divergences from the plan are
folded in below: three additive surface changes (`serve_tcp_listener(listener,
router)` holds the one accept loop and `serve_tcp` is a bind plus a delegate, the
public `remote_authority(addr)`, and `ConfigError::RemoteBind { addr, source }`),
and one prose-only change (`crates/api-contracts/src/lib.rs`, the `transport`
module doc, and the `client` crate doc no longer call the module "the local
transport" and name the remote front-ends and the off-by-default rule). One
acceptance criterion was narrowed by the operator before the chain: criterion 4
asserts the echo RPC over TCP, and the stream-over-TCP assertion belongs to
`event-stream`, the ticket that owns the first streaming RPC, because the
contract declares no streaming RPC yet. The seams below never asserted a stream,
so the narrowing changed no seam.

### Exports

- `crates/api-contracts/src/transport.rs` gains the TCP front-ends beside the
  landed Unix ones, sharing `serve_stream` exactly as `serve_uds` does:
  - `pub async fn serve_tcp_listener(listener: tokio::net::TcpListener,
    router: Router) -> io::Result<()>`: the one TCP accept loop. It wraps the
    router in the `Arc` itself, exactly as `serve_uds` does, and hands each
    accepted stream to `serve_stream` on its own task. This is the function
    `skeleton::serve_remote` drives today, and the one the daemon effort wires
    into `main`.
  - `pub async fn serve_tcp(addr: SocketAddr, router: Router) -> io::Result<()>`:
    binds `tokio::net::TcpListener` and delegates to `serve_tcp_listener`. No
    stale file to remove and no second serve loop. The split exists because a
    caller that must report the address the operating system assigned to port
    `0` has to bind first, read `local_addr()`, and then hand the listener on; a
    reserve-then-rebind wrapper would race the client's first connect. It is
    exported for the daemon's `main`, and no test drives it yet (recorded
    below).
  - `pub fn tcp_transport(addr: SocketAddr) -> connectrpc::client::Http2Connection`,
    beside `uds_transport` and built the same way (`lazy_with_connector` over a
    `service_fn` that opens `TcpStream::connect`), because `client::Client`
    deliberately exposes no transport injection point as landed. Its authority
    comes from `remote_authority(addr)`, never from a second format string.
  - `pub fn remote_authority(addr: SocketAddr) -> http::Uri`: `http://<addr>`,
    the one place that rule lives, mirroring the landed `local_authority()`.
    `tcp_transport` and `Client::with_remote_addr` both call it, which is what
    keeps the connection authority and the request authority from drifting
    apart. Additive beyond the plan's export list; a non-loopback authority (a
    proxy or a TLS SNI host) is its next consumer.
  - `pub const REMOTE_ADDR_ENV: &str = "METIS_TCP_ADDR";` beside the transport
    that reads it, and `pub fn resolve_remote_addr() -> Result<Option<SocketAddr>,
    ConfigError>`: unset or empty is `None`, which is the whole of "off by
    default"; an `IP:port` literal is `Some`; anything else, a bare hostname
    included, is a typed `ConfigError` naming the variable and the value, never
    a silent disable. The literal is parsed here and bound later, so resolution
    happens at bind time, not at config time.
  - `pub enum ConfigError`, a new `#[non_exhaustive]` enum in this module, the
    first configuration error in the repository, with `Display` and
    `Error::source`: the specified `InvalidRemoteAddr { variable: &'static str,
    value: String, source: std::net::AddrParseError }` and the additive
    `RemoteBind { addr: SocketAddr, source: io::Error }`, because `serve_remote`
    returns `Result<_, ConfigError>` and a bind can fail, so the start path
    reports the failure instead of panicking. `#[non_exhaustive]` is what lets
    the later transport tickets add their own settings and bind errors without a
    breaking change. No wire or ABI surface moves.
- `crates/client/src/lib.rs`: `pub async fn with_remote_addr(addr: SocketAddr) ->
  Result<Client, ClientError>` on the landed `Client`, mirroring `with_endpoint`
  but not `#[cfg(unix)]`, because TCP is the first transport every target has.
  It builds `tcp_transport(addr).shared(64)` and names
  `transport::remote_authority(addr)` as the client's authority, so the request
  authority is the one the connector was opened with. The wrapper keeps the
  transport private, so caller code names no TCP type. There is no remote
  constructor that reads the environment, because `METIS_TCP_ADDR` is the
  server's bind address; how a client finds a remote daemon is
  `pairing-and-tokens`' subject.
- `crates/client/src/skeleton.rs`: `pub async fn serve_remote() ->
  Result<Option<SocketAddr>, ConfigError>`. It resolves through
  `resolve_remote_addr()`; `None` returns `Ok(None)` having bound nothing;
  `Some(addr)` binds, reads `local_addr()`, spawns
  `transport::serve_tcp_listener(listener, router())` on its own task, and
  returns the bound address, so the test connects without guessing a port. Both
  the bind failure and the `local_addr` failure become
  `ConfigError::RemoteBind`. Test support keeps this start path until the daemon
  effort wires it into `main`.
- `docs/testing.md`: the transport bullet gains the remote test command and the
  variable, next to `METIS_ENDPOINT`, and names the TCP front-ends.

### Existing abstractions to use

- `serve_stream` unchanged, and `serve_uds` as the shape the TCP front-end
  copies: bind, accept, clone the `Arc` per stream, spawn.
- `skeleton::router()`: the remote test serves the same router the socket path
  serves, so it proves the transport and not a second service.
- The RAII environment helper in `crates/client/tests/echo.rs`
  (`EndpointOverride`, including the note that mutating the environment is
  unsafe in edition 2024) is the prior art for the remote variable's test. The
  new test lives in its own binary, `crates/client/tests/remote.rs`, and is not
  `#[cfg(unix)]`, because TCP is the first transport here that every target has.
- `Client`, `ClientError`, and the existing wrapper test
  `wrapper_echoes_over_a_temporary_socket`, whose assertion pair the TCP test
  repeats.
- `connectrpc::ConnectRpcService` and the `hyper_util` auto builder stay
  internal to `serve_stream`.

### Do NOT reimplement

- Do not add a second serve loop, a second service, or a per-connection router
  build: each accepted stream gets a clone of the one `Arc<Router>`.
- Do not wire TLS, a proxy header, or a trust rule. `reverse-proxy-mode` owns
  the proxy rule; this listener is plaintext, and ADR 0004 removed the native
  TLS work (`tls-and-client-trust` is deprecated).
- Do not add an `enabled` flag beside the address. Naming the address is the
  enablement; a separate flag would add an "enabled but unconfigured" state with
  nothing behind it.
- Do not create a configuration crate or a config file format for one variable.
  The environment is this effort's configuration surface (`METIS_ENDPOINT`
  landed that way).
- Do not add a public transport injection point to `Client`, and do not change
  the generated tree, the contract, or the ABI layout.
- Do not add authentication. That is `local-identity-and-acl`, and it applies to
  every transport.

### Seams

1. **Configuration seam.** `resolve_remote_addr()` reads `METIS_TCP_ADDR`:
   unset and empty give `None`; `127.0.0.1:0`, a port-bearing IPv4 value, and an
   IPv6 literal give `Some`; a malformed value and a bare hostname give
   `ConfigError`. Asserted by three unit tests in
   `crates/api-contracts/src/transport.rs`,
   `an_absent_or_empty_remote_addr_disables_the_listener`,
   `an_ip_literal_with_a_port_is_the_bind_address`, and
   `a_value_that_is_not_an_ip_port_literal_is_a_typed_error`, through the
   private `RemoteAddrEnv` RAII guard. The rule is an `IP:port` literal so
   resolution happens at bind time, not at config time.
2. **Off-by-default seam.** With the variable unset the start path binds
   nothing: `no_remote_configuration_opens_no_tcp_socket` asserts
   `serve_remote()` returns `Ok(None)`, then binds the port it reserved before
   the call and requires that bind to succeed, so the absence is measured rather
   than inferred.
3. **Listener seam.** A TCP listener serves the landed router:
   `the_client_echoes_over_the_remote_listener` starts `serve_remote()` on
   `127.0.0.1:0` with `METIS_TCP_ADDR` set, takes the returned address, and
   drives the echo RPC through `Client::with_remote_addr` end to end. The test
   is not `#[cfg(unix)]`, because TCP is the first transport every target has.
4. **Shared-path seam.** The TCP connection traverses the same `serve_stream` as
   the duplex and socket tests, which the echoed value asserts: a second serve
   loop would have to reproduce the service to pass. Seams 3 and 4 share the one
   test.
5. **No-leak seam.** Asserted by
   `the_listener_task_releases_its_port_when_it_is_dropped`: the listener owns
   its socket through the task it runs on, the test drops that runtime, and the
   port must then rebind, so a listener outliving its test cannot hide behind a
   passing suite.

Test types: unit tests inside `crates/api-contracts/src/transport.rs` for seam
1, integration tests in `crates/client/tests/remote.rs` for seams 2 to 5 over a
real loopback listener, run by the existing workspace `cargo test`. No new
harness and no new test protocol.

### Interface contract

- `reverse-proxy-mode` adds its own bind and
  connector front-end against `serve_stream`, `serve_tcp_listener`'s shape (the
  one accept loop; `serve_tcp` is a bind plus a delegate), and
  `resolve_remote_addr()`'s rule. It names its authority through a helper
  beside `local_authority` and `remote_authority` and adds its own error
  variants to the non-exhaustive `ConfigError`, and it still does not add an
  injection point to `Client`. `tls-and-client-trust` is deprecated by ADR 0004:
  the daemon does not terminate TLS, so no front-end of this module carries a
  certificate or a pin.
- `local-identity-and-acl` and `pairing-and-tokens` add the credential path.
  Both have since landed: `local-identity-and-acl` the local socket's OS access
  control and the identity declaration, which the TCP listener can carry as
  request metadata but which authenticates nothing, and `pairing-and-tokens` the
  token, the `AuthInterceptor` and the policy the local socket mounts. The landed
  TCP front-ends still take a bare `Router`, so they cannot mount the authorized
  service, and an enabled listener still serves an unauthenticated surface,
  which is why the bind address is required and has no default;
  `serve-tcp-service` owns the service-taking front-end, and it has since landed
  that front-end (`serve_tcp_service` and `serve_tcp_listener_service`), so the
  unauthenticated TCP surface is closed for a listener that mounts a service. The
  ordering risk is recorded, not hidden.
- The plaintext warning the map's constraints require belongs to the daemon's
  startup policy and to `reverse-proxy-mode`; this ticket records the residual
  instead of inventing a policy surface for it.
- The realtime stream over TCP was not asserted here: no streaming RPC existed
  until `event-stream` landed, and `serve_stream` serves any Connect service over
  the stream it is handed. This is the acceptance-criterion narrowing the
  operator approved before the chain: criterion 4 asserts the echo RPC only, and
  the stream-over-TCP assertion was carried by `event-stream`, the ticket that
  owns the first stream. Closed by `event-stream` (merge `0727964`):
  `crates/daemon/tests/stream.rs` asserts the stream over
  `serve_tcp_listener_service` with a paired token.

### Residual, recorded rather than resolved

- Plaintext and unauthenticated: an enabled listener serves the served surface
  with no credential check, because the landed TCP front-ends cannot mount the
  authorized service `pairing-and-tokens` landed. The required, defaultless bind
  address is the only mitigation until `serve-tcp-service` lands, and that
  ticket exists on `main` (`97e0a91`), which `remote-pairing` is blocked by.
  Closed by `serve-tcp-service` (merge `4f57d3d`), which landed the
  service-taking TCP front-end `serve_tcp_service`; the bind address stays
  required and defaultless, and a listener that mounts a bare `Router` still
  serves unauthenticated.
- `serve_tcp` is exported and unexercised: no test drives it and no caller
  exists until the daemon effort wires the listener into `main`. The tested path
  is `serve_tcp_listener`. If the operator wants it covered before the daemon
  adopts it, the cheapest fix is one assertion in the `serve_remote` bind step
  or a direct `serve_tcp` test.
- The RAII environment helper now exists three times:
  `crates/client/tests/echo.rs::EndpointOverride`, the private
  `transport.rs::RemoteAddrEnv`, and
  `crates/client/tests/remote.rs::RemoteAddrOverride`. Each carries its own
  mutex and each lives in a different test binary, so nothing races today;
  folding them is a coherence-pass item.
- `remote_authority` panics on an unparseable authority, in practice never,
  matching the landed `local_authority`. The IPv6 form is covered by the
  configuration unit test but not end to end.
- `serve_remote` and `serve_tcp_listener` stop only when their task or their
  runtime is dropped; there is no graceful shutdown, matching `serve_uds`.
- The rebind probe in `no_remote_configuration_opens_no_tcp_socket` is
  near-vacuous on its own, because a listener started on a kernel-assigned port
  would not take the reserved port: the `Ok(None)` assertion and the
  configuration unit tests carry criterion 3. Recorded rather than fixed.

---

## Landed ticket: `local-identity-and-acl`

Landed on `task/local-identity-and-acl` as the merge commit `694fe0c`, parents
`9f5fde2` (the landing branch, created at this ticket's arch spec commit, which
is also `main`'s tip and carries every earlier landing merge) and `fd2057d` (the
working branch `ticket/local-identity-and-acl`, six checkpoints off `main` at
`9f5fde2`, deleted after the merge), followed by `8e56620`, which carries the
ticket note and the committed deviation report. The merge was clean and
`git diff fd2057d HEAD` is empty, so the landing tree is byte for byte the tree
the verify worker gated, and no gate was re-run at landing.

Local access control from the operating system, plus the identity a client
declares on every call, so local clients need no credential and the daemon's log
says which surface did what. It also settles the endpoint default the landed
`local-transport-rpc-skeleton` section recorded as an open conflict.

The surfaces below are the measured ones. The divergences from the plan are
folded in: the ACL covers the bound socket file as well as its directory,
`IdentityError` gained `MalformedVersion` and `ConfigError` gained
`UnknownSocketGroup`, `api_contracts::identity` owns the whole refusal channel,
`Client::with_surface` rebuilds the config because connectrpc 0.9.1 has no
mutable default-header accessor, and `libc` is a new Unix-only dependency for
`chown(2)` and the group database. All of them are additive or prose. The plan's
"no chown, and no privileged requirement" sentence is the one correction that
matters: the owner never changes and no privilege is needed, but the group half
does call `chown(2)`.

### Exports

- `crates/client/src/lib.rs`, the endpoint rule, exactly as planned:
  - `pub enum EndpointError` (non-exhaustive) with `NoPrivateBase`, and
    `default_endpoint() -> Result<PathBuf, EndpointError>`: the path moves off
    `/tmp/metis-daemon.sock` to a private base,
    `$XDG_RUNTIME_DIR/metis/daemon.sock` when that variable is set and non-empty,
    else `$TMPDIR/metis/daemon.sock`, else `NoPrivateBase`. There is no silent
    `/tmp` fallback, because `/tmp` cannot carry the ACL.
  - `resolve_endpoint() -> Result<PathBuf, EndpointError>` keeps `METIS_ENDPOINT`
    first, so an operator or a test that names an endpoint wins. Both signatures
    gained a `Result`, and `local()`, `with_endpoint()` and
    `crates/client/examples/echo.rs` map the error into `ClientError::Endpoint`.
    Observed end to end: the example resolves `/run/user/1000/metis/daemon.sock`
    and completes with `[result] PASS`.
  - `crates/client/tests/acl.rs` serves a daemon over its own temporary socket
    base and reads the resulting modes back, so the rule is measured on a base
    the ticket owns, exactly as a runtime directory would be.
- `crates/api-contracts/src/transport.rs`, the ACL, applied by the listener
  rather than by callers:
  - `pub const SOCKET_MODE_PRIVATE: u32 = 0o700;`,
    `pub const SOCKET_MODE_GROUP: u32 = 0o770;`, and
    `pub const SOCKET_GROUP_ENV: &str = "METIS_SOCKET_GROUP";`.
  - `pub fn ensure_socket_dir(path: &Path) -> Result<(), ConfigError>`: creates
    the socket's directory if absent and applies the rule, 700 with no group
    configured and 770 with one.
  - `serve_uds` resolves the rule once from the environment, ensures the
    directory, removes the stale socket, binds, and then applies the same rule to
    the bound socket file. The rule itself is a private `SocketAcl` with
    `from_env`, `ensure_dir`, `check_dir`, `apply_acl` and `resolve_group`. This
    is the one deviation from the plan's export list, which named only
    `ensure_socket_dir`: acceptance criterion 1 says "the socket and its
    directory", and on Linux `connect(2)` checks write permission on the socket
    inode, so a 770 directory with a 700 socket would silently defeat the
    configured group. No signature moved, and because the listener applies the
    rule, every local bind carries it and no caller can skip it.
  - A pre-existing directory that is a symlink, is not a directory, is owned by
    another user, or carries a wider mode than the rule allows is refused with
    `ConfigError::SocketAcl { path, source }`, never chmodded. The daemon owns the
    directory as its own user: the group is applied with `chown(2)` carrying
    `uid = uid_t::MAX`, so the owner never moves, and the group name resolves
    through `getgrnam_r`/`getgrgid_r`. A `METIS_SOCKET_GROUP` naming no group is
    refused with the additive `ConfigError::UnknownSocketGroup { variable,
    value }` rather than falling back to the primary group: the planned
    `SocketAcl` variant has no path to name before the directory is resolved, and
    a silent fallback would weaken the rule the operator asked for. The enum
    stays `#[non_exhaustive]`.
- `crates/api-contracts/src/identity.rs`, the declaration:
  - `pub const PROTOCOL_VERSION: u32 = 1;` and
    `pub const SUPPORTED_PROTOCOL_VERSIONS: &[u32] = &[1];`.
  - `pub const PROTOCOL_VERSION_HEADER: &str = "metis-protocol-version";`,
    `PROTOCOL_SURFACE_HEADER = "metis-surface"`, and
    `PROTOCOL_CLIENT_VERSION_HEADER = "metis-client-version"`.
  - `pub struct ClientIdentity { protocol_version, surface, client_version }`
    with accessors, and `pub fn from_headers(headers: &HeaderMap) ->
    Result<ClientIdentity, IdentityError>`, whose doc comment states that
    reading the declaration grants nothing.
  - `pub enum IdentityError` (non-exhaustive): `MissingHeader { header }` and
    `UnsupportedVersion { found: u32, supported: &'static [u32] }`, whose
    `Display` names the supported versions. This is the typed error acceptance
    criterion 3 names. The additive third variant `MalformedVersion { value:
    String }` covers a version header that is present but not a number, which is
    reachable peer input neither planned variant can describe.
  - The refusal channel, one module owning both ends:
    `pub const SUPPORTED_VERSIONS_HEADER: &str = "metis-supported-versions";`,
    `refusal_headers(&IdentityError) -> HeaderMap`,
    `supported_versions_value() -> String`, and
    `parse_supported_versions(&str) -> Vec<u32>`. The plan said the typed error
    crosses the wire as a header but named neither the header nor its format, so
    the wrapper's mapping reads the header rather than parsing message text.
    These four names are additive; downstream tickets must use them rather than
    inventing their own header spelling.
- `crates/client/src/lib.rs`, the client half:
  - `pub enum Surface { Cli, Ui, Test, Unspecified }` with `as_str()`, and
    `Client::with_surface(self, Surface) -> Self` with the signature the plan
    named. The three headers are set as `ClientConfig` default headers where the
    config is built, so every call carries them, and the landed `local`,
    `with_endpoint` and `with_remote_addr` default to `Surface::Unspecified`.
    `with_surface` rebuilds the `ClientConfig` rather than mutating it, because
    connectrpc 0.9.1 exposes only consuming `with_default_header` and
    `with_default_headers` and no mutable default-header accessor; the config the
    wrapper builds holds only the authority and the declaration, so the rebuild
    is lossless. The plan sentence that named `config_mut()` described an API the
    dependency does not have.
  - `ClientError` gains `Endpoint(EndpointError)` and `ProtocolVersion { found:
    u32, supported: Vec<u32> }`. `supported` is parsed from the refusal's
    `metis-supported-versions` header through
    `identity::parse_supported_versions`, never from message text, and `found` is
    `identity::PROTOCOL_VERSION`, the constant the wrapper declares rather than a
    read-back from the built config.
- `crates/client/src/skeleton.rs`: `IdentityRecorder`, a clonable handle with
  `last()`, and `router_recording(&IdentityRecorder) -> Router` beside the
  unchanged `router()`. The recorder fills when a handler calls
  `identity::from_headers`, so the recording is per handler, which is the first
  residual below.
- `crates/api-contracts/Cargo.toml` and the root `Cargo.toml`: `libc` as a
  workspace dependency and a dependency of `api-contracts` under
  `[target.'cfg(unix)'.dependencies]` only. `std` exposes neither `chown` nor
  `getgrnam_r`, and a `chgrp` subprocess or hand-parsed `/etc/group` would be
  worse. It is the first new third-party dependency this line adds and it widens
  `api-contracts`' unix closure, so the plan's "no new dependency anywhere"
  sentence is the second correction this section records.
- `docs/testing.md` gains the access-control and identity run commands
  (`cargo test -p api-contracts`, `cargo test -p client --lib`,
  `cargo test -p client --test acl`, `cargo test -p client --test identity`), and
  `docs/dev-env.md` records the new default endpoint and that the daemon creates
  and restricts the socket directory. The plan named `docs/testing.md` only.

### Existing abstractions to use

- The `connectrpc` metadata path as landed: default headers on `ClientConfig`,
  `RequestContext::header` on the server,
  `ConnectError::new(ErrorCode::InvalidArgument, ...)` and
  `ConnectError::with_headers` for the refusal. No new wire shape and no RPC: the
  contract, the generated trees, the ABI layout and the plugin crates are
  untouched, so the drift, layout and harness gaps stay as they were.
- `serve_uds` as the ACL hook point, `default_endpoint` and `resolve_endpoint`
  for the rule, the non-exhaustive `ConfigError`, the `EndpointOverride` RAII
  helper, `TempSocket`, `Client::with_endpoint`, `skeleton::router()` and
  `SkeletonDaemon`.
- `crates/client/tests/echo.rs`'s `TempSocket` now creates its base directory
  private, because the ACL correctly refuses the 755 base the landed suite
  created. The assertion the test makes is unchanged, so no served-path coverage
  was weakened.

### Do NOT reimplement

- Do not add a `Handshake` RPC or connection state. Identity travels as request
  metadata on every call, which is the same channel `Authorization` needs later.
- Do not add a credential, a token, or an authorization check: the declaration is
  not authentication, and `pairing-and-tokens` owns credentials.
- Do not add a logging framework. Recording is the seam the test reads.
- Do not chmod or chown a path the daemon did not create, and never follow a
  symlink while doing either. The applied `chown(2)` carries `uid = uid_t::MAX`,
  so the owner never moves.
- Do not change the contract, the generated tree, the ABI, or the plugin crates.
  This ticket adds no RPC, and `bash scripts/regenerate-abi-header.sh --check`
  reports no drift.
- Do not keep a `/tmp` default behind a fallback that silently weakens the ACL.

### Seams

1. **Endpoint seam.** `default_endpoint()` derives from `XDG_RUNTIME_DIR` and
   then `TMPDIR`, and returns `NoPrivateBase` when neither is set;
   `METIS_ENDPOINT` still wins. Unit-tested in `crates/client/src/lib.rs`, where
   the RAII environment helper lives and where the resolver the example calls is
   defined.
2. **ACL seam.** `serve_uds` on a fresh directory leaves it mode 700 owned by the
   daemon's user; with `METIS_SOCKET_GROUP` set to a group that user belongs to,
   mode 770 with that group. The same rule lands on the bound socket file, and
   both modes are read back from the filesystem by the unit tests in
   `transport.rs`; `crates/client/tests/acl.rs` reads the directory and the
   socket a served daemon creates, independently of them, and asserts a shared
   directory is refused and left as it was. A pre-existing directory with a wider
   mode, one that is a symlink, one that is not a directory, or one owned by
   another user, and an unknown group name, are each refused with the typed
   error.
3. **Declaration seam.** A wrapper call carries the three headers, and the
   recorder sees exactly the declared surface and client version, over a real
   socket: `crates/client/tests/identity.rs` drives a wrapper call against
   `skeleton::router_recording` and asserts version 1, surface `cli`, the client
   version, and the `unspecified` default.
4. **Version seam.** A client declaring version 2 is refused, and the refusal
   names version 1 and carries `metis-supported-versions`; a client with no
   version header is refused the same way. The wrapper's mapping of that header
   to `ClientError::ProtocolVersion` is unit-tested over a synthetic refusal, so
   the client half is proven in two layers rather than one end-to-end mismatch
   call.
5. **Additivity seam.** The generated contract tree, the ABI layout and the
   plugin crates are untouched, so the drift, layout and harness checks stay
   green, and `bash scripts/regenerate-abi-header.sh --check` reports no drift.

Test types: unit tests in `client` and `api-contracts` for seams 1, 2 and the
header parsing, plus the two ACL mode assertions in `crates/client/tests/acl.rs`;
integration tests in `crates/client/tests/identity.rs` for seams 3 and 4; the
existing contract and ABI gates for seam 5.

### Interface contract

- `pairing-and-tokens` consumes the same metadata channel for
  `Authorization: Bearer` and owns the `ClientError` credential variants. The
  downstream outline sentence that placed those variants in this ticket is
  superseded, because this ticket adds no credential.
- `remote-pairing` carries the same three headers plus
  the credential over TCP. `event-stream` carries them on the stream request, and
  its `StreamEvents` handler calls `identity::from_headers` too (landed in
  `event-stream`, merge `0727964`), because the recording is per handler and the
  landed `RecordingDaemon` implemented only the one RPC the contract had then. No
  new refusal format appears anywhere: both ends of the version-refusal channel
  live in `api_contracts::identity` under the names above.
- The endpoint default changes the path both the daemon and the client use, while
  `METIS_ENDPOINT` remains the override for operators and tests. The effort map's
  fog entry about the `/tmp` conflict is closed by this rule, and so is the open
  conflict the landed `local-transport-rpc-skeleton` section recorded: `/tmp` is
  world-writable and sticky, so the daemon's socket lives under
  `$XDG_RUNTIME_DIR`, then `$TMPDIR`, and the resolver refuses with
  `EndpointError::NoPrivateBase` when neither is set.
- The ACL covers the bound socket file as well as its directory and is applied by
  `serve_uds`, so every local bind carries it and no caller can skip it.
  `windows-named-pipe` has its own rule and is unaffected. New settings and bind
  or ACL failures join the non-exhaustive `ConfigError`, which now carries
  `InvalidRemoteAddr`, `RemoteBind`, `SocketAcl` and the additive
  `UnknownSocketGroup`.
- `api-contracts` gained `libc` on Unix targets and nothing else new. The
  contract, the generated trees, the ABI layout and the plugin crates are
  untouched, so no drift, layout or harness gap moved.

### Residual, recorded rather than resolved

- The "directory not owned by this user" refusal is implemented but not
  integration-tested: creating a directory owned by another uid needs root, so
  only the symlink, non-directory, wider-mode and unknown-group refusals are
  asserted.
- The recording is per handler. `RecordingDaemon` implemented the one RPC the
  contract had then, so `event-stream`'s `StreamEvents` handler had to call
  `identity::from_headers` too, or streaming calls would be unrecorded rather than
  refused. It does, in the recording skeleton's handler and in the daemon's own
  (merge `0727964`), and the daemon effort inherits the same obligation.
- A daemon restarted without `METIS_SOCKET_GROUP` after a run with it meets a 770
  directory that its 700 rule refuses. Refusing is what the rule asks for, so an
  operator who removes the group must also remove the directory.
- The client's version refusal is proven in two layers, the wire refusal through
  the generated client and the wrapper's mapping over a synthetic refusal, rather
  than one end-to-end call, because the wrapper always declares the version this
  build speaks and no API in this ticket supplies a mismatching peer.
- `version_refusal` treats any failure whose response carries a parsable,
  non-empty `metis-supported-versions` header as a version refusal, from any
  peer. That is the intended channel, but a peer that sets the header for another
  reason would be reported as a version refusal.
- TDD ordering: the `identity` module landed with its unit tests in the same
  step, so its RED evidence is the client crate's earlier
  `unresolved import api_contracts::identity` rather than a per-test RED run.
  Every other seam has a real RED run.
- Acceptance criterion 1's wording says "a configured owner and group"; per the
  approved decision only the group is configurable. The directory is owned by the
  daemon's user by construction and the group half of the rule is applied with
  `chown(2)` and `uid = uid_t::MAX`, so the owner never moves. The ticket's
  implementation note records the caveat.

---

## Landed ticket: `state-store`

Landed on `task/state-store` as the merge commit `925371a`, parents `e88b055`
(the landing branch, created at this ticket's arch spec commit, which is also
`main`'s tip and carries every earlier landing merge) and `f8331b5` (the working
branch `ticket/state-store`, eleven `wip:` commits off `main` at `e88b055`,
deleted after the merge); `e824546` is the docs commit carrying the ticket note
and the committed deviation report. The merge was clean, and the landing tree
differs from `f8331b5` only in the ticket note and that report, so the tree the
verify worker gated is the tree that landed and no gate was re-run at landing.
The gates were green on `f8331b5` through `devenv shell --` with `DATABASE_URL`
unset: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test -p daemon` (10 passed), `cargo test -p api-contracts` (23 passed),
and the whole workspace `cargo test` (99 passed, 0 failed, up from 88).

The daemon's persistent state: one SQLite database opened through sqlx, migrated
on open, with the table the token hashes live in, so pairing and the later
session and message work share one store instead of inventing a file format each.
The decision record is `docs/adr/0003-daemon-state-in-sqlite.md`.
`crates/daemon` gained a library target beside its binary, because
`docs/rough-plan.md` starts storage as an internal module of the daemon crate
rather than as a new crate or a module of `api-contracts`.

The surfaces below are the measured ones. The divergences from the plan are
folded in: the `Verify` variants, the `&str` surface parameters, the added
`TokenError::Malformed`, the pinned hash input, the async `Result` signatures on
every store method, the two new public helpers in `api-contracts::transport`
with the `SocketAcl` to `PathAcl` rename, the dependency versions, and the
empty-variable rule in `resolve_db_path`. All of them are additive and inside the
recorded approval.

### Exports

- `crates/daemon/Cargo.toml`: a `[lib]` target beside the binary, because the
  migration seam reads the file the store created with an independent connection,
  which also moved `sqlx` into `[dev-dependencies]` beside the normal dependency.
  `sqlx` 0.9 with `default-features = false` and the `runtime-tokio`, `sqlite`,
  `macros` and `migrate` features; `sha2` 0.11 for hashing and `getrandom` 0.4
  for token bytes; `subtle` 2 for the constant-time hash comparison rather than a
  hand-rolled fold. `subtle`'s MSRV is 1.94 and the devenv shell is on 1.101;
  `Cargo.lock` grew by 921 lines. `libsqlite3-sys` therefore bundles a C SQLite
  library into every build. No other crate depends on any of these yet, so
  `pairing-and-tokens` is the first consumer.
- `crates/daemon/src/lib.rs`: `pub mod state;` and `pub mod token;`; `main.rs`
  stays a thin binary.
- `crates/daemon/src/state.rs`:
  - `pub const STATE_DB_ENV: &str = "METIS_STATE_DB";` and
    `resolve_db_path() -> Result<PathBuf, StateError>`: the variable wins, then
    `$XDG_DATA_HOME/metis/metis.db`, then `$HOME/.local/share/metis/metis.db`,
    then `StateError::NoStateDir`. An absent **or empty** variable is treated as
    unset, so an empty value falls back to the data home instead of naming the
    current directory, which is the rule the landed `METIS_ENDPOINT` and
    `METIS_TCP_ADDR` already follow.
  - `pub async fn Store::open(path) -> Result<Store, StateError>`: creates the
    directory at mode 700 and the file at mode 600 through the landed helpers
    (see `transport.rs` below), opens a pool, and applies the embedded
    migrations. `Store::open_in_memory()` is `async` too, for tests, and its pool
    is capped at one connection because an in-memory SQLite database is per
    connection; file-backed stores use the default pool sizing.
  - Every store method is `async` and returns `Result<_, StateError>`:
    `insert_token(&self, surface: &str, &TokenHash)`,
    `verify_token(&self, surface: &str, &TokenHash)` returning
    `Result<Verify, StateError>`, `revoke_token(&self, surface: &str)` and
    `surfaces(&self) -> Result<Vec<String>, StateError>`. The surface parameter
    is a `&str` name, so the store stays decoupled from
    `api_contracts::identity::Surface` and from the identity module entirely,
    and the daemon crate does not depend on the identity types. A caller passes
    the surface name it already reads from the declaration headers.
  - `pub enum StateError` (non-exhaustive): `NoStateDir`, `Io`, `Database`,
    `SurfaceAlreadyPaired`. Migration failures reuse `StateError::Database`
    through `sqlx::Error::Migrate`, so no variant beyond these four was needed.
  - `pub enum Verify` (non-exhaustive): `Verified`, `UnknownSurface`, `Mismatch`.
    The three are distinguishable on purpose, so a caller tells a revoked or
    never-paired surface from a bad token without a second query. The enum
    carries no hash, so a caller that verifies never holds a value it could
    replay, and criterion 5's "verification never returns a stored hash"
    therefore holds structurally rather than by test.
- `crates/daemon/src/token.rs`: `mint() -> (Token, TokenHash)` where `Token` is
  32 random bytes hex-encoded and `TokenHash` is its SHA-256;
  `Token::parse(&str) -> Result<Token, TokenError>` with the new non-exhaustive
  `TokenError::Malformed`, which implements `Display` and `Error`, so a
  malformed credential from the wire is a typed refusal rather than a panic;
  `TokenHash`'s comparison is constant time through `subtle`'s `ct_eq`.
  - The hashed input is pinned: `Token::hash()` is the SHA-256 of the token's
    canonical lowercase hex **text**, which is what a client presents, not of the
    32 decoded bytes, and `Token::parse` normalises case so the digest does not
    depend on how a client wrote the text. A known-answer test pins it: the
    digest of
    `0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef` is
    `a8ae6e6ee929abea3afcfc5258c8ccd6f85273e0d4626d26c7279f3250f77c8e`. This is
    the one wire-visible rule the ticket fixes, and `pairing-and-tokens` must
    hash the presented text exactly this way: a client that hashes the decoded
    bytes produces a credential that never verifies, and no test outside this
    ticket would catch it.
- `crates/daemon/migrations/0001_tokens.sql`:
  `tokens(surface TEXT PRIMARY KEY, hash BLOB NOT NULL, paired_at TEXT NOT NULL)`,
  applied by `sqlx::migrate!` so the binary embeds its schema and CI gained no
  step.
- `crates/api-contracts/src/transport.rs`: the landed private-path rule
  generalises into two new public functions, `ensure_private_dir(dir, mode)` and
  `restrict_file(path, mode)`, both `io::Result<()>`, and the private `SocketAcl`
  type became `PathAcl` with a `private(mode)` constructor. `ensure_socket_dir`
  and `serve_uds` keep their signatures and behaviour, `ConfigError::SocketAcl`
  keeps its fields, and the `path` in a directory refusal still names the
  directory. The internal refusal messages were reworded to name the path they
  refuse, which is visible only through an error's message text that no test
  asserts. The socket directory at 700, the socket file at 700 or 770, and the
  state directory at 700 with its file at 600 therefore all use one rule, and the
  store calls the same code path `serve_uds` uses rather than a copy of it.
- `docs/dev-env.md` records the variable, the default path and the modes, and
  `docs/testing.md` records the run commands (`cargo test -p daemon`) plus the
  no-server, no-`DATABASE_URL` property. The plan named `docs/dev-env.md` and
  `docs/testing.md`; both were updated in place.

### Existing abstractions to use

- The landed ACL rule and its refusals (symlink, owner, wider mode) as the
  pattern the state directory inherits at 700 and 600, reused through
  `transport::ensure_private_dir` and `transport::restrict_file` rather than
  copied.
- The non-exhaustive error convention of `ConfigError`, `IdentityError` and
  `ClientError`, carried over to `StateError`, `TokenError` and `Verify`.
- The RAII environment and self-cleaning temporary-directory helpers in
  `crates/client/tests/{echo,acl}.rs`, reproduced inside the daemon's
  `tests/path.rs` (`StateEnv`, with its own lock and restore-on-drop), because
  the client crate's helper is not visible across crates. Same pattern, no new
  mechanism.
- `docs/testing.md`'s command-list convention for the new run commands.

### Do NOT reimplement

- Do not store the token, the preshared key, or any plaintext secret in the
  database. Only the hash is written.
- Do not use `sqlx`'s compile-time `query!` macros: no `DATABASE_URL`, no
  committed `.sqlx` offline data, no database server in CI.
- Do not put the store in `api-contracts` or a new crate, and do not add a
  `sqlx-cli` step to CI: migrations are committed SQL applied by the embedded
  migrator on open.
- Do not build SQL by concatenation, and do not compare hashes with `==`.
- Do not add connection-pool tuning, a config file, or a migration rollback
  path.
- Do not implement pairing, the `Pair` RPC, the authorization check, or client
  credential storage: `pairing-and-tokens` owns all of that.

### Seams

1. **Path seam.** `METIS_STATE_DB` wins; then `$XDG_DATA_HOME`, then
   `$HOME/.local/share`; with no home directory the result is `NoStateDir`, and
   an absent or empty variable is treated as unset. Unit-tested with the RAII
   helper in `crates/daemon/tests/path.rs`.
2. **Creation seam.** Opening a fresh path creates the directory at 700 and the
   file at 600, asserted on Unix by `a_fresh_database_is_created_private`, which
   reads both modes back with `symlink_metadata().mode()`, matching how the ACL
   tests are gated. Non-Unix creation carries no mode rule (see the residual
   below).
3. **Migration seam.** `a_fresh_database_is_migrated_on_open` opens a fresh
   file, reads the schema through an independent connection, asserts the
   `tokens` table and `_sqlx_migrations(version = 1, success = 1)`, then re-opens
   the same file, so the migration is recorded as applied and opening twice is
   idempotent.
4. **Token seam.** `mint()` returns 64 hex characters from 32 random bytes and
   two mints differ; `hash()` is stable and 32 bytes, pinned by the known-answer
   digest above; verification accepts the stored hash and refuses any other.
   `a_token_row_stores_the_hash_and_verifies_it` reads the raw row (`surface`, a
   32-byte `hash`) and asserts the token text is absent from the database file
   and its `-wal`/`-journal` sidecars, so the row holds the hash and never the
   token.
5. **Revocation seam.** With two surfaces paired,
   `revoking_one_surface_leaves_the_other_verifying` asserts the revoked surface
   reports `Verify::UnknownSurface` or `Verify::Mismatch` while the other still
   verifies.
6. **Duplicate seam.** `an_insert_for_a_paired_surface_is_refused` asserts
   `StateError::SurfaceAlreadyPaired` and that the first row is intact, and
   verification cannot return the stored bytes at all because `verify_token`
   returns only `Verify`.
7. **Build seam.** The workspace builds and the daemon tests pass with no
   database server, no `DATABASE_URL` and no new CI step.

Test types: `sqlite::memory:` for the store operations, temporary files for the
path, creation and migration seams, all inside `crates/daemon`; the existing
workspace gates for the build seam, run with `DATABASE_URL` unset.

### Interface contract

- `pairing-and-tokens` consumes `Store`, `mint` and `TokenHash`, adds the `Pair`
  RPC and the authorization check, and adds no token table of its own. It calls
  `Store::open`, `mint`, `insert_token(surface, &TokenHash)`, `verify_token` and
  `revoke_token` with the surface name as a `&str`, maps
  `Verify::UnknownSurface` and `Verify::Mismatch` onto its two rejection causes,
  and turns a `TokenError::Malformed` from `Token::parse` into a typed refusal
  rather than an ended connection. The hash rule above is a cross-ticket
  contract: it must hash the token's canonical lowercase hex text, exactly as
  the known-answer digest pins it.
- The daemon effort later adds session, message and workflow tables through new
  migrations in the same directory, and the `agent-harness` `SessionStore`
  implementation is the daemon's, not this store's. `pairing-and-tokens` and the
  daemon effort inherit the three-step `resolve_db_path` fallback and its
  empty-variable rule.
- The store is the daemon's state boundary: nothing in `api-contracts` or
  `client` reads the database.
- The private-path helpers are shared: `ensure_private_dir` and `restrict_file`
  are what `windows-named-pipe` and `reverse-proxy-mode` reuse for their own
  directories and files, and the `PathAcl` rename is internal, so no caller
  signature and no `ConfigError` field moved.

### Residual, recorded rather than resolved

- `Store::open` narrows a pre-existing widened database file to 600 rather than
  refusing it, because the symlink, owner and wider-than-mode refusal rule
  applies to the directory, as it does for the socket directory.
- No rotation, no pruning and no stale-database unlink, and no test opens a
  database written by another version.
- No test asserts that the comparison is constant time, only that it works; the
  property rests on `subtle`'s `ct_eq`.
- The in-memory store is capped at one connection, because an in-memory SQLite
  database is per connection; file-backed stores use the default pool sizing.
- Non-Unix creation carries no mode rule, and `prepare` creates both paths
  without a mode there, with a comment that the front-end for that platform
  brings its own rule; `windows-named-pipe` owns it.
- `libsqlite3-sys` bundles a C SQLite library into every build, and `sqlx` 0.9
  needs a recent toolchain (MSRV 1.94 against the devenv shell's 1.101).
- `docs/dev-env.md` and `docs/testing.md` were updated with the ticket, so
  finalize needs no further docs change for the store itself.

---

## Landed ticket: `pairing-and-tokens`

Landed on `task/pairing-and-tokens` as the merge commit `1c89660`, parents
`97e0a91` (the landing branch, created at `main`'s tip, which already carried the
`state-store` landing merge and the docs-only `serve-tcp-service` follow-up) and
`7dc60b1` (the working branch `ticket/pairing-and-tokens`, nine `wip:` commits
off `main` at `4f7c999`, deleted after the merge); the docs commit `3c91ea0`
carries the ticket note and the committed deviation report. The merge was clean
and the landing tree differs from `7dc60b1` only by the three docs files
`97e0a91` added, so every source file landed byte for byte as verified. The gates
were green on `7dc60b1` through `devenv shell --`: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, the workspace `cargo test` (140
passed, 0 failed, 7 ignored), `buf lint`,
`buf breaking --against '.git#branch=main' --against-config buf.yaml`,
`bash scripts/regenerate-contracts.sh --check`, the ABI header drift check, the
ABI harness (`[result] PASS (73 checks)`) and `cargo run -p client --example
echo` PASS. The tdd step also ran one adversarial mutation on
`AuthInterceptor::needs_credential`: pinning it to `false` failed exactly the
four refusal and revocation tests, and the file was restored byte for byte.

Builds the credential lifecycle, blocked by `local-identity-and-acl` and
`state-store`, both landed: a surface pairs once with a preshared key carried in
the request body, receives its own token, keeps it in the platform keychain when
it can, and is revoked on its own. First ticket to append to the wire contract
since the skeleton.

The surfaces below are the measured ones. The divergences from the plan are
folded in: the factory returns the interceptor-carrying service rather than a
`Router`, which is the one that constrains every listener ticket, with the
additive `transport::serve_uds_service`; `Client::with_surface` is fallible and
`Client::pair` takes `&mut self`; the credential wire vocabulary lives in the new
`api_contracts::auth`, with `daemon::auth` re-exporting both header names; `pair`
takes four parameters; and `api_contracts::time` is a new public module.

### Exports

- `contracts/proto/metis/v1/daemon.proto`: `rpc Pair(PairRequest) returns
  (PairResponse);` appended to `DaemonService`, with
  `PairRequest { string preshared_key = 1; }` and
  `PairResponse { string token = 1; }`, the token returned once and never again.
  Regenerated through `scripts/regenerate-contracts.sh`, which brings `buf lint`,
  `buf breaking` and the drift check along. Landed as planned, with the proto
  file's top comment updated from `One ordinary RPC for now` to `Two ordinary
  RPCs for now`; the regenerated buffa and connect trees are committed, the
  drift check passes, and `buf breaking` reports nothing against `main`.
- `crates/api-contracts/src/auth.rs`, the credential's wire vocabulary, and the
  placement divergence that moved a planned daemon name into this crate: the
  daemon refuses through the same header the client reads, so the channel is
  spelled in one place. `AUTHORIZATION_HEADER = "authorization"`,
  `BEARER_SCHEME = "Bearer"`, `AUTH_ERROR_HEADER = "metis-auth-error"`, and
  `bearer(token) -> String` beside `bearer_token(value) -> Option<&str>`, where
  the scheme is matched case-insensitively and a present header that is not a
  bearer credential is not a missing one. The refusal vocabulary is
  `CredentialRefusal::{Missing, Invalid, Revoked}`, non-exhaustive, with `as_str`,
  `parse` and `Display`; its meaning crosses the wire as the header value and
  never as message text. `daemon::auth` re-exports both header names.
- `crates/api-contracts/src/keychain.rs`: the platform keychain through its
  command line tool, because the daemon reads a preshared key from one and the
  client writes its token to one, and `api-contracts` is the only crate both
  depend on. `available() -> Option<KeychainTool>`,
  `lookup(account) -> Result<Option<String>, KeychainError>`,
  `store(account, secret)`, `remove(account)`, and a non-exhaustive
  `KeychainError` with `Spawn { tool, source }` and `Refused { tool, status,
  message }`. The tool is `secret-tool` on Linux and `security` on macOS, probed
  on `PATH` and named by the additive public `KeychainTool::{SecretTool,
  Security}` with `command()`; the service name is the additive public
  `SERVICE_NAME = "metis-daemon"` and the account is the surface name. The
  measured rules those names carry: `available()` answers `None` when no tool is
  on `PATH`, an entry that is not there is `Ok(None)` (both tools report that
  with a silent non-zero exit), and a tool that runs and refuses is `Refused`,
  never a silent fallback. `store` writes the secret on standard input for
  `secret-tool` and as an argument for `security`.
- `crates/daemon/src/secret.rs`: `SecretRef::{Literal, Env, Keystore, File}`
  parsed from one string by the infallible `SecretRef::parse` with explicit
  `literal:`, `env:`, `keystore:` and `file:` prefixes, a bare value treated as
  a literal (a value that merely contains a colon included), and
  `resolve() -> Result<Secret, SecretError>`; `SecretError::{Unresolved,
  Unreadable(UnreadableSource), TooPermissive { path, mode }}`, whose additive
  `UnreadableSource::{Env, Keychain, File}` says what kind of source failed
  without a caller matching on error text. A secret file readable by group or
  others is refused, and `keystore:` goes through `api_contracts::keychain`. The
  measured rules: `PRESHARED_KEY_ENV = "METIS_PRESHARED_KEY"` carries the
  reference and `resolve_preshared_key() -> Result<Secret, SecretError>`
  resolves it, treating an absent or empty variable as `Unresolved`; an empty
  secret is `Unresolved` rather than a key an empty presentation could match;
  and a keystore reference is `Unresolved` when no tool is on `PATH` and
  `Unreadable(Keychain)` when the tool runs and refuses. `Secret` holds the
  value privately, exposes it only through `expose()`, and redacts it in
  `Debug`.
- `crates/daemon/src/auth.rs`:
  - Both header names, `AUTHORIZATION_HEADER` and `AUTH_ERROR_HEADER`, are
    re-exported from `api_contracts::auth` rather than declared here, so the
    daemon's refusals and the client's mapping cannot name different headers.
    `AuthError::{MissingCredential, RejectedCredential { cause }}` with
    `RejectionCause::{Invalid, Revoked}`, both non-exhaustive, with `Display`
    and `Error::source`, and `refusal_headers(&AuthError) -> HeaderMap` beside
    the identity module's equivalent.
  - `authorize(store, &HeaderMap) -> Result<Granted, AuthError>`: reads the
    surface from the identity declaration, takes `Authorization: Bearer`, parses
    the token with `Token::parse`, hashes it with `Token::hash`, and maps the
    store's `Verify` (`UnknownSurface` to `Revoked`, `Mismatch` to `Invalid`).
    `Granted` names the declaration and answers `identity()` and `surface()`,
    and carries no scope: one token grants the whole exposed surface. The
    measured edge rules: a declaration the daemon cannot read is
    `MissingCredential`; a header that is present but is not a bearer credential
    is `Invalid` rather than missing; a malformed token is `Invalid`; and a
    store fault fails closed as `Invalid`.
  - `enum AuthPolicy { LocalAcl, TokenRequired }` and `AuthInterceptor::new(
    store, policy)`, implementing `connectrpc::Interceptor`: a private
    `needs_credential(path)` decides the exemption by the
    `DAEMON_SERVICE_PAIR_SPEC.procedure` path, and one private `admit` is shared
    by `intercept_unary` and `intercept_streaming`, so the two halves cannot
    diverge and a handler cannot forget the check. Under `TokenRequired` a
    declaration the identity channel cannot read is refused inside `admit` as
    `InvalidArgument` carrying `identity::refusal_headers`, and a credential
    refusal crosses as `Unauthenticated` carrying `AUTH_ERROR_HEADER`.
  - `pair(store, &Secret, surface, presented) -> Result<Token, PairError>`:
    four parameters, because the plan's form did not say how the target surface
    and the presented key are both supplied. The key is compared with `subtle`
    in constant time and before any write, so a wrong key pairs nothing and
    learns nothing, and the token text is returned once while only its hash is
    inserted. `PairError::{WrongPresharedKey, AlreadyPaired, Store(StateError)}`,
    non-exhaustive, whose `Display` and `source` are implemented.
- `crates/daemon/src/service.rs`: `Service::new(store, preshared)` implementing
  the generated `DaemonService` under `#[allow(refining_impl_trait)]`, where
  `echo` stamps `received_at` through `api_contracts::time::rfc3339_now` and
  `pair` reads the declaration itself with `identity::from_headers`, because
  pairing is the interceptor's exemption, and refuses an unreadable one as
  `InvalidArgument` with the identity refusal headers. The factory is the
  planned name with the measured return type:
  `service(store, preshared, policy) -> ConnectRpcService<Router>`. connectrpc
  0.9.1 keeps the interceptor chain on `ConnectRpcService` and offers no
  conversion back to a `Router`, so the policy is welded to the service it was
  installed on: `ConnectRpcService::new(router).with_interceptor(
  AuthInterceptor::new(store.clone(), policy))`, and `Store` gained `Clone` so
  the interceptor holds one store while the service holds another. A listener
  therefore mounts a service, not a router: `transport::serve_uds_service` with
  `AuthPolicy::LocalAcl` on the local socket, and `TokenRequired` on the remote
  listener once a service-taking TCP front-end exists, which is
  `serve-tcp-service`. That is how a local call needs no token while a remote
  call does, with no peer-address sniffing. `PairError` maps to
  `PermissionDenied`, `AlreadyPaired` to `AlreadyExists` and `Store` to
  `Internal`.
- `crates/client/src/credential.rs`: `Backend::{Keychain, File(PathBuf)}` with
  `Display`; `store(surface, token) -> Result<Backend, CredentialError>`, which
  answers with the backend that took the credential, and `load(surface) ->
  Result<Option<Stored>, CredentialError>` beside `remove(surface)`, where
  `Stored` carries the token and the backend, answers `token()` and `backend()`,
  and redacts the token in `Debug`. `CredentialError` is non-exhaustive, with
  `NoDataDir`, `InvalidSurface { surface }`, `Keychain(KeychainError)` and
  `Io(io::Error)`. The keychain is tried first through
  `api_contracts::keychain`, and the fallback file is `metis/client/<surface>.token`
  under `$XDG_DATA_HOME`, else under `$HOME/.local/share`, at mode 600 inside a
  700 directory through `transport::{ensure_private_dir, restrict_file}`, the
  same private-path rule the socket and the state database use. Only one backend
  is consulted, and a keychain that is present but refuses is an error rather
  than a silent fallback; a surface name that is not one file name is
  `InvalidSurface` rather than a path that steps out of the directory.
- `crates/client/src/lib.rs`: `with_surface(self, Surface) -> Result<Self,
  ClientError>`, fallible because declaring a surface loads that surface's stored
  credential, and `pair(&mut self, preshared_key: &str) -> Result<Backend,
  ClientError>`, taking `&mut self` because attaching the minted token replaces
  the built configuration through `config_mut()`. The three landed constructors
  `local`, `with_endpoint` and `with_remote_addr` keep their signatures, default
  to `Surface::Unspecified`, and attach whatever credential that surface has
  stored, so the landed in-tree caller moved to `?` with no assertion change.
  `Client::surface()` reports the declaration. The credential is attached as a
  default `Authorization: Bearer` header through `auth::AUTHORIZATION_HEADER` and
  `auth::bearer`, on the call metadata path alone: no caller names a credential
  and nothing lands in a URL. `ClientError` gains `Credential(CredentialError)`,
  `CredentialMissing` and `CredentialRejected { cause: CredentialCause }` with
  `CredentialCause::{Invalid, Revoked}`, all read from `AUTH_ERROR_HEADER` and
  never from message text; a refusal value this build does not know stays
  `ClientError::Call` rather than being guessed at.
- `crates/api-contracts/src/time.rs`: `rfc3339_now() -> String` and
  `rfc3339(seconds: u64) -> String`, the RFC 3339 UTC stamp
  `EchoResponse.received_at` asks for, plus its unit tests. The module is
  additive and new here: the two functions moved out of `client::skeleton` so the
  daemon's own service can stamp a reply without depending on test support, and
  the skeleton now delegates to them, so both services that answer the contract
  share one calendar implementation.
- Configuration and docs: `METIS_PRESHARED_KEY` (a `SecretRef`) recorded in
  `docs/dev-env.md` together with the token file's path and mode and the keychain
  rule, and the run commands for the new suites in `docs/testing.md`. Both files
  were extended beyond the single variable the plan named.

### Existing abstractions to use

- The landed store: `daemon::state::Store`, `daemon::token::{mint, Token,
  TokenHash}`, including the pinned rule that the digest is SHA-256 over the
  token's canonical lowercase hex text
  (`a8ae6e6ee929abea3afcfc5258c8ccd6f85273e0d4626d26c7279f3250f77c8e`), which
  this ticket must not re-derive differently.
- `connectrpc::Interceptor`, `Next`, `NextStream`, `UnaryRequest`, `StreamRequest`
  and `ConnectRpcService::with_interceptor` as the middleware the framework
  already provides, rather than a hand-rolled tower layer.
- The landed metadata and refusal channels: `identity::{from_headers,
  refusal_headers}` and the `metis-supported-versions` header are the shape
  `metis-auth-error` copies; `PROTOCOL_SURFACE_HEADER` is where the surface comes
  from.
- `transport::{serve_uds, serve_uds_service, ensure_private_dir, restrict_file}`
  for the tests' socket, the token file's mode and its directory. `serve_uds` now
  delegates to `serve_uds_service` through the private `serve_connection` body
  `serve_stream` also uses, so the ACL the local listener applies covers the
  service path as well and a test that needs the interceptor mounts the service
  rather than the router.
- `ConfigError` and the other non-exhaustive error enums; the RAII environment and
  temporary-directory helpers in the existing tests; `crates/client/tests/identity.rs`
  as the integration pattern over a real socket.
- `client::skeleton` stays the second `DaemonService` implementation, and it now
  answers `Pair` with `ErrorCode::Unimplemented` and the message `the skeleton
  does not pair; the daemon's own service does`, so pairing is exercised only
  through the daemon's service while the identity and transport tests keep the
  skeleton.
- `api_contracts::time` is the one calendar implementation the two services that
  stamp `received_at` share; the skeleton delegates to it instead of keeping its
  own.

### Do NOT reimplement

- Do not put the credential, the preshared key or the token in a URL, a query
  string, a log line, or a message body on the wire. The preshared key is the one
  exception, and only inside `PairRequest`.
- Do not store a token, or the preshared key, in the database: only the hash,
  through the landed store.
- Do not add per-client permissions or scopes: one token grants the whole exposed
  surface.
- Do not add TLS, a certificate, a pin or a proxy trust rule (ADR 0004).
- Do not build a second credential store, a second token format, or a second
  hashing rule.
- Do not add a keyring crate when the platform's command line tool can be probed,
  and do not silently fall back from a keychain that exists but fails: fall back
  only when no tool is on `PATH`.
- Do not hand-roll middleware when `connectrpc::Interceptor` exists, and do not
  enforce authorization inside handlers.
- Do not mount an authorized service on the landed TCP front-ends (`serve_tcp`,
  `serve_tcp_listener`): they take a bare `Router`, so the interceptor would not
  travel with it and the listener would serve unauthenticated. A TCP front-end
  that takes a `ConnectRpcService` is `serve-tcp-service`'s work, shaped like
  `serve_uds_service`.

### Seams

1. **Secret-reference seam.** Each of the four forms resolves; a file readable by
   group or others is `TooPermissive { path, mode }`; an unset or empty variable,
   an empty secret, a missing file and a keychain that is not there are typed
   `Unresolved` errors while a keychain that refuses is `Unreadable(Keychain)`; a
   bare value is a literal. Driven by `crates/daemon/tests/secret.rs` (12 tests)
   with a temporary file and the RAII environment helper.
2. **Pairing seam.** Over a real socket, `Pair` with the resolved key returns a
   token once and the store holds only its hash; the same call with a wrong key
   is `WrongPresharedKey`; pairing an already-paired surface is `AlreadyPaired`
   and leaves the first row intact. The socket is the authorized service mounted
   with `transport::serve_uds_service` under `AuthPolicy::LocalAcl`, and the test
   drives it through `client` as a dev-dependency (`crates/daemon/tests/auth.rs`,
   10 tests).
3. **Authorization seam.** Under `TokenRequired`, a call with no `Authorization`
   header is `CredentialMissing`, an unknown surface is `CredentialRejected {
   Revoked }`, a wrong token is `CredentialRejected { Invalid }`, and a valid
   token is served; each refusal is produced by the interceptor, so the handler
   is proven not to have run. Each refusal reaches the client as its typed error
   through `AUTH_ERROR_HEADER`, never through message text. Under `LocalAcl`, the
   same call is served with no credential at all. The unary half is wire-tested;
   the streaming half is proven by construction through the shared `admit` and is
   `event-stream`'s wire test.
4. **Revocation seam.** With two surfaces paired, deleting one row and re-serving
   leaves that surface rejected and the other working. Moved by the same
   mutation check as seam 3: pinning `needs_credential` to `false` fails exactly
   the four refusal and revocation tests.
5. **Credential-storage seam.** With no keychain tool on `PATH`, the file backend
   is used, reports itself, and writes mode 600 inside a 700 directory; with a
   stub tool earlier on `PATH`, the keychain backend is used for store and load.
   `crates/client/tests/credential.rs` (5 tests) drives the client half and
   `crates/api-contracts/tests/keychain.rs` (5 tests) the keychain module.
6. **Digest-contract seam.** The token `Pair` returns verifies against the row the
   store wrote, using the landed hash rule, so a change to that rule fails here
   rather than at a client. The unit tests in `daemon/src/auth.rs` assert that the
   returned text verifies against the stored row, so the cross-ticket digest
   contract `state-store` pinned is asserted here rather than assumed.
7. **Contract seam.** The proto edit regenerates cleanly, `buf lint` and
   `buf breaking` stay green, and the generated tree is committed.

Test types: unit tests in `daemon` for seams 1 and 6, in `client` for the
credential store's backends and in `api-contracts` for the keychain module and
its time module; integration tests in `crates/daemon/tests/` over a real socket,
with `client` as a dev-dependency, for seams 2 to 5; the contract gates for seam
7. The measured suite sizes on the landing tree are `daemon/tests/secret.rs` 12,
`daemon/tests/auth.rs` 10, `daemon/tests/token.rs` 4, `client/tests/credential.rs`
5 and `api-contracts/tests/keychain.rs` 5, and the workspace total is 140 passed
with 0 failed.

### Interface contract

- A listener mounts a service, not a router. `service(store, preshared, policy)`
  returns `ConnectRpcService<Router>`, and the interceptor chain lives on that
  value; `transport::serve_uds_service(path, service)` is the Unix front-end that
  takes it and applies the same ACL `serve_uds` applies. The landed TCP
  front-ends (`serve_tcp`, `serve_tcp_listener`) still take a bare `Router`, so an
  authorized service cannot be mounted on TCP yet: a TCP listener wired through
  today's surface would serve unauthenticated. `serve-tcp-service` on `main`
  (`97e0a91`) owned the service-taking front-end, and `remote-pairing` was blocked
  by it; that front-end has since landed (`serve_tcp_service` and
  `serve_tcp_listener_service`, merge `4f57d3d`). `windows-named-pipe` and
  `reverse-proxy-mode` have the same choice to make and the same front-end shape
  to copy.
- `remote-pairing` reuses `Pair`, the auth interceptor, the secret references and
  the client credential store over the remote transport; it adds no second
  credential path. It also owns whether a remote listener runs `TokenRequired`
  from the start, because the factory takes the policy, so the daemon effort or
  that ticket decides the wiring, and it can now do that over TCP, because the
  landed `serve-tcp-service` mounts the factory's `ConnectRpcService` through
  `serve_tcp_service`.
- `event-stream` inherits authorization through `intercept_streaming`, so its
  `StreamEvents` handler does not call the credential check itself; the obligation
  is the interceptor's, which is why this ticket removes that residual from
  `local-identity-and-acl`. That handler does check the identity declaration,
  which is a separate obligation. The streaming half had no wire test, because
  the contract declared no streaming RPC: it was proven by construction through
  the one shared `admit`, and `event-stream` has since supplied the wire test
  (merge `0727964`).
- A dependent that maps a credential refusal reads `api_contracts::auth`
  (`CredentialRefusal`, `AUTH_ERROR_HEADER`, `bearer_token`), not the daemon's
  own spelling; `daemon::auth` re-exports both header names for its refusals
  (the landed `remote-pairing` added a third, `PAIR_ERROR_HEADER`, and its
  `PairRefusal` vocabulary to that surface).
  `reverse-proxy-mode` and the daemon effort consume `SecretRef` for their own
  settings, `api_contracts::keychain` is the one place the platform tool is
  called, and `api_contracts::time` is the one calendar implementation.
- `Client::with_surface` is now a fallible call on a landed surface and
  `Client::pair` takes `&mut self`, so `terminal` and the app call the first as a
  `Result`; the landed constructors are unchanged.

Recorded residuals:

- The streaming half of the interceptor had no wire test, because the contract
  declared no streaming RPC; `event-stream` exercises it now, refusing a stream
  with no credential and serving one with a paired token over TCP (merge
  `0727964`).
- A store fault during verification fails closed as `CredentialRejected {
  Invalid }`, because `AuthError` carries the two planned causes; the enum is
  non-exhaustive, so a store-fault variant stays additive.
- The secret file's access rule is Unix only, and keychain absence versus refusal
  is inferred from the tool's exit status and standard error, so a silently
  failing tool reads as absence. macOS compiles but is never run against the real
  `security` tool.
- A wrong preshared key reached the client as `ClientError::Call` with
  `PermissionDenied` rather than as a typed pairing error; the spec named none,
  and the typed credential mapping covered the credential path only. Closed by
  `remote-pairing`, which landed the `metis-pair-error` channel and
  `ClientError::Pairing`; recorded here as the state at this landing.
- The `Pair` exemption is by procedure path, so a future pre-auth RPC must be
  added to that list deliberately; there is no token rotation or expiry.
- `client::skeleton` remains a second `DaemonService` implementation used by the
  identity and transport tests and answers `Pair` with `Unimplemented`, and the
  daemon effort replaces it; the daemon binary does not mount either service yet,
  so the listeners exist only in tests and in the daemon effort's future wiring.
- An authorized service could not be mounted on the landed TCP front-ends until
  `serve-tcp-service` landed; it has landed, so `serve_tcp_service` and
  `serve_tcp_listener_service` mount the interceptor-carrying service.

---

## Landed ticket: `serve-tcp-service`

Landed on `task/serve-tcp-service` as the merge commit `4f57d3d`, parents
`fb36073` (the landing branch, created at `main`'s tip because it did not exist
before this landing, the convention this spec records) and `c4c2c16` (the working
branch `ticket/serve-tcp-service`, seven `wip:` commits off `main` at `fb36073`,
deleted after the merge); the docs commit `5ef1042` carries the ticket note and
the committed deviation report. The merge was clean and `git diff c4c2c16 HEAD`
is empty, so the landing tree is byte for byte the tree the verify worker gated.
The gates were green on `c4c2c16` through `devenv shell --`: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test -p daemon --test auth`
(14 passed), the workspace `cargo test` (144 passed, 0 failed, 7 ignored),
`buf lint`, `buf breaking` against `main` with `--against-config buf.yaml`,
`bash scripts/regenerate-contracts.sh --check`, the ABI header drift check, and
the ABI harness (`[result] PASS (73 checks)`). The implementer also ran
one adversarial mutation: making `serve_tcp_listener_service` ignore the
mounted service failed exactly the four new tests, and the file was restored
byte for byte.

The TCP twin of the landed `serve_uds_service`: a front-end that serves a mounted
`ConnectRpcService<Router>` rather than a bare `Router`, so a remote listener can
mount the credential policy `pairing-and-tokens` landed. Before this ticket a TCP
listener wired through the landed surface served unauthenticated, because
`connectrpc` 0.9.1 keeps the interceptor chain on `ConnectRpcService` and offers
no conversion back to a `Router`.

Three files changed: `crates/api-contracts/src/transport.rs` (+65/-21),
`crates/daemon/tests/auth.rs` (+285) and `docs/testing.md` (+9/-1).

The surfaces below are the measured ones. The divergences from the plan are
folded in below: the accept loop wraps the mounted SERVICE in one `Arc` and hands
each accepted stream a clone of it rather than building an `Arc` for the router
per stream, because the interceptor chain lives on the service; the landed
`serve_tcp_listener` and `serve_stream` were reduced to delegations to that loop
and to the private connection body, which is the shape that satisfies "no second
accept loop" and "keep their signatures and behaviour" together; the bind seam is
driven with a reserved loopback port because `serve_tcp_service` returns
`io::Result<()>` and cannot report a kernel-assigned address; and the four new
tests are appended to `crates/daemon/tests/auth.rs`, so that binary's
`#![cfg(unix)]` gate covers them.

### Exports

- `crates/api-contracts/src/transport.rs`:
  - `pub async fn serve_tcp_listener_service(listener: tokio::net::TcpListener,
    service: ConnectRpcService<Router>) -> io::Result<()>`: the one TCP accept
    loop. It wraps the mounted service in one `Arc`, hands each accepted stream a
    clone of the service, and delegates the stream to the private
    `serve_connection` body that `serve_stream` and `serve_uds_service` already
    share. The service is the value the loop clones, not the router, because the
    interceptor chain lives on the service; `ConnectRpcService`'s clone is
    shallow (`Arc<D>`), so the per-connection semantics match the
    per-connection `from_arc` the loop used before.
  - `pub async fn serve_tcp_service(addr: SocketAddr, service:
    ConnectRpcService<Router>) -> io::Result<()>`: binds
    `tokio::net::TcpListener` and delegates to `serve_tcp_listener_service`,
    mirroring the landed `serve_tcp` and `serve_tcp_listener` split. It returns
    `io::Result<()>` only, so it cannot report a kernel-assigned address: a
    caller that must report the port binds first, reads `local_addr()`, and calls
    the listener-taking twin.
  - Those two are the only public additions; the diff shows exactly two new
    `pub async fn` lines in the file.
  - The landed `serve_stream`, `serve_uds`, `serve_uds_service`, `serve_tcp` and
    `serve_tcp_listener` keep their signatures, error handling, and observable
    behaviour, and no existing transport test changed. Two bodies moved:
    `serve_tcp_listener` is now
    `serve_tcp_listener_service(listener, ConnectRpcService::from_arc(Arc::new(router)))`,
    and `serve_stream` is now
    `serve_connection(stream, ConnectRpcService::from_arc(router.into()))`, the
    same delegation `serve_uds` already had to `serve_uds_service`. The
    repository still holds exactly two accept loops, the Unix one in
    `serve_uds_service` and the TCP one in `serve_tcp_listener_service`, and
    `client::skeleton::serve_remote` keeps calling
    `serve_tcp_listener(listener, router())` unchanged.
    `ConnectRpcService<Router>` is exactly what
    `daemon::service::service(store, preshared, policy)` returns.
- `crates/daemon/tests/auth.rs`: four new tests over a real TCP socket, 10 tests
  before and 14 after, with the three new helpers `serve_over_tcp`,
  `reserve_loopback_port` and `wait_for_tcp`:
  `the_tcp_service_front_end_refuses_a_call_with_no_credential`,
  `the_bind_and_delegate_front_end_serves_the_mounted_service`,
  `pairing_stays_exempt_over_the_tcp_service_front_end` and
  `a_service_mounted_on_both_front_ends_answers_the_same_way`. No new unit test
  in `crates/api-contracts`: both new functions are covered at the integration
  seam. The file carries `#![cfg(unix)]`, so these four tests are Unix-gated
  although TCP is available on every target (recorded below).
- `docs/testing.md`: the transport bullet extended with the service-taking TCP
  front-ends, why they exist, and the parity assertion. No acceptance criterion
  asked for that doc update; it follows how the earlier transport tickets
  documented their run paths.

### Existing abstractions to use

- The private `serve_connection` body, `serve_uds_service`'s service-taking
  shape, and `serve_tcp_listener`'s accept loop: the two new functions are the
  intersection of those and add no loop.
- The landed `daemon::auth::AuthInterceptor` and
  `daemon::service::service(...)` as the service the test mounts.
- `crates/client/tests/remote.rs` for the loopback bind and `local_addr()`
  pattern, and `crates/daemon/tests/auth.rs` for the policy fixture and its
  refusal assertions.
- `client::credential` and `Client::pair` for the paired-token half of the test.

### Do NOT reimplement

- Do not add a second accept loop or a second connection body.
- Do not change `serve_tcp` or `serve_tcp_listener` to take a service: the Router
  shape has callers, and both shapes are legitimate. The new functions are
  additive.
- Do not touch the contract, the generated trees, the ABI, or the plugin crates.
- Do not re-test the credential logic or the interceptor: this ticket tests the
  transport that mounts a policy, not the policy.

### Seams

1. **Mounting seam.** A `ConnectRpcService` carrying `AuthPolicy::TokenRequired`,
   served over a real TCP listener, refuses a call with no credential with the
   typed missing-credential error and serves a call carrying a paired token, with
   the `Pair` procedure still exempt. Three tests in
   `crates/daemon/tests/auth.rs` drive it:
   `the_tcp_service_front_end_refuses_a_call_with_no_credential` (the refusal
   reaches the client as `ClientError::CredentialMissing` read from
   `AUTH_ERROR_HEADER`, so it can only come from the interceptor, and the `Served`
   recorder proves the handler never ran),
   `the_bind_and_delegate_front_end_serves_the_mounted_service` (a paired token
   is served through the address-taking front-end), and
   `pairing_stays_exempt_over_the_tcp_service_front_end` (the `Pair` procedure is
   exempt over TCP and the minted token is then served).
2. **Parity seam.** One cloned service value mounted through `serve_uds_service`
   and through `serve_tcp_listener_service` produces the same refusals and the
   same served result for the same requests, so the TCP front-end is not a
   second, weaker policy path. Asserted by
   `a_service_mounted_on_both_front_ends_answers_the_same_way`. The assertion
   covers the unary path only, because the contract still declares no streaming
   RPC; `event-stream` carries the stream half.
3. **Bind seam.** `serve_tcp_service` serves through a real bound socket, and the
   listener-taking twin is the entry point a caller uses when it needs
   `local_addr()` first, which is the reason the landed split exists. The test
   reserves a loopback port by bind-and-release, passes that address to
   `serve_tcp_service`, and waits for the listener to accept; port `0` is
   exercised through `serve_tcp_listener_service`, which the other three tests
   use. The reserved-port pattern is racy in principle and is recorded below.
4. **Signature seam.** The landed transport front-ends keep their signatures and
   the existing transport tests are unchanged, so nothing built on the Router
   shape breaks.
5. **Additivity seam.** No contract, generated tree, ABI layout, or plugin
   change: the drift, lint, breaking and harness gates stay green.

Test types: four integration tests in `crates/daemon/tests/auth.rs` for seams 1
to 3, over a real loopback socket and reusing that binary's landed auth fixture,
temporary store directory, `client::credential` and `Client::pair` (the fixture
is private to that binary, which is why they were appended there rather than
placed in a new file); the existing transport tests plus the workspace gates for
seams 4 and 5.

### Interface contract

- `remote-pairing`, which this ticket blocked, mounts
  `AuthPolicy::TokenRequired` on its TCP listener through `serve_tcp_service`; the
  service-taking front-end it needed now exists.
- The daemon effort mounts the local service through `serve_uds_service` and the
  remote one through `serve_tcp_service`. `reverse-proxy-mode` extends the same
  module and inherits the service-taking shape for its own front-end.

### Residual, recorded rather than resolved

- Two parallel front-end families now exist over one shared connection body, one
  Router-taking and one service-taking, so a future front-end must choose
  deliberately. Keeping the body private and shared is what stops them diverging
  in behaviour, and the parity seam is what asserts it today.
- The bind test's reserved-port pattern is racy in principle on a busy host: the
  port is chosen by bind-and-release, and another process can take it before
  `serve_tcp_service` binds.
- The four TCP tests are Unix-gated, because they live in
  `crates/daemon/tests/auth.rs`, whose `#![cfg(unix)]` covers the whole binary, so
  the service-taking TCP front-end has no test coverage on non-Unix targets even
  though TCP is the first transport every target has. A dedicated test binary is
  the fix if that coverage is wanted.
- The parity assertion covers the unary path only, because the contract declared
  no streaming RPC; `event-stream` owns the stream, both in the contract and at
  the wire, and it landed the wire half over `serve_tcp_listener_service` (merge
  `0727964`). The parity comparison itself is still unary-only.
- `serve_tcp_listener`'s body changed from holding the accept loop to delegating
  to it, so the landed function keeps its signature and its observable behaviour
  but is no longer byte-identical to what `tcp-listener` landed. Criterion 3 read
  literally as "not touched" would require a duplicated accept loop instead,
  which the same criterion forbids.

---

## Landed ticket: `remote-pairing`

Landed on `task/remote-pairing` as the merge commit `adb4c5d`, parents `fd65ccc`
(the landing branch, created at `main`'s tip because it did not exist before this
landing, the convention this spec records) and `4651798` (the working branch
`ticket/remote-pairing`, four `wip:` commits off `main` at `fd65ccc`, deleted
after the merge); the docs commit `1d29511` carries the ticket note and the
committed deviation report. The merge was clean and `git diff 4651798 HEAD` is
empty, so the landing tree is byte for byte the tree the verify worker gated. The
gates were green on `4651798` through `devenv shell --`: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test -p daemon --test auth`
(16 passed), `cargo test -p daemon --test secret` (12 passed), the workspace
`cargo test` (146 passed, 0 failed, 7 ignored), `buf lint`,
`buf breaking --against '.git#branch=main' --against-config buf.yaml`, both drift
checks (`scripts/regenerate-contracts.sh --check` and
`scripts/regenerate-abi-header.sh --check`) and the ABI harness
(`[result] PASS (73 checks)`).

Pairing from a device with no access to the daemon's local socket, so a phone can
set itself up over the network: the `Pair` procedure is reachable on the TCP
front-end `serve-tcp-service` landed, gated by the same preshared key, and the
call after a successful pairing carries the minted token over the same transport.
Removing the key closes pairing on both transports and revokes nothing.

Nine tracked files changed (+577/-76): `crates/api-contracts/src/auth.rs`,
`crates/daemon/src/auth.rs`, `crates/daemon/src/secret.rs`,
`crates/daemon/src/service.rs`, `crates/daemon/tests/auth.rs`,
`crates/daemon/tests/secret.rs`, `crates/client/src/lib.rs`, `docs/dev-env.md`
and `docs/testing.md`. The tenth change in the working tree is the uncommitted
orchestration pointer `docs/tasks/state.yaml`, as in the earlier landings.

The surfaces below are the measured ones. The divergences from the plan are
folded in below: `pair_refusal_headers` returns `Option<HeaderMap>` so the
internal store fault carries no wire value; `PairError::PairingDisabled` maps to
`FailedPrecondition`, which the plan left unnamed; the public `Service::new`
takes the key, because the service is the value that holds it;
`api_contracts::auth` gained the `PairRefusal` type the plan described without
naming; and seam 4 gained no dedicated parity test, for the reason recorded at
that seam.

### Exports

- `crates/daemon/src/secret.rs`: `pub fn resolve_preshared_key() ->
  Result<Option<Secret>, SecretError>`. An unset or empty `METIS_PRESHARED_KEY`
  is `Ok(None)`, which means pairing is not configured; a configured reference
  that cannot be resolved stays a typed error, so a typo is loud rather than
  silently disabling pairing. A caller that ignores the `None` arm would treat a
  closed pairing as a configured one, and the daemon effort is the next caller.
- `crates/daemon/src/service.rs`:
  - `pub fn service(store: Store, preshared: Option<Secret>, policy: AuthPolicy)
    -> ConnectRpcService<Router>`; `None` builds a service whose pairing handler
    refuses with `PairError::PairingDisabled` while the rest of the surface still
    serves.
  - `pub fn Service::new(store: Store, preshared: Option<Secret>) -> Self`, the
    key-taking constructor the plan did not name: the service is the value that
    holds the key, so a caller building one passes `Some(key)` or `None`. Only the
    landed test fixture does that today; the daemon effort is the first product
    caller.
  - The private `refuse_pair(&PairError)` maps a refusal onto the wire: the
    headers when `pair_refusal_headers` has some, and the status code
    `FailedPrecondition` for `PairingDisabled`, `PermissionDenied` for
    `WrongPresharedKey`, `AlreadyExists` for `AlreadyPaired` and `Internal` for
    `Store`. The status code is not what a client acts on, because the typed
    refusal is read from the header; a client that inspects codes only sees a new
    code for the closed state.
- `crates/daemon/src/auth.rs`: `PairError::{PairingDisabled, WrongPresharedKey,
  AlreadyPaired, Store}` (non-exhaustive) plus the refusal channel
  `pub fn pair_refusal_headers(error: &PairError) -> Option<HeaderMap>`, mirroring
  `refusal_headers`. The header spelling is not declared here: `daemon::auth`
  re-exports `api_contracts::auth::{AUTH_ERROR_HEADER, AUTHORIZATION_HEADER,
  PAIR_ERROR_HEADER}`, exactly as it re-exported the first two. `None` is the
  `PairError::Store` case, which carries no wire value: a store fault is an
  internal failure rather than a pairing state a client should act on, so that
  case keeps its status code and stays `ClientError::Call` at the client.
- `crates/api-contracts/src/auth.rs`: `pub const PAIR_ERROR_HEADER =
  "metis-pair-error"` and the non-exhaustive `pub enum PairRefusal { Disabled,
  WrongKey, AlreadyPaired }` with `as_str`, `parse` and `Display`, spelling the
  header values `disabled`, `wrong-key` and `already-paired`. This is the type
  the plan described without naming it, and it mirrors the landed
  `CredentialRefusal`, so both ends of the pair-error channel spell the
  vocabulary once, which is the rule `identity` and `auth` already follow.
- `crates/client/src/lib.rs`: `ClientError::Pairing { cause: PairingCause }` with
  `PairingCause::{Disabled, WrongKey, AlreadyPaired}`, read through
  `PairRefusal::parse` from `auth::PAIR_ERROR_HEADER` and never from message text;
  a header value this build does not know is left as `ClientError::Call` rather
  than guessed at. This closes the residual the `pairing-and-tokens` section
  recorded, that a wrong preshared key arrived as `ClientError::Call` carrying
  `PermissionDenied`.
- `docs/dev-env.md` and `docs/testing.md`: pairing is optional, an absent key
  means pairing is closed rather than broken, and the remote run commands.

No new client constructor: `Client::with_remote_addr(addr).await?.with_surface(surface)?`
then `pair(psk)` already composes the remote case, and the minted token is
attached by `pair` as a default header.

### Existing abstractions to use

- `serve_tcp_service` and `serve_tcp_listener_service`, landed in
  `serve-tcp-service`, mount the policy on TCP; `AuthInterceptor` already exempts
  the `Pair` procedure, so the preshared key reaches the handler there.
- The landed fixture in `crates/daemon/tests/auth.rs` with its `serve_over_tcp`,
  `remote_client` and `reserve_loopback_port` helpers, and the port-0 pattern in
  `crates/client/tests/remote.rs`.
- `identity::refusal_headers` and `auth::refusal_headers` as the shape the
  pair-refusal channel copies, including the non-exhaustive enum with `as_str`
  and `parse`.
- `daemon::token`, `daemon::state::Store` and the landed pairing handler: this
  ticket adds a state, not a second mint or a second store write.

### Do NOT reimplement

- Do not add a second pairing path, token mint, or store write.
- Do not make the daemon refuse to start when no key is configured. Pairing is
  optional, and its absence closes pairing rather than daemon start.
- Do not revoke tokens when pairing is closed: closing pairing stops new
  pairings, and revocation stays the token-row delete that `pairing-and-tokens`
  landed.
- Do not add TLS, a certificate or a pin (ADR 0004).
- Do not assert the event stream here; no streaming RPC exists until
  `event-stream` lands.

### Seams

1. **Remote pairing seam.** With `AuthPolicy::TokenRequired` mounted on a real
   TCP listener and the key configured, a client pairs over TCP with the preshared
   key, receives a token, and a following call over the same transport is served.
   The `Pair` path stayed exempt, and neither the key nor the token appears in a
   URL. Asserted by
   `pairing_over_the_remote_listener_is_gated_by_the_preshared_key`, which also
   asserts that the configured key alone pairs, that a wrong key writes no row,
   and that the recorded request paths are the bare procedure path with neither a
   query nor the key.
2. **Closed-pairing seam.** A service built with `None` refuses `Pair` on both the
   Unix socket and TCP, with `disabled` on the wire and
   `ClientError::Pairing { cause: PairingCause::Disabled }` at the client, while a
   surface paired before the key was removed still serves. That is the property
   that closing pairing revokes nothing. Asserted by
   `removing_the_preshared_key_closes_pairing_on_both_transports`, which pairs one
   surface on each transport, unsets `METIS_PRESHARED_KEY`, rebuilds both
   listeners from `resolve_preshared_key()` (`Ok(None)`) over a reopened store on
   the same file, and then asserts the refusal on both transports, an unchanged
   token table and both previously paired surfaces still served.
3. **Typed-refusal seam.** A wrong key is `WrongKey` and a repeated pairing is
   `AlreadyPaired` at the client, both read from the pair-error header, with
   message text never parsed. The wrong key is asserted on the remote transport by
   `pairing_over_the_remote_listener_is_gated_by_the_preshared_key` and the
   repeated pairing on the local socket by the landed
   `pairing_mints_one_token_the_daemon_keeps_only_as_a_hash`; the landed
   `the_pairing_gate_uses_the_configured_key_reference` and
   `a_wrong_preshared_key_is_refused_and_mints_nothing` now expect the typed
   wrong-key refusal in place of `Call` carrying `PermissionDenied`, and
   `the_configured_preshared_key_is_a_secret_reference` in
   `crates/daemon/tests/secret.rs` now expects `Ok(None)` for an unset or empty
   variable in place of `SecretError::Unresolved`.
4. **Two-transport seam.** The same assertions run over the Unix socket and TCP
   through the landed fixture. The closed-pairing test drives both transports in
   one body, and the typed refusals are asserted on the remote transport by the
   new test and on the local socket by the landed ones. No separate parity test
   was added, because a third test repeating wrong-key and already-paired over
   both transports would be verbatim duplication; the two-transport claim
   therefore rests on the closed-pairing test, and the landed
   `a_service_mounted_on_both_front_ends_answers_the_same_way` still covers the
   interceptor path itself.
5. **Additivity seam.** No contract, generated tree, ABI layout, or plugin
   change. The wire gains a response header only, which is not a proto change, so
   the drift, lint, breaking and harness gates stay green.

Test types: two new integration tests in `crates/daemon/tests/auth.rs`, that file
being 14 tests before and 16 after, for seams 1 to 4, reusing its landed fixture
(`Served`, `serve_over_tcp`, `remote_client`, `reserve_loopback_port`, `AuthEnv`)
plus `daemon::secret::resolve_preshared_key`, and the workspace and contract gates
for seam 5. The fixture's `RecordingDaemon` was strengthened into a decorator that
records the request path and then delegates to the daemon's own `Service` instead
of answering with canned payloads, and `AuthEnv::unset`, `Fixture::unset_var` and
`Fixture::serve_configured_daemon_over_tcp` were added; no landed assertion was
dropped or loosened by that change. The file carries `#![cfg(unix)]`, so all 16
tests are Unix-gated, and these two inherit that.

### Interface contract

- `reverse-proxy-mode` owns the trusted-proxy rule and the plaintext warning for
  remote exposure; this ticket changed nothing about plaintext, which ADR 0004
  records as the operator's chosen posture.
- `event-stream` owns the stream half of this ticket's second acceptance
  criterion, and inherits both the token requirement and the pairing exemption
  through the interceptor, which applies the same `admit` to the streaming shape
  through `intercept_streaming`.
- The daemon effort mounts the remote listener with `AuthPolicy::TokenRequired`
  and the key `resolve_preshared_key` resolved, passing `None` when the
  configuration names no key; the binary still runs no listener, so the wiring
  remains the daemon effort's. A caller that builds `Service` directly passes
  `Some(key)` or `None`, and the `None` arm is the closed-pairing state rather
  than a misconfiguration.
- A later pairing refusal is additive at both ends: `PairRefusal` and `PairError`
  are non-exhaustive, a new variant travels through `pair_refusal_headers`
  unchanged, and the client's mapper leaves a header value this build does not
  know as `ClientError::Call` rather than guessing at it.

### Residual, recorded rather than resolved

- A client that pairs over the plaintext remote transport exposes the preshared
  key to anyone who can read the connection. The map records it and ADR 0004
  accepts it, since remote exposure is meant to sit behind a proxy;
  `reverse-proxy-mode` owns the warning and the trusted-proxy rule.
- The two new tests live in `crates/daemon/tests/auth.rs`, whose `#![cfg(unix)]`
  gates the whole binary, so remote pairing has no test coverage on non-Unix
  targets. That is the gate `serve-tcp-service` recorded and it is unchanged.
- The event-stream half of criterion 2 was unproven by design: the contract
  declared no streaming RPC, and `event-stream` owned that wire test. It has
  landed it (merge `0727964`), asserting the refusal and the paired-token stream
  over TCP.
- Closing pairing revokes nothing, as approved and asserted: a surface paired
  before the key was removed keeps serving until its token row is deleted.
- Pairing over a network-reachable listener has no rate limit and no lockout, and
  this ticket is what makes repeated attempts reachable over the network. The
  constant-time comparison is unchanged from `pairing-and-tokens`.
- The daemon binary still mounts no listener, so wiring the optional key and
  `AuthPolicy::TokenRequired` into a real remote listener remains the daemon
  effort's work.
- One assertion was lost rather than moved when the refusing tests moved to the
  typed channel: the landed test required the unset-key refusal text to name
  `METIS_PRESHARED_KEY`, and an unset key now has no refusal text, because it is
  `Ok(None)`. The variable appears only in `PairError::PairingDisabled`'s
  `Display`, which no test asserts. No other assertion was weakened.

---

## Landed ticket: `event-stream`

Landed on `task/event-stream` as the merge commit `0727964`, parents `4235803`
(the landing branch, created at `main`'s tip because it did not exist before this
landing, the convention this spec records) and `64bd7d5` (the working branch
`ticket/event-stream`, seven `wip:` commits off `main` at `4235803`, deleted after
the merge); the docs commit `f765728` carries the ticket note and the committed
deviation report. The merge was clean and `git diff 64bd7d5 HEAD` is empty, so the
landing tree is byte for byte the tree the verify worker gated and no gate was
re-run at landing. The gates were green on `64bd7d5` through `devenv shell --`:
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, the workspace
`cargo test` (163 passed, 0 failed), `cargo test -p api-contracts` (42 passed),
`cargo test -p daemon --test stream` and `cargo test -p client --test stream` (3
and 2 passed), `buf lint` with the one scoped exception,
`buf breaking --against '.git#branch=main' --against-config buf.yaml`, both drift
checks (`scripts/regenerate-contracts.sh --check` and
`scripts/regenerate-abi-header.sh --check`), the ABI harness (`[result] PASS
(73 checks)`) and `cargo run -p client --example events` (`[result] PASS`). The
first attempt's chain timed out in tdd because the plugin-resolving drift check
was refused by the BSR's anonymous rate limit, the failure the
`contract-pipeline` section records; the operator installed a BSR credential and
the retry re-verified the unchanged, already-complete tree, which is why that
step needs `BUF_TOKEN` in practice as well as in CI.

The realtime path, and the first ticket of the realtime line: the contract's
first streaming RPC and its first enum, the daemon-wide event ring and the
snapshot that opens every stream, and a client stream that can be drained. It
pays two landed debts: the streaming half of the interceptor, which
`pairing-and-tokens` could prove only by construction, and the stream-over-TCP
assertion `tcp-listener`'s acceptance criterion 4 deferred.

The surfaces below are the measured ones. The divergences from the plan are
folded in below: the delegated payload choice (`string payload = 4`), the prefixed
`EventKind` constants, the ring's extra members and the `Ring::stream` assembly,
the `Service` surface, the client's stream helpers and its wider dependency
footprint, and the example landing as a new file beside `echo.rs`.

### Exports

- `contracts/proto/metis/v1/daemon.proto`:
  - `rpc StreamEvents(StreamEventsRequest) returns (stream Event);` appended to
    `DaemonService`.
  - `StreamEventsRequest { repeated string channels = 1; uint64 last_event_id =
    2; }`, where `0` means no cursor and ids start at 1, so the field needs no
    `optional`.
  - `Event { uint64 id = 1; EventKind kind = 2; string channel = 3; string
    payload = 4; }`, carrying the event's channel, kind and identifier. **The
    payload field landed as `string payload = 4` carrying JSON text**, the
    operator-delegated choice and the plan's accepted fallback, recorded here as
    the plan required. A `string` pulls no well-known type or schema dependency
    into generation, and both the ring and the client treat the payload as opaque
    text. The lock-in is the one the plan named: `buf breaking` runs in FILE mode
    and this is field 4 of a committed message, so a later native encoding is a
    breaking change. `stream-resume-and-expiry`,
    `cancellation-and-slow-consumers` and the daemon's real event sources encode
    payloads as JSON text.
  - `enum EventKind { EVENT_KIND_UNSPECIFIED = 0; EVENT_KIND_SNAPSHOT = 1;
    EVENT_KIND_HEARTBEAT = 2; EVENT_KIND_EVENT = 3; }`, the contract's first enum,
    declared with the idiomatic prefixed proto constants rather than the
    unprefixed spelling this section first carried. buffa additionally emits the
    short Rust aliases `EventKind::Snapshot` and `EventKind::Heartbeat` under
    `#[allow(non_upper_case_globals)]`. **The spelling `EventKind::SNAPSHOT` does
    not exist**: a caller writes `EventKind::Snapshot` or
    `EventKind::EVENT_KIND_SNAPSHOT`, and a kind this build does not know reads as
    `EventKind::Unspecified` through `Event::from_message`.
    `cancellation-and-slow-consumers` adds kinds in this spelling, and it needed
    none: a stream ending in the middle is a trailer, not an event, so that landing
    added no `EventKind` member. The enum is the
    contract's first, the one the buffa mount's dormant
    `clippy::derivable_impls` allow was reserved for.
  - `Cancel` is **not** in this ticket; `cancellation-and-slow-consumers` appends
    it.
- `buf.yaml`: the `RPC_RESPONSE_STANDARD_NAME` exception this file already
  reserves for the streaming message, scoped as tightly as buf v2 allows, with
  `buf lint` green. Regeneration through `scripts/regenerate-contracts.sh`, so the
  committed trees and the drift check move with it.
- `crates/api-contracts/src/events.rs`, mounted as `pub mod events;`: the ring, in
  the shared crate because the daemon serves it and the client's in-process test
  service and example also need one, and this is the only crate both reach. The
  crate's manifest gained `futures-util` for it.
  - `pub struct Event { id: u64, kind: EventKind, channel: String, payload:
    String }`, the Rust mirror of the message. `kind` is the generated enum, so
    the two sides cannot drift on what a snapshot, a heartbeat, or an event is,
    and the payload is text. `Event::to_message()` and
    `Event::from_message(&v1::Event)` convert between the mirror and the wire
    message.
  - `Ring` is `Clone`, so the daemon can publish into the ring its service serves
    and a test can hold a second handle. `Ring::new(capacity)` raises a capacity
    of `0` to `1`, because a window that holds nothing cannot carry an
    identifier.
  - `publish(channel, kind, payload) -> u64`, `snapshot(&[&str]) -> Vec<Event>`,
    `since(id, &[&str]) -> Vec<Event>`, `earliest_resumable()`, `latest_id()`,
    `subscribe() -> Subscription`, and `Ring::stream(channels, cursor, heartbeat)
    -> impl Stream<Item = Result<v1::Event, ConnectError>> + Send + 'static`.
    `latest_id()` and `earliest_resumable()` answer `0` on an empty ring, which is
    the "nothing has been assigned an identifier yet" value the cursor rule
    already used.
  - One daemon-wide monotonic counter across channels, starting at `1`, and a
    bounded capacity that drops the oldest event and advances the earliest
    resumable id. `snapshot` is the head of each selected channel in identifier
    order, a channel named twice contributes once, and an empty selection selects
    nothing; `since` returns every held event after an identifier.
  - `Ring::stream` is the one assembly of a stream's shape: the snapshot first
    (one empty snapshot message, naming no channel and carrying the ring's latest
    identifier, when no selected channel holds anything), then the live
    subscription, with heartbeats on an idle stretch. The subscription is taken
    before the snapshot, so an event published between the two is not lost, and
    the daemon's service and the client's in-process test service both call it, so
    the snapshot-then-live-then-heartbeat order cannot drift between them. The
    item is a `Result` so a later drop policy can refuse in-stream without a
    signature change. Nothing failed an item when this ticket landed;
    `cancellation-and-slow-consumers` then put the reserved seam to use, so an item
    now fails with the slow-consumer or cancelled refusal, and that section below
    records the surface.
  - The cursor floor is the placeholder `stream-resume-and-expiry` replaces:
    `floor = cursor.max(head identifiers)`, and only live events above the floor
    are carried.
- `crates/daemon/src/service.rs`: `Service::new(store, preshared, heartbeat:
  Duration)`, the planned signature with the two-argument constructor
  `remote-pairing` landed widened by the interval, which is why
  `crates/daemon/tests/auth.rs` moves in the same commit. `HEARTBEAT_DEFAULT` is
  15 seconds, and the interval belongs to the service rather than to the serving
  loop, so a test sets a short one instead of waiting. `Service::ring() -> Ring`
  is the handle the daemon publishes its real events into, and `Service::mount(self:
  Arc<Self>, policy) -> ConnectRpcService<Router>` builds the interceptor-carrying
  service; the factory `service(store, preshared, policy)` now delegates to it, so
  a caller that needs another interval builds the `Service` and mounts it itself.
  A private `EVENT_CAPACITY` here is 1024.
  - `stream_events`: the snapshot first, then live events for the selected
    channels, with heartbeats as ordinary messages, answered as
    `Response::stream(self.ring.stream(&channels, request.last_event_id,
    self.heartbeat))`. The channels are copied out of the request body first,
    because the stream outlives that borrow. The handler checks the declaration
    itself with `identity::from_headers(ctx.headers())` and refuses one it cannot
    act on with the same typed refusal and header the handlers behind the
    interceptor produce, because the recording `local-identity-and-acl` landed is
    per handler.
- `crates/client/src/lib.rs`: `pub async fn stream_events(&self, channels: &[&str],
  last_event_id: u64) -> Result<EventStream, ClientError>`, as planned, and the
  wrapper still names no transport and no generated stub type. One measured
  addition the plan did not name: a Connect streaming response carries its status
  inside the stream, so `stream_events` reads the refusal headers on the response
  the stream was established with, and an unauthorized stream fails at the call
  that opened it instead of at the first event. The two `pub(crate)` helpers
  `refusal(err) -> ClientError` and
  `typed_refusal(&HeaderMap) -> Option<ClientError>` are where that read lives, and
  the unary, pairing and stream paths all go through them.
- `crates/client/src/stream.rs::EventStream`: `message() -> Result<Option<Event>,
  ClientError>`, yielding `api_contracts::events::Event` and nothing else. It holds
  `ServerStream<hyper::body::Incoming, EventView<'static>>` privately, so no caller
  names the transport, the generated client, or a buffa view. The module is private
  and the type is re-exported (`pub use stream::EventStream;`).
- `crates/client/src/skeleton.rs`: the stream method too, so the in-process
  service stays the one the wrapper tests and the example drive. `SkeletonDaemon`
  changed shape from a unit struct to a struct over a ring and a heartbeat
  interval, built with `new()` (its own ring at `HEARTBEAT_DEFAULT`) and the
  additive `with_ring(ring, heartbeat)`; `Default` delegates to `new()`. Its
  stream delegates to `Ring::stream`, and `RecordingDaemon` records and checks the
  declaration on the stream handler as well. The additive `serve_uds_with(path,
  ring, heartbeat)` and `router_with(ring, heartbeat)` are the start path a stream
  test and the example drive. `HEARTBEAT_DEFAULT` is the daemon's 15 seconds here
  as well, while the private `EVENT_CAPACITY` here is 64, deliberately smaller than
  the daemon's window so a drop is reachable from a test.
- `crates/client/examples/events.rs` is new; `crates/client/examples/echo.rs` is
  untouched. It serves the skeleton over a ring holding one event on the resolved
  endpoint, opens the stream through the wrapper, prints `[snapshot]`, publishes a
  second event, prints `[event]`, prints `[result] PASS`, and exits non-zero on
  failure.
- Manifests: `crates/client/Cargo.toml` takes `hyper` as a runtime dependency
  (the private stream body type) and moves `http` from `[dev-dependencies]` to
  `[dependencies]` (the refusal header map), so the wrapper's dependency footprint
  is wider than the generated client and `connectrpc` alone. The root
  `[workspace.dependencies]` gains
  `futures-util = { version = "0.3", default-features = false, features = ["std"] }`,
  `tokio`'s feature set gains `sync` for the ring's fan-out, and `api-contracts`
  names `futures-util` itself.
- `crates/api-contracts/tests/contract.rs` extended beyond the plan's letter: the
  `StreamEvents` procedure path, the byte-level field numbering of
  `StreamEventsRequest`, `StreamType::ServerStream`, and message round trips,
  consistent with the file's existing style and with the spec's contract seam.
- `docs/testing.md` gains an `Event stream` section with the four run commands (the
  ring unit tests, the daemon stream tests, the client stream test, and the
  `events` example) and the authorization property; `contracts/README.md` gains a
  sentence about the scoped lint exception. The plan named `docs/testing.md` only.

### Existing abstractions to use

Used as specified: yes.

- `serve_stream` and the private `serve_connection`, with `connectrpc`'s native
  server-streaming support: no new transport path was added, and
  `serve_tcp_listener_service` serves the stream over TCP, which is where the
  deferred assertion landed.
- `AuthPolicy` and `AuthInterceptor` with `intercept_streaming`: the stream is
  refused without a credential and served with a paired token over a real TCP
  listener, which supplies the streaming half `pairing-and-tokens` recorded as
  untested and the stream-over-TCP assertion `tcp-listener`'s criterion 4
  deferred.
- The landed test helpers and fixtures, unchanged: the transport helpers, the
  paired-token fixture and the private-path ACL helpers. The daemon stream test
  carries its own `Fixture`, `serve` and `serve_over_tcp`, and the client stream
  test its own private temporary socket base.
- `crates/client/examples/echo.rs` as the walkthrough pattern, with
  `docs/testing.md`'s command-list convention; the landed walkthrough is the new
  `crates/client/examples/events.rs`.
- `identity::from_headers` in both streaming handlers with the shared typed
  refusal, which the downstream-contracts outline required of this ticket.

### Do NOT reimplement

- Do not add resume or expiry policy, a slow-consumer policy, or the `Cancel`
  RPC. This ticket carried a cursor only; `stream-resume-and-expiry` and
  `cancellation-and-slow-consumers` give it meaning.
- Do not add a second event loop, a second ring, or a per-channel counter.
- Do not put the ring in SQLite: it is in-memory and bounded, which is the
  ticket's storage decision.
- Do not hand-roll framing or a second streaming mechanism when `connectrpc` has
  one.
- Do not change the ABI, the plugin crates, or the existing non-streaming RPCs.

### Seams

1. **Contract seam.** The generated server-streaming trait method and client
   method exist, `buf lint` is green with the one scoped exception, `buf breaking`
   against `main` is green, and `regenerate-contracts.sh --check` reports no drift.
   `crates/api-contracts/tests/contract.rs` asserts the procedure path, the
   byte-level field numbering of `StreamEventsRequest`, `StreamType::ServerStream`,
   and the message round trips.
2. **Ring seam.** Ids are monotonic and daemon-wide across channels and start at
   `1`, the ring is bounded with the oldest dropped past capacity and
   `earliest_resumable` following the event that opened the window, `latest_id()`
   and `earliest_resumable()` answer `0` on an empty ring, `snapshot` returns the
   current head per selected channel, `since` returns what follows an id, a
   subscription delivers what was published after it, and `Ring::stream` opens
   with the snapshot and then follows the ring. Unit-tested in
   `crates/api-contracts/src/events.rs` as nine tests, six `#[test]` and three
   `#[tokio::test]` (the tdd report's "seven unit tests" was a miscount, and it
   spelled the `EventKind` members unprefixed; the landed names are above).
3. **Stream seam.** Over a temporary socket, the first message is the snapshot, a
   published event follows with a greater id, a channel the stream did not select
   contributes nothing, heartbeats arrive as ordinary `HEARTBEAT` events carrying
   the latest identifier, and the identifiers never decrease. This is criterion 5.
   Driven by three tests in `crates/daemon/tests/stream.rs`, which serve the
   daemon's own service the way a listener serves it, and by two tests in
   `crates/client/tests/stream.rs`, which drive the wrapper over the skeleton's
   socket without naming a transport or a generated stub.
4. **Authorization seam.** With `TokenRequired` mounted on a TCP listener, the
   stream is refused without a credential and served with a paired token, which
   closes the streaming-interceptor gap the pairing tickets recorded. The refusal
   reaches the client as the typed `ClientError::CredentialMissing` read from
   `AUTH_ERROR_HEADER` at the call that opened the stream, so it can only come from
   the interceptor; the landed test is
   `the_stream_is_refused_without_a_credential_and_served_with_a_paired_token`,
   and it carries the stream-over-TCP assertion over
   `serve_tcp_listener_service` as well.
5. **Example seam.** The example prints the snapshot and one event, prints
   `[result] PASS`, and exits non-zero when the stream fails.
6. **Additivity seam.** The ABI layout and plugin crates are untouched, the
   harness stays at 73 checks, `Echo` and `Pair` keep their signatures and
   behaviour, and `bash scripts/regenerate-abi-header.sh --check` reports no
   drift.

Test types: unit tests in `api-contracts` for the ring; integration tests in
`crates/daemon/tests/stream.rs` and `crates/client/tests/stream.rs` over a real
socket for the stream and authorization seams; the example for its own seam; the
contract and workspace gates for the rest.

### Interface contract

- `stream-resume-and-expiry` consumes `last_event_id`, `since`,
  `earliest_resumable` and `latest_id()`; the rule that `0` means "no cursor" and
  the cursor floor `floor = cursor.max(head identifiers)`, which suppresses live
  events only, are this ticket's placeholders, and that ticket replaced them with
  real resume and expiry semantics. The proto field's doc and `Ring::stream`'s doc
  both named the placeholder, so the replacement was a documented change rather
  than a surprise. Replaced by `stream-resume-and-expiry` (merge `b43d7dd`),
  whose landed section is above: the real rule lives in `Ring::stream`, which is
  fallible at establishment, and a resumed stream replays what the ring still
  holds, carries no snapshot, and is refused through `metis-cursor-error` when
  the window has moved past its cursor.
- `cancellation-and-slow-consumers` appended the `Cancel` RPC, added no
  `EventKind` member, and owned the drop policy for a slow consumer. When this
  ticket landed the live fan-out was a `tokio::sync::broadcast` channel whose
  capacity is the ring's, and a subscriber that fell behind skipped exactly what
  the ring dropped; that was the channel's own behaviour without a policy. It has
  since landed (merge `9465e0d`) and replaced it deliberately:
  `Subscription::recv` reports the lag as `events::Live::Lagged`, and the serving
  stream turns the first one into its terminal `slow-consumer` refusal. That
  section below records the landing.
- The daemon effort publishes real events into the ring through `Service::ring()`
  and encodes payloads as JSON text. The skeleton's events are test support and
  stay minimal, and `client::skeleton::SkeletonDaemon` is the landed test-support
  surface whose shape changed here (`new()` and `with_ring()`); every in-tree
  caller moved in the same commit and the workspace is green.
- `windows-named-pipe` and `reverse-proxy-mode` inherit the stream: `serve_stream`
  serves it over their front-ends and `Ring::stream` is the one assembly, so
  neither adds framing or a second streaming mechanism.
- The three identity headers travel on the stream request, both streaming handlers
  call `identity::from_headers` and refuse with the shared typed refusal, and no
  new refusal format appears anywhere, so `local-identity-and-acl`'s per-handler
  recording obligation is paid and the daemon effort inherits a handler that
  already checks.
- Recorded residuals: heartbeats are unconditional rather than idle-detected; the
  ring is per-process and lost on restart; the slow-consumer policy was left to the
  library here, and `cancellation-and-slow-consumers` has since landed it (merge
  `9465e0d`), so a subscriber that lags is refused in-stream rather than carried
  across a gap; `last_event_id` floor-suppresses live events only; the payload
  encoding is JSON text in a `string`, a convention rather than a schema, and a
  later native encoding is a FILE-mode breaking change; the ring capacity is not
  configurable and is declared once per crate (1024 in `daemon::service`, 64 in
  `client::skeleton`, deliberately smaller there); `HEARTBEAT_DEFAULT` is declared
  twice, in `daemon::service` and `client::skeleton`, both at 15 seconds, and the
  two can drift in value; the stream tests are timing dependent (a 25 ms
  heartbeat against 5 s bounds, with no flake in three repeats); and
  `crates/client/Cargo.toml` carries a cosmetic double blank line that no gate
  reads.

---

## Landed ticket: `stream-resume-and-expiry`

Landed on `task/stream-resume-and-expiry` as the merge commit `b43d7dd`, parents
`11b6a45` (the landing branch, created at `main`'s tip because it did not exist
before this landing, the convention this spec records) and `8c5b41d` (the working
branch `ticket/stream-resume-and-expiry`, seven `wip:` commits off `main` at
`11b6a45`, deleted after the merge); the docs commit `4097e49` carries the ticket
note and the committed deviation report. The merge was clean and the landing tree
is byte for byte the verified tip (`git diff 8c5b41d b43d7dd` is empty), so no
gate was re-run at landing. The gates were green on `8c5b41d`
through `devenv shell --`: `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, the workspace `cargo test` (173 passed, 0 failed, 7 pre-existing
ignored), `cargo test -p api-contracts --lib` (34 passed), `cargo test -p daemon
--test stream` (7 passed), `cargo test -p client --test stream` (3 passed), `buf
lint`, `buf breaking --against '.git#branch=main' --against-config buf.yaml`, both
drift checks (`scripts/regenerate-contracts.sh --check` and
`scripts/regenerate-abi-header.sh --check`), the ABI harness (`[result] PASS (73
checks)`) and `cargo run -p client --example events`, which now prints
`[snapshot]`, `[event]`, `[resume] cursor=2 id=3` and `[result] PASS`. The chain's
first attempt timed out at its 900 second budget after the tdd and verify steps
and before the deviation report and the landing; the retry's tdd step re-verified
the unchanged tree at `8c5b41d` and added no commit, which is why the branch
carries seven `wip:` commits and no eighth.

Gives the landed stream's cursor a meaning, blocked by `event-stream`: a reconnect
carrying `last_event_id` replays what the ring still holds, a cursor the ring can
no longer serve earns one distinct refusal, and the client answers that refusal
with a resync, meaning a fresh cursorless stream that begins with the snapshot,
rather than with a gap, a duplicate, or a retry loop. Twelve tracked files changed
(+988/-159) against `main`.

The surfaces below are the measured ones. The deviation report found **no planned
surface differing from the actual one**; the additive and test-only details it
names are folded in below. The counts differ from the `event-stream` section's
because the daemon and client stream tests gained the cursor, expiry, reconnect
and restart cases.

### Exports

- `crates/api-contracts/src/events.rs`:
  - `Ring::stream` landed fallible at establishment: `stream(&self, channels:
    &[&str], cursor: u64, heartbeat: Duration) -> Result<impl Stream<Item =
    Result<v1::Event, ConnectError>> + Send + 'static, CursorError>`. The refusal is
    decided under the same lock that reads the window and builds the opening
    messages, so the check and the stream cannot disagree, and it returns before
    the stream opens rather than as its first item. The item type keeps its
    `Result`, which is what the slow-consumer policy owns; that policy has since
    landed with `cancellation-and-slow-consumers` (merge `9465e0d`), and an item now
    fails with its refusal rather than never failing. Its section is below.
  - `pub enum CursorError { Expired { cursor: u64, earliest_resumable: u64,
    latest_id: u64 } }`, carrying the cursor presented and both window bounds, so
    the reason is in the value and not in prose. Its `Display` names which of the
    two causes it was: the window moved past the cursor, or the daemon restarted
    and the identifier space began again. The measured cause split is
    `cursor > latest_id` for the restarted identifier space and everything else
    for the moved window. The enum is **not** `#[non_exhaustive]`, unlike
    `CursorRefusal` below: it is single-valued and internal to the daemon's
    handler, so a second way to refuse a cursor is a deliberate change rather
    than an additive one, and the wire vocabulary `CursorRefusal` is where
    additivity lives. `impl std::error::Error for CursorError` landed as an
    additive detail the approved section did not name, consistent with
    `IdentityError` and `AuthError`, so a caller can box the error.
  - `pub const CURSOR_ERROR_HEADER: &str = "metis-cursor-error";`, `pub enum
    CursorRefusal { Expired }` with `as_str`, `parse` and `Display`, and `pub fn
    cursor_refusal_headers(error: &CursorError) -> HeaderMap`, the refusal
    vocabulary in the pattern `identity::refusal_headers` and
    `auth::pair_refusal_headers` set: the meaning crosses the wire as a header and
    never as message text. `CursorRefusal` is non-exhaustive like
    `CredentialRefusal` and `PairRefusal`, and an unknown value parses to `None`,
    which leaves the caller with the transport error rather than a guess.
  - **The cursor rule**, replacing the landed placeholder: `0` means no cursor and
    is never expired; a non-zero cursor is servable exactly when `cursor + 1 >=
    earliest_resumable()` and `cursor <= latest_id()`, and expired otherwise. It
    landed as the one predicate `cursor != 0 && (cursor > latest ||
    cursor.saturating_add(1) < earliest)`, inside the lock that reads the window
    and builds the opening messages, and the saturating arithmetic is what keeps a
    cursor at `u64::MAX` a refusal rather than a wrap. The
    second clause is what makes a restart expire every cursor. The first says that
    a cursor one below the oldest held identifier is still servable, because the
    event that opened the window is exactly what follows it.
  - **What a servable cursor opens with**: `since(cursor, channels)` in identifier
    order and typed `EVENT`, then the live subscription. A stream opened without a
    cursor keeps the landed shape, the head of each selected channel typed
    `SNAPSHOT` (one empty snapshot message when the selection holds nothing), then
    the live events. A resumed stream carries **no snapshot**: its caller already
    holds the state a snapshot would restate, and restating the head would deliver
    an identifier the replay also delivers. The map's "begins with a snapshot
    message" is read as the cursorless stream's rule, which is how the map pairs it
    with "a resync is a fresh stream"; the proto comments are corrected to say so.
  - **The suppression floor**, restated: the subscription is taken before the
    window is read, so an event published between the two is not lost, and the
    floor is the highest identifier the opening messages carried (`max(head ids)`
    without a cursor, `max(cursor, last replayed id)` with one), so no identifier is
    carried twice. Heartbeats keep the landed shape and the landed identifier,
    which for a resumed stream is at or above the floor.
  - `Event`, `publish`, `snapshot`, `since`, `subscribe`, `earliest_resumable` and
    `latest_id` keep their signatures and behaviour. The ring is still in memory,
    still bounded, and still the only thing a resume can reach.
- `contracts/proto/metis/v1/daemon.proto`: **no field is added, moved, or retyped,
  so this ticket's contract change is prose only and `buf breaking` stays green.**
  Four comments stop describing the placeholder:
  - `StreamEventsRequest.last_event_id`: a non-zero cursor resumes after that
    identifier and replays what the ring still holds; a cursor the ring can no
    longer serve is refused before the stream opens, through
    `metis-cursor-error: expired`.
  - `EventKind.EVENT_KIND_SNAPSHOT`: it opens a stream that carries no cursor; a
    resumed stream opens with the events that follow its cursor instead.
  - `rpc StreamEvents` and `Event`: the "snapshot is the first message" sentence is
    narrowed the same way.
  - Because buffa emits proto comments into the generated Rust, the correction
    moves `crates/api-contracts/generated/buffa/metis.v1.rs` and
    `crates/api-contracts/generated/connect/metis.v1.rs`, so
    `scripts/regenerate-contracts.sh` runs in this ticket and its `--check`
    follows.
- `crates/daemon/src/service.rs`: `stream_events` binds the stream with `?` and
  then answers `Ok(Response::stream(stream))`, rather than the approved one-liner
  `self.ring.stream(&channels, request.last_event_id,
  self.heartbeat).map_err(refuse_cursor).map(Response::stream)`; it is the same
  value and the same mapping, written so type inference does not depend on it.
  `refuse_cursor` is a private function beside the landed `refuse_identity` and
  `refuse_pair`, in their shape: `ConnectError::new(ErrorCode::FailedPrecondition,
  error.to_string())` with `events::cursor_refusal_headers(&error)`. The handler
  body before that line is unchanged, and `EVENT_CAPACITY` and `HEARTBEAT_DEFAULT`
  are untouched. No test pins that status code end to end; see the residuals.
- `crates/client/src/skeleton.rs`: the same mapping in the private `events` helper,
  whose signature became `ServiceResult<ServiceStream<Event>>` to carry the cursor,
  so the in-process service refuses a cursor exactly as the daemon does. Both
  skeleton services call that one helper, and the private `refuse_cursor` beside
  it is the daemon's twin, same name and same status code.
  `RecordingDaemon` checks the declaration before the cursor, as the daemon's
  handler does, and the `with_ring` surface is unchanged.
- `crates/client/src/lib.rs`:
  - `stream_events(&self, channels: &[&str], last_event_id: u64)` keeps its
    signature and gains one typed failure: `ClientError::CursorExpired`, read from
    `CURSOR_ERROR_HEADER` in the landed `typed_refusal` chain, so it arrives from
    the call that opened the stream and from no other path. The variant is
    fieldless, and its `cursor_refusal` arm is the fourth in `typed_refusal`,
    after version, credential and pairing; a header value this build does not know
    stays `ClientError::Call` rather than being guessed at. Neither
    `stream_events` nor `EventStream::message` retries.
  - **The resync is the documented cursorless form of the same call**,
    `stream_events(channels, 0)`, which is a fresh stream that begins with the
    snapshot. No hidden retry is added: a caller that asked to resume learns that it
    cannot and decides.
- `crates/client/src/stream.rs`: no new method. `EventStream::message()` keeps its
  shape, and its doc gains the cursor rule a caller resumes by.
- `crates/client/examples/events.rs`: one more step. After printing the event, it
  opens a second stream carrying that event's identifier as the cursor, publishes
  a third event, prints `[resume] cursor=2 id=3` naming the cursor and the
  identifier the resumed stream delivered, and still prints `[result] PASS`. The
  approved section asked for a `[resume]` line and the usual `[result] PASS`; the
  landed line names the cursor, the identifier, the kind and the payload.
- `docs/testing.md`: the `Event stream` section gained the resume and expiry
  properties under its existing commands and its `events` example description. No
  command was added, because the ticket adds tests to files the section already
  lists.

### Existing abstractions to use

- `CursorError`, `cursor_refusal_headers` and `CursorRefusal` as the one spelling of
  the refusal, consumed by the daemon handler and the skeleton helper and read by
  the client's `typed_refusal`: the module owns both ends, as `identity`, `auth` and
  `pairing` already do.
- `Ring::since`, `Ring::earliest_resumable`, `Ring::latest_id` and the landed
  suppression floor: the replay is `since`, and the placeholders in `Ring::stream`
  and the proto field comment are the seams this ticket replaces rather than new
  machinery beside them.
- `Ring::stream`'s one assembly of a stream's shape, still the only one: the daemon
  and the skeleton both call it, so resume is implemented once and neither
  front-end can drift from the other.
- The landed refusal plumbing on both ends: `ErrorCode` plus
  `ConnectError::with_headers` on the daemon's side, and the `refusal` /
  `typed_refusal` chain plus the response-header read on the stream path on the
  client's side. No new wire mechanism is introduced.
- The landed test fixtures, with one test-only shape change: the daemon stream
  test's own `Fixture`, `serve` and `serve_over_tcp`, and the client stream test's
  own private temporary socket base. `serve` in `crates/daemon/tests/stream.rs`
  now returns its listener `JoinHandle`, so the restart test can abort the served
  service; the three earlier callers bind it to `_served`, so their behaviour is
  unchanged.

### Do NOT reimplement

- Do not add the `Cancel` RPC, a slow-consumer policy, or a way to distinguish a
  deliberate stream drop from a transport failure: those were
  `cancellation-and-slow-consumers`, which has since landed them (merge `9465e0d`),
  so the deliberate drop is now the typed `ClientError::StreamDropped` and a
  cancellation the typed `ClientError::StreamCancelled`.
- Do not add per-channel cursors, a second identifier space, a second ring, or
  cursor persistence. Resume reaches exactly as far as the in-memory window does.
- Do not add an automatic resync, a retry loop, or a backoff inside the wrapper.
  The refusal is answered by the caller, once, with a cursorless stream.
- Do not change the wire shape: no field, no `optional`, no new message. Only
  comments move, and they move through `scripts/regenerate-contracts.sh`.
- Do not change the ABI, the plugin crates, the non-streaming RPCs, or the landed
  authorization behaviour of the stream.

### Seams

1. **Ring seam (cursor rule).** Unit tests in `crates/api-contracts/src/events.rs`:
   a servable cursor replays exactly the held events after it, in identifier order
   and typed `EVENT`; `cursor == earliest_resumable() - 1` is servable; `cursor ==
   latest_id()` is servable and replays nothing; a cursor below the window and a
   cursor above `latest_id()` each answer `CursorError::Expired` carrying both
   bounds; a cursor of `0` is never expired; a live event published after the
   stream opened still arrives once. Criterion 1's unit half. Landed as three
   tests, `a_servable_cursor_replays_only_what_follows_it`,
   `the_edges_of_the_window_are_servable` and
   `a_cursor_outside_the_window_is_expired_and_names_both_bounds`, in a file that
   now holds thirteen unit tests (seven `#[test]`, six `#[tokio::test]`).
2. **Refusal seam.** A `Ring::stream` refusal maps to one `metis-cursor-error:
   expired` header, `CursorRefusal::parse` round-trips it, an unknown value parses
   to `None`, and `buf lint`, `buf breaking` against `main` and
   `regenerate-contracts.sh --check` are green with the corrected comments. The
   round trip is `the_cursor_refusal_round_trips_as_a_header`. No test pins the
   `FailedPrecondition` status code end to end: the integration tests assert the
   header and the typed client error, and the one assertion that names the code
   constructs the error inside a unit test rather than driving it over the wire.
3. **Expiry seam.** Over a temporary socket, a daemon stream opened with a cursor
   below its window is refused before any message is served, and the client reads it
   as `ClientError::CursorExpired` from the call that opened the stream; the same
   test then opens the stream with cursor `0` and asserts its first message is the
   snapshot. Criteria 2 and 4 together. Landed as
   `an_expired_cursor_is_refused_and_the_resync_opens_with_the_snapshot` in
   `crates/daemon/tests/stream.rs` and its client-side twin
   `an_expired_cursor_is_a_typed_refusal_and_the_resync_opens_with_the_snapshot` in
   `crates/client/tests/stream.rs`.
4. **Reconnect seam.** The test drains a stream, drops it, publishes while nothing
   is connected, reconnects with the highest `EVENT` identifier it saw, and asserts
   the identifiers it now receives are exactly the ones it missed: contiguous after
   the cursor, and no identifier carried twice. Criterion 3. Landed as
   `a_reconnect_resumes_without_a_gap_or_a_duplicate` in
   `crates/daemon/tests/stream.rs`.
5. **Restart seam.** A `Service` is served, a stream is opened, and the service and
   its listener are dropped; a second `Service` over the same store is served on a
   second socket, because the landed listener leaves its socket file behind, and the
   cursor earned from the first is refused as expired while a cursorless stream
   still opens with the snapshot the new, empty ring produces. Criterion 5. Landed
   as `a_restart_expires_every_cursor_and_a_cursorless_stream_starts_fresh`, which
   aborts the listener task it was handed rather than dropping it, because that is
   what drops the served service and its ring.
   The daemon's expiry and the client's expiry fixture reach their moved window by
   publishing until `earliest_resumable() > 2` under an assertion bounded at
   10 000 identifiers, rather than by restating the private `EVENT_CAPACITY`, so
   the rule now exists in two test binaries and neither reads the private
   constant. The bound keeps working if the capacity changes and fails loudly if
   the window cannot move at all.
6. **Additivity seam.** `Echo`, `Pair` and the cursorless stream keep their
   behaviour, the ABI layout and plugin crates are untouched, the harness stays at
   73 checks, and `bash scripts/regenerate-abi-header.sh --check` reports no drift.

Test types: unit tests in `api-contracts` for the cursor rule and the refusal
vocabulary; integration tests in `crates/daemon/tests/stream.rs` and
`crates/client/tests/stream.rs` over a real socket for the refusal, reconnect and
restart seams; the example for its own step; the contract and workspace gates for
the rest. The measured counts on the landing tree: `cargo test -p api-contracts
--lib` 34, of which thirteen are this section's ring and cursor unit tests,
`cargo test -p daemon --test stream` 7 (three before this ticket, four added) and
`cargo test -p client --test stream` 3 (two before, one added), against the
workspace total of 173 passed, 0 failed, 7 pre-existing ignored.

### Interface contract

- `cancellation-and-slow-consumers` consumed the shape this ticket left: a
  fallible `Ring::stream`, whose call sites thread the same `?`; the stream item
  type stays `Result` so a drop policy can refuse in-stream, which was true of this
  landing (nothing here fails an item) and is now in use, because the policy refuses
  an item where it landed; the delivered-identifier floor was not broken; and a
  dropped slow consumer got its own distinguishable signal, because `CursorExpired`
  is a refusal at establishment and not a mid-stream event. Recorded in that
  section below (merge `9465e0d`).
- `windows-named-pipe` and `reverse-proxy-mode` inherit this unchanged:
  `Ring::stream` is still the one assembly of a stream's shape, `serve_stream` is
  still the one connection path, and the refusal is an ordinary Connect error on a
  handler, which every front-end carries without knowing about it. An expired
  cursor is therefore a refusal at stream establishment on every front-end,
  without a line of their own.
- The daemon effort publishes real events into the ring through `Service::ring()`
  and encodes payloads as JSON text, and it must not treat a servable cursor as a
  durable position: the window is the only memory, so a chatty channel or a restart
  can expire a cursor.
- A caller's cursor is the highest `EVENT` identifier it has processed. A
  `SNAPSHOT` advances it only for the channels that message names, and a
  `HEARTBEAT` advances nothing, even though it carries the daemon's latest
  identifier. `last_event_id` is earned on one channel selection: a caller that
  changes its selection while resuming resyncs instead, which is the documented
  answer to "the state I hold is for other channels". That rule is untested: the
  tests only ever resume on a selection whose state the caller already holds.
- Recorded residuals: expiry is daemon-wide, not per-channel, so a chatty channel
  can expire a quiet channel's cursor and the answer is a resync; the replay is
  bounded by the ring's capacity, so the window is also the resume reach, and the
  capacity is still declared once per crate (1024 in `daemon::service`, 64 in
  `client::skeleton`) with the duplicated `HEARTBEAT_DEFAULT` and `EVENT_CAPACITY`
  constants and the non-configurable capacity carried forward; a restarted daemon
  expires every cursor with no persistence; the client offers no automatic resync
  by decision; no test pins the `FailedPrecondition` status end to end, so a later
  change of that code would keep every test green while changing what a client
  that does not read the header sees; the two expiry fixtures move their window by
  publishing under a 10 000 identifier bound rather than by naming the private
  capacity, so a much larger capacity would fail loudly instead of adapting; and
  the map's fog about distinguishing a deliberate slow-consumer drop from a
  transport failure was untouched here, because nothing this ticket lands drops a
  stream on purpose. `cancellation-and-slow-consumers` has since closed it for the
  drops the daemon causes (merge `9465e0d`), as its section below records.

---

## Landed ticket: `cancellation-and-slow-consumers`

Landed on `task/cancellation-and-slow-consumers` as the merge commit `9465e0d`,
parents `bb1bd07` (the landing branch, created at `main`'s tip because it did not
exist before this landing, the convention this spec records) and `e960712` (the
working branch `ticket/cancellation-and-slow-consumers`, eleven `wip:` commits off
`main` at `bb1bd07`, deleted after the merge); the docs commit `ad7839e` carries
the ticket note and the committed deviation report. The merge was clean and the
landing tree is byte for byte the verified tip (`git diff e960712 HEAD` is empty,
and both trees hash to `cb005d58`), so no gate was re-run at landing. The gates
were green on `e960712` through `devenv shell --`: `cargo fmt --check` clean;
`cargo clippy --all-targets -- -D warnings` clean, the one C `_FORTIFY_SOURCE`
line being the pre-existing `plugin-contract` build script rather than a Rust lint;
the workspace `cargo test` at 197 passed, 0 failed, 7 ignored, up from `main`'s 173
passed; `cargo test -p api-contracts --lib` 48; `cargo test -p daemon --test
cancel` 7; `cargo test -p daemon --test stream` 7; `cargo test -p client --test
stream` 5; `buf lint` and `buf breaking --against '.git#branch=main' --against-config
buf.yaml` green, because the `Cancel` RPC and its two messages are additive; both
drift checks (`scripts/regenerate-contracts.sh --check` and
`scripts/regenerate-abi-header.sh --check`) clean; the ABI harness at `[result]
PASS (73 checks)`, unchanged from `main`; and `cargo run -p client --example events`
printing its `[snapshot]`, `[event]`, `[resume]`, `[cancel] request_id=...
outcome=Cancelled` and `[result] PASS` steps. The verify step also repeated the two
integration-test binaries that carry the new tests, `crates/daemon/tests/cancel.rs`
(new here) and `crates/client/tests/stream.rs`, five times each with no failure on
any iteration, so the stalled-reader tests are not obviously flaky.

Adds the two guarantees the landed stream lacked, blocked by `event-stream`: a
caller can stop work it started with an explicit idempotent call naming the
request, and a caller that cannot keep up is dropped rather than silently missing
events, with the drop crossing the wire as a typed refusal the caller answers with
a resync. Nineteen source files changed (+4084/-565) against `main`; with the
ticket note and the deviation report the landing branch's diff is 21 files and
+4452/-565, the two docs files being the docs commit `ad7839e`.

The surfaces below are the measured ones. **Every planned export landed and no
named surface is missing.** The four public shapes the approved section left to the
implementer are folded in below: `events::Live` replaces `Subscription::recv`'s
landed `Option<Event>` return type; `events::cancellable(inner, guard)` is the new
public assembly the daemon and both skeleton services wrap their streams in; the
hand-written `api_contracts::cancel::CancelOutcome` mirror converts at the wire
edge; and `RequestIdError` landed non-exhaustive. The one service pair that
disagrees on an input is recorded beside the seams, not here.

### Exports

- `crates/api-contracts/src/request.rs` (new, mounted as `pub mod request;`): the
  correlation vocabulary, in the shape `identity` and `auth` set for their own
  headers.
  - `pub const REQUEST_ID_HEADER: &str = "metis-request-id";`
  - `pub struct RequestId(String)` with `parse(&str) -> Result<Self, RequestIdError>`,
    `as_str()`, `Display`, and `from_headers(&HeaderMap) -> Result<Option<RequestId>,
    RequestIdError>`, plus `Clone`/`Hash`/`PartialEq`/`Eq` because it is a registry
    key. The rule is the approved one, implemented as
    `is_ascii_graphic() && byte != b' '` (the same range as `0x21..=0x7E`): at most
    64 bytes, so an identifier is header-safe, bounded and printable. `None` means
    the call declared no request, which is legal and means the call is not
    cancellable. The generated identifier is 16 random bytes as 32 lowercase hex
    characters, a length the approved section left open.
  - `pub enum RequestIdError { Malformed }`, **`#[non_exhaustive]`**: the shape
    `CredentialRefusal`, `PairRefusal` and `CursorRefusal` already have, and the
    opposite of the single-valued `CursorError`. A malformed identifier is refused
    with a plain `InvalidArgument` and no new refusal header, because no client can
    act differently on it.
  - **The echo**: `pub struct EchoRequestId`, an `Interceptor` that copies the
    incoming identifier **verbatim, a malformed one included**, from the request
    headers into `Response::headers` on the unary and the streaming path, so the
    map's decision that correlation ids are "carried in request metadata and echoed
    verbatim in the response metadata" landed once rather than per handler. It is
    the inner layer of the daemon's chain
    (`with_interceptor(AuthInterceptor).with_interceptor(EchoRequestId)`, so a
    refused call never reaches it), and the skeleton's serve path mounts it too.
    Validation stays where the approved section put it, in the handlers, so an
    `Echo` or `Pair` call declaring an unreadable identifier is still served and
    now echoes it back: nothing new can be refused because of the echo, and a caller
    can always correlate the answer with the call it made.
- `contracts/proto/metis/v1/daemon.proto`:
  - `rpc Cancel(CancelRequest) returns (CancelResponse);` appended to
    `DaemonService`, with `string request_id = 1;` in the request and
    `CancelOutcome outcome = 1;` in the response.
  - `enum CancelOutcome { CANCEL_OUTCOME_UNSPECIFIED = 0; CANCEL_OUTCOME_CANCELLED =
    1; CANCEL_OUTCOME_ALREADY_FINISHED = 2; }`. `CANCELLED` means an in-flight
    request under that identifier was found and is being stopped;
    `ALREADY_FINISHED` means none was in flight, which covers work that finished,
    work already cancelled and work never seen, and is what makes a second call
    idempotent. One RPC, two messages and one enum are additive, so `buf breaking`
    stays green. Both generated trees were regenerated in the same commits, with
    about 1900 lines of buffa churn because `CancelRequest` and `CancelResponse`
    are declared before `message Event` and generation follows declaration order;
    `scripts/regenerate-contracts.sh --check` is clean, so the tree is reproducible
    rather than hand-edited.
- `crates/api-contracts/src/cancel.rs` (new, mounted as `pub mod cancel;`): the
  in-flight registry, in the shared crate for the reason the ring is, because the
  daemon and the client's in-process test service both need one and it is the only
  crate both reach.
  - `pub struct Registry`, cheap to clone, one per daemon, keyed by the pair of the
    declaring surface and the request identifier, so a cancel stops only work its
    own surface started. A cancel naming another surface's request finds nothing in
    flight and answers `ALREADY_FINISHED`, which is both the idempotent answer and
    no oracle.
  - `register(&self, surface: &str, id: RequestId) -> InFlight`: the RAII guard for
    one request, holding a cancellation token. The entry lives exactly as long as
    the guard, so it is removed when the work ends for any reason: finished,
    cancelled or dropped. **The duplicate rule the plan left uncased is decided:** a
    second registration under the same surface and identifier replaces the first
    entry, and the earlier guard's `Drop` removes only its own entry (checked with
    `Arc::ptr_eq`), so a drop can never unregister another registration's work. A
    caller that reuses a generated identifier therefore loses the ability to cancel
    its first piece of work, and if the newer guard drops first the older work runs
    with no entry at all. Nothing today reuses one: the wrapper generates a fresh
    identifier per stream.
  - `InFlight::cancelled(&self)` (the signal a handler's work selects on),
    `is_cancelled()`, `request_id()`.
  - `cancel(&self, surface: &str, id: &RequestId) -> CancelOutcome`: removes the
    entry, fires its token, answers `Cancelled`; with no entry, `AlreadyFinished`.
  - `in_flight(&self) -> usize` and `cancellations(&self) -> u64`, so a test can
    assert that a client's disconnect cancelled nothing.
  - `pub enum CancelOutcome { Cancelled, AlreadyFinished }` is **the hand-written
    Rust mirror of the generated wire enum**, with `to_proto()` and `from_proto()`,
    so the conversion happens at the wire edge: `Registry::cancel`, `Client::cancel`
    and both handlers pass the Rust value, and the wrapper's public API names no
    generated type, which is the property `client` has kept since `event-stream`.
    `from_proto` returns `None` for `UNSPECIFIED` and for a value a later version
    adds, and `Client::cancel` maps that to `ClientError::Call` rather than guessing.
    Dependents therefore name `api_contracts::cancel::CancelOutcome` and
    `api_contracts::request::RequestId`, because the client re-exports neither (only
    `EventStream`), exactly as they already name `api_contracts::events::Event`.
- `crates/api-contracts/src/events.rs` (extended): the drop vocabulary, beside the
  cursor vocabulary that landed there.
  - `pub const STREAM_ERROR_HEADER: &str = "metis-stream-error";`, `pub enum
    StreamRefusal { SlowConsumer, Cancelled }` with `as_str`, `parse`, `Display` and
    the non-exhaustive escape an unknown value parses to, and `pub fn
    stream_refusal_headers(&StreamError) -> HeaderMap`.
  - `pub enum StreamError { SlowConsumer { skipped: u64 }, Cancelled }`. The private
    `stream_refusal` maps `SlowConsumer` to `ResourceExhausted` and `Cancelled` to
    `Canceled`, each carrying the trailer its refusal names.
  - **The new public `Live` enum**: `Live::Event(Event)` and
    `Live::Lagged { skipped: u64 }`, so `Subscription::recv` is now `Option<Live>`
    where it was `Option<Event>`. That replaces a landed public signature rather
    than adding beside it. `api-contracts` is shared, so the daemon effort and the
    two front-end tickets must match the enum; nothing outside `Ring::stream` and
    that file's tests calls `recv` today, and the wire is unaffected. The landed
    unit test `a_subscription_delivers_published_events_in_identifier_order` changed
    shape in the same commit (a `next_event` helper), which "stops hiding a lag"
    implies but does not say, and `a_subscription_reports_what_a_lag_skipped` is new.
  - `Ring::stream`'s item type is unchanged, which is the seam `event-stream`
    reserved for this policy. `Subscription::recv` stops hiding a lag: it reports
    what the window lost, and `Ring::stream` turns the first lag into the terminal
    refusal instead of continuing with a gap. It sets a `done` flag with the
    refusal, so the poll after the refusal ends the stream, and `Next::Lagged` joins
    `Next::Event`, `Next::Heartbeat` and `Next::Ended`. The bound is the broadcast
    channel's capacity, which is the ring's window, and it stays unconfigurable.
  - **The new public assembly** `pub fn cancellable<S>(inner: S, guard:
    Option<InFlight>) -> impl Stream<Item = Result<v1::Event, ConnectError>> + Send`:
    it boxes the inner stream, selects the guard's cancellation against it (biased,
    so a message already produced wins), holds the `InFlight` guard for the stream's
    lifetime, and emits the typed `Cancelled` refusal once and then ends. The
    daemon's handler and both skeleton services wrap their streams in it, so a
    cancellation ends a stream identically in the daemon and in process. This is the
    one assembly a dependent front-end or the daemon effort must reuse rather than
    re-implement: it is where "a cancel ends its own stream" and "a dropped stream
    cancels nothing" are actually enforced.
- `crates/daemon/src/service.rs`: `Cancel` as a handler (`identity::from_headers`
  first, as every handler here does, then the registry lookup), the `stream_events`
  handler registering the stream under the declared surface and request identifier
  when the request declares one and carrying the guard inside the serving stream,
  and `Service::cancellations() -> Registry` as the handle a test or the daemon
  effort reads, named after `Service::ring()`. Two measured details: the guard is
  **registered after the cursor is served**, so a stream refused for an expired
  cursor leaves nothing in flight, and the guard travels inside the serving stream
  through `events::cancellable`, so the work is in flight exactly as long as the
  stream is. A malformed identifier is refused through a private `refuse_request_id`
  beside the landed `refuse_cursor`, as a plain `InvalidArgument` carrying no
  refusal header.
- `crates/client/src/skeleton.rs`: the same registry and the same registration in
  the in-process service, so the client tests drive cancellation and the drop policy
  without a TCP listener. The serve path changed shape beside that: `serve_uds` and
  `serve_uds_with` now call `transport::serve_uds_service` with a
  `ConnectRpcService` carrying `EchoRequestId`, through the new private `service`
  and `service_with`. `router()` and `router_with()` stay exported and unchanged, so
  the callers that serve them themselves (the identity recording tests, the duplex
  echo test) mount no echo, and the ACL is unchanged: `serve_uds` was already a
  delegation over `serve_uds_service`, and both apply `PathAcl`. The one input the
  skeleton answers differently from the daemon is recorded at seam 2.
- `crates/client/src/lib.rs`:
  - `Client::stream_events` generates a `RequestId` from the operating system's
    random source (the `getrandom` dependency the daemon already uses, the ticket's
    one new dependency edge), sends it as request metadata on that call through
    `CallOptions::try_with_header`, and the returned handle exposes it as
    `EventStream::request_id()`. Two streams opened by one client never share an
    identifier.
  - `Client::cancel(&self, request_id: &RequestId) -> Result<CancelOutcome,
    ClientError>`.
  - One typed failure per terminal refusal: `ClientError::StreamDropped` for a slow
    consumer and `ClientError::StreamCancelled` for a cancelled stream, read from
    `STREAM_ERROR_HEADER` rather than from message text. A stopped stream yields no
    further events, so a caller cannot mistake a gap for current state, and the
    documented answer is the landed cursorless resync.
- `crates/client/src/stream.rs`: `EventStream::message()` maps a terminal error
  through the refusal chain extended to trailers, because a Connect streaming
  failure carries the daemon's metadata in the end-of-stream envelope rather than in
  the response headers the stream was established with. The response-header read
  stays first, so the landed cursor and credential refusals are untouched. **The
  extension landed in `crate::refusal` and `typed_refusal`, in
  `crates/client/src/lib.rs`, not in `stream.rs`, which keeps calling it**: the
  trailer read is the second step of the shared chain, and the new `stream_refusal`
  arm is the fifth in `typed_refusal`, behind version, credential, pairing and
  cursor. `EventStream` gained the private `request_id` field and its accessor and
  nothing else. One widening worth naming: because the trailer fallback sits in the
  shared chain, a **unary** failure carrying trailer metadata now maps to a typed
  error as well. No daemon path puts a `metis-*` refusal in unary trailers today, so
  nothing observable changed, but the mapping is no longer stream-only.
- `docs/testing.md`: the `Event stream` section gained the
  `cargo test -p daemon --test cancel` command with its properties, extended the
  `cargo test -p client --test stream` entry with the two terminal refusals, and
  extended the `events` example entry with the `[cancel]` step. No command was
  removed.
- `crates/client/examples/events.rs`: one more step, `[cancel]`, cancelling the
  stream it still holds, printing the identifier and the outcome (`outcome=Cancelled`)
  and additionally asserting that the cancelled stream's next read is
  `ClientError::StreamCancelled`, where the approved section asked for a `[cancel]`
  line and the usual `[result] PASS`.
- `crates/client/Cargo.toml` and `Cargo.lock` take the `getrandom` edge the approved
  section named; the pre-existing double blank line in that manifest is still there,
  now just above `[dev-dependencies]`. Doc-only corrections beyond the list above:
  the crate docs of `api-contracts` and `client` name the new modules and errors,
  `Ring::stream`'s "nothing fails today" paragraph now names the slow-consumer
  refusal, the manifest comment about "response headers" reads "response metadata",
  and the proto file's header comment reads "Three ordinary RPCs, the event stream,
  and cancellation".

### Existing abstractions to use

Used as specified: yes.

- The landed refusal machinery on both ends, extended rather than duplicated:
  `ConnectError::with_trailers` and `stream_refusal_headers` on the daemon's side,
  and the `refusal` / `typed_refusal` chain on the client's side, which read
  response headers only and gained the trailer read. The new vocabulary follows the
  landed spelling exactly: `metis-stream-error` with the values `slow-consumer` and
  `cancelled`, a non-exhaustive parse, and `None` for a value this build does not
  know.
- The landed `Result` item seam, the suppression floor and `Ring::stream` as the one
  assembly of a stream's shape: the drop is the stream's own terminal error item,
  which is what `event-stream`'s section reserved the seam for, and the snapshot,
  cursor, suppression-floor and heartbeat behaviour is unchanged. `events::cancellable`
  is the one gate around `Ring::stream`, and `serve_stream` is still the only
  connection path.
- `identity::from_headers` and `ClientIdentity::surface()`, which is what the
  registry keys on, called first in both new handlers, with the recording preserved
  for the stream in `RecordingDaemon`; and the interceptor chain `Service::mount`
  already builds, since `with_interceptor` appends rather than replaces, so the echo
  rides beside `AuthInterceptor` rather than replacing it.
- `getrandom` as the client's source for a generated identifier, and the landed test
  fixtures: the daemon cancel test carries its own `Fixture`, `serve`, `raw_client`,
  `wait_for` and `stall_past_the_bound` rather than extracting a shared module, which
  is the repo's fixture-per-binary convention, and the client stream test keeps its
  private socket base and its own `stall_past_the_bound` copy.
- Additivity held: the ABI layout and plugin crates are untouched, the harness stays
  at 73 checks, the proto addition is additive with `buf lint` and `buf breaking`
  green, a call that declares no identifier is served as before, and `Echo`, `Pair`,
  the cursor and the expiry semantics keep their signatures and behaviour.

### Do NOT reimplement

- Do not tie a stream's lifetime to cancellation: a dropped stream stays a
  disconnect, and that it "cancels nothing by itself" is what criterion 2 asserts.
- Do not add a second buffering layer, a per-subscriber queue, or a retry inside the
  daemon. The bound is the landed broadcast window and the answer to falling behind
  is the drop.
- Do not add an automatic resync, a retry loop or a backoff in the wrapper.
- Do not add a second identifier space for requests, a handshake that registers
  identifiers up front, or cancellation RPCs per call shape.
- Do not change `Echo`, `Pair`, the cursor and expiry semantics, the ABI, or the
  plugin crates. The request header is optional, so a call that declares none
  behaves exactly as before.

### Seams

1. **Request-id seam.** Bounds and parse rules are unit-tested (empty, 65 bytes, a
   control byte, and a valid identifier round-tripping); a call that declares none
   is served exactly as before; and a declared identifier comes back in the response
   metadata over a socket, for a unary call and for the stream. Landed as the unit
   tests in `crates/api-contracts/src/request.rs` and
   `a_declared_identifier_comes_back_in_the_response_metadata` in
   `crates/daemon/tests/cancel.rs`, which drives the generated client raw so it can
   declare an identifier the wrapper's own generator would not produce, and asserts
   a call with no declaration gets no identifier back.
2. **Cancel seam.** With a stream in flight under a generated identifier, the first
   `Cancel` answers `CANCELLED` and the stream stops; the second answers
   `ALREADY_FINISHED`; an identifier that was never in flight answers
   `ALREADY_FINISHED`; and a cancel under another surface's identifier answers
   `ALREADY_FINISHED` without touching that work. This is criterion 1. Landed as
   `a_cancel_stops_the_stream_and_a_second_call_is_already_finished` and
   `a_cancel_under_another_surfaces_identifier_touches_nothing`, both driving the
   wrapper against the daemon's own service on a real socket, with
   `the_stream_carries_the_identifier_the_wrapper_declared` proving that the
   identifier on the handle is the one the daemon registered the stream's work
   under.
   **The one input the daemon and the skeleton answer differently**, recorded beside
   the seams rather than in the exports: a malformed identifier is refused as
   `InvalidArgument` on the daemon's path
   (`a_malformed_identifier_is_refused_as_invalid_argument`, the only place it is
   asserted) and is answered `AlreadyFinished` in the skeleton, whose
   `SkeletonDaemon::cancel` parses the identifier and answers the lenient outcome
   when it cannot, on the stated ground that the skeleton is declaration-free test
   support; `RecordingDaemon` is lenient about the identifier the same way. No
   client can reach the difference, because the wrapper only ever sends its own
   generated identifier, so it is test-support drift rather than a wire difference.
   The skeleton's own cancellation tests in `crates/client/tests/stream.rs`
   therefore assert the stop and the typed refusal, not the refusal of a malformed
   identifier.
3. **Disconnect seam.** The test disconnects a client mid-stream and observes the
   work continuing: a second stream under its own identifier keeps receiving events,
   a `Cancel` for the second still answers `CANCELLED` afterwards, the ring still
   holds every published event, and the registry's cancellation count is unchanged by
   the disconnect. `Cancel` for the disconnected stream's identifier answers
   `ALREADY_FINISHED`, because its work ended as finished and never as cancelled.
   This is criterion 2. Landed as `a_disconnected_client_cancels_nothing`, which
   waits on the registry for the disconnect to be noticed rather than racing it, and
   asserts `in_flight() == 2`, then `1`, and `cancellations() == 0`.
4. **Slow-consumer seam.** A reader that stops reading, followed by publishes past
   the buffer bound, gets the terminal refusal on the next read: the serving stream
   ends with `ResourceExhausted` carrying `metis-stream-error: slow-consumer`, no
   events are delivered after it, and the events it lost are named in the message
   text rather than skipped in silence. Criteria 3 and 5. Landed as
   `a_stalled_reader_is_dropped_and_the_loss_is_named` in
   `crates/daemon/tests/cancel.rs`, over the daemon's own service on a real socket,
   driving the generated client so that the status code and the trailer are both
   visible; the count is parsed out of the message rather than pinned to a number,
   and asserted positive and no greater than `latest_id()`. The approved "a reader
   that keeps up never sees the refusal, under the same publish load" half landed as
   no dedicated test, recorded below.
5. **Client seam.** The drop reaches the wrapper as the typed
   `ClientError::StreamDropped` read from the trailer metadata, not as a transport
   error and not as a clean end; the handle yields no event afterwards; and the
   cursorless resync opens with a snapshot whose heads replace the caller's stale
   state rather than appending to it. Criterion 4. Landed as
   `a_dropped_stream_is_resync_required_and_the_resync_opens_with_the_snapshot` in
   `crates/client/tests/stream.rs`, over the skeleton's socket, which also asserts
   that the drop is terminal and sticky; `a_cancelled_stream_stops_as_the_typed_refusal`
   carries the same shape for `ClientError::StreamCancelled` and asserts that the
   handle's identifier is the one the daemon registered.
6. **Additivity seam.** The ABI layout and plugin crates are untouched, the harness
   stays at 73 checks, the cursor, expiry, resume and authorization behaviour of the
   stream is unchanged, and a call with no request identifier behaves exactly as it
   did before this ticket. Two foreign test files moved only to keep the generated
   trait implementable: `crates/api-contracts/tests/contract.rs` gained the `cancel`
   arm of its `DummyDaemon` plus a contract test beyond the plan's letter (the
   procedure path, `StreamType::Unary`, the byte-level field numbering of both
   messages, the three outcome numbers, and the route set), and
   `crates/daemon/tests/auth.rs` gained a recording `cancel` arm that delegates.

Test types: unit tests in `api-contracts` for the identifier rules, the refusal
vocabulary and the registry's outcomes; integration tests over a socket in the new
`crates/daemon/tests/cancel.rs` for the cancel, disconnect and slow-consumer seams,
with the stop and the drop also asserted on the skeleton path in
`crates/client/tests/stream.rs`; the example for its own step; the contract and
workspace gates for the rest. The measured counts on the landing tree:
`cargo test -p api-contracts --lib` 48, `cargo test -p daemon --test cancel` 7 (the
new binary), `cargo test -p daemon --test stream` 7 (unchanged), `cargo test -p
client --test stream` 5 (three before this ticket, two added), against the workspace
total of 197 passed, 0 failed, 7 ignored.

The timing shape these tests rely on, recorded so a later change is deliberate: the
daemon's stalled-reader test depends on the default current-thread test runtime,
where a synchronous publish burst outruns the server's reads, so switching that test
to `flavor = "multi_thread"` would make it flaky rather than wrong; both
`stall_past_the_bound` copies (one per binary) fill the window under a 10 000
publish bound rather than naming the private ring capacity, so they keep working if
the capacity moves and fail loudly if the window cannot fill at all; `next` in
`crates/daemon/tests/cancel.rs` carries a five second bound, the bound the landed
stream tests already use; and the client stream test builds a four-slot ring so the
drop bound is reachable from a test, which is the one place a smaller-than-production
window is built for this policy.

### Interface contract

- `reverse-proxy-mode` and `windows-named-pipe` inherit all of this unchanged:
  cancellation is a handler-level RPC and the drop is an ordinary handler-level
  stream failure, so every front-end carries both without knowing about them.
- The daemon effort registers its real work in `Registry` through the handle
  `Service::cancellations()` returns, and it must not turn the drop into a retry
  inside the daemon: the answer to falling behind is the caller's resync. It also
  inherits the duplicate rule above: a registration under a surface and identifier
  already in flight replaces the earlier entry.
- The plugin line keeps its own cancellation: the plugin table's `cancel` stops one
  invoke inside an instance, while this ticket's `Cancel` names a daemon request
  identifier. The two must not be conflated, and the daemon effort is where they
  meet.
- The map's fog entry asking how a client distinguishes a deliberate slow-consumer
  drop from a transport failure is closed for the drops the daemon causes: the
  deliberate drop is the typed `ClientError::StreamDropped` and a transport failure
  stays `ClientError::Call`. What a caller should do about a transport failure stays
  where the map put it.
- A dependent that cancels or correlates names
  `api_contracts::cancel::{CancelOutcome, Registry}` and
  `api_contracts::request::{RequestId, REQUEST_ID_HEADER}`, not the generated enum
  and not the client's re-export, which is `EventStream` alone. `Subscription::recv`'s
  new `Option<Live>` return type means a caller that drained a subscription by hand
  must match `events::Live`; the wire carries no part of that change, because a lag
  is a buffer's own behaviour rather than a protocol event.
- Recorded residuals: the drop bound is the ring capacity and remains
  unconfigurable, so the daemon and the skeleton drop at different points (1024 and
  64) and the client stream test builds a four-slot ring to make the bound reachable;
  the ring is daemon-wide, so a caller behind on one channel can be dropped because
  of traffic on another; neither stalled-reader test pins a specific skipped count,
  because both parse any positive number, so a regression reporting the wrong
  magnitude would stay green while the direction (a lag is surfaced) is still
  checked; cancellation stops work but cannot undo what the work already published;
  the registry is in memory, so a restart forgets in-flight work and a cancel for a
  pre-restart identifier reports `ALREADY_FINISHED`; a cancelled stream's own end
  signal says the stream stopped, not that its work was undone; and the registry's
  duplicate rule means a caller that reuses an identifier loses the ability to
  cancel its first piece of work.
- Recorded residuals, structural: the planned "a reader that keeps up never sees the
  refusal, under the same publish load" half of seam 4 has no dedicated test; the
  trailer read sits in the shared `refusal` / `typed_refusal` chain, so a unary
  failure carrying trailer metadata now maps to a typed error as well, though no
  daemon path puts a `metis-*` refusal in unary trailers today; a malformed
  identifier is answered leniently by the skeleton where the daemon refuses it, which
  is deliberate and asserted only for the daemon; the generated buffa tree churns
  about 1900 lines because
  `CancelRequest` and `CancelResponse` are declared ahead of `message Event`; the two
  foreign test files the trait addition forced (`crates/api-contracts/tests/contract.rs`,
  `crates/daemon/tests/auth.rs`) carry the `cancel` arm so the generated trait stays
  implementable from outside the crate; and `scripts/regenerate-contracts.sh` and
  `scripts/regenerate-abi-header.sh` need `rustfmt` on `PATH` and therefore fail
  outside `devenv shell`, which is the documented usage.

---

## Pending ticket: `reverse-proxy-mode`

Blocked by the deprecated `tls-and-client-trust`, whose alternative it is, so its
blocking edge is satisfied by a done ticket. It lands the deployment shape ADR
0004 chose: the daemon stays plaintext, a reverse proxy terminates TLS in front of
it, the daemon reads forwarded information only from proxies the operator names,
and it warns honestly wherever plaintext remains.

### Exports

- `crates/api-contracts/src/proxy.rs` (new, mounted as `pub mod proxy;`): the
  trusted-proxy set, the forwarded-header rule and the exposure decision. It sits
  beside `transport` because both the listener and the daemon's handlers reach
  `api-contracts`.
  - `pub const TRUSTED_PROXIES_ENV: &str = "METIS_TRUSTED_PROXIES";`
  - `pub struct TrustedProxies(Vec<IpAddr>)` with `resolve() -> Result<Self,
    ConfigError>`, `parse(&str) -> Result<Self, ConfigError>`, `contains(&self, peer:
    IpAddr) -> bool`, `is_empty()` and `Display`. An absent or empty variable is the
    empty set, which is off by default, the shape `resolve_remote_addr` set; a
    malformed entry is `ConfigError::InvalidTrustedProxy { variable, value }`, the
    variant the non-exhaustive enum was declared to gain. Entries are exact
    addresses, comma separated, whitespace tolerated, and loopback is not trusted
    implicitly: the trust rule stays visible in configuration.
  - `pub enum ClientAddress { Peer(IpAddr), Forwarded(IpAddr) }` with `address()`
    and a `Display` naming which it is, so a test asserts where the answer came from
    and not only what it is.
  - `pub fn client_address(peer: Option<IpAddr>, trusted: &TrustedProxies, headers:
    &HeaderMap) -> Option<ClientAddress>`: the peer, or, when the peer is trusted and
    `X-Forwarded-For` is present, its left-most entry. An untrusted peer's forwarded
    headers are never read, so a hostile client cannot influence the answer, and a
    missing peer (the Unix socket) is `None`.
  - `pub enum ClientScheme { Plaintext, Encrypted }` and `pub fn client_scheme(peer,
    trusted, headers) -> ClientScheme`: `X-Forwarded-Proto: https` from a trusted peer
    is `Encrypted`; everything else, an untrusted peer included, is `Plaintext`,
    because the daemon must not claim an encryption it cannot see.
  - `pub enum Warning { PlaintextListener { addr: SocketAddr }, PlaintextSecret {
    client: Option<IpAddr> } }`, whose `Display` names what is exposed and the remedy
    (`TRUSTED_PROXIES_ENV`), with the two decisions as pure functions:
    `listener_warning(addr, trusted)`, `Some` exactly when the bind address is not
    loopback and no trusted proxy is named, and `secret_warning(scheme, client)`,
    `Some` on a plaintext client leg. Plus `pub struct WarnOnce` (an `AtomicBool`) so
    a per-request warning reaches the operator once per process rather than once per
    call.
- `crates/api-contracts/src/transport.rs`: the TCP front-ends stop discarding the
  peer. `serve_tcp_listener_service` reads `stream.peer_addr()` and stamps
  `connectrpc::PeerAddr(peer)` into each request's extensions before the mounted
  service sees it, which is what `RequestContext::peer_addr()` reads. The private
  `serve_connection` and the single accept loop keep their shape, so the interceptor
  chain stays mounted and no second connection path appears. The front-end resolves
  `TRUSTED_PROXIES_ENV` the way `serve_uds_service` resolves its ACL, and prints
  `listener_warning(local_addr, &trusted)` once at bind through the `eprintln!` this
  file already uses for connection errors. The two doc comments that say the proxy
  trust rule belongs to another ticket are corrected.
- `crates/daemon/src/service.rs`: the `Pair` handler, the one call that carries a
  secret in a body, asks `secret_warning` with the request's peer and scheme and
  emits through the service's `WarnOnce` when the client leg is plaintext. Nothing
  on the wire changes, and a loopback local call warns at most once.
- `crates/client/src/skeleton.rs`: unchanged in shape; its remote path reaches the
  same front-end, so the peer stamp and the warning come for free and the in-process
  tests can drive the rule over a real loopback listener.
- `docs/dev-env.md` gains `TRUSTED_PROXIES_ENV` beside `REMOTE_ADDR_ENV`, and
  `docs/testing.md` gains the rule and warning commands with the properties they
  assert.

### Existing abstractions to use

- `ConfigError`'s non-exhaustive enum, declared for exactly this, and
  `resolve_remote_addr`'s shape (absent or empty means off, a malformed value is a
  loud error) as the pattern for the new variable.
- The landed TCP front-ends and their one accept loop as the only place a peer
  address is known, and `connectrpc::PeerAddr` with `RequestContext::peer_addr()`
  rather than a hand-rolled extension type the library would not read.
- `PathAcl::from_env()` in the Unix front-end as the precedent for reading listener
  configuration where the listener binds, and the file's existing `eprintln!` for its
  own diagnostics.
- `identity::from_headers` and the existing per-handler header reads as the shape for
  reading forwarded headers, and the landed test fixtures: the daemon stream and
  auth tests' `serve_over_tcp` and `Fixture`, and the client remote test's loopback
  listener.

### Do NOT reimplement

- Do not terminate TLS, add a certificate, key or trusted-CA setting, or present a
  fingerprint (ADR 0004). No rustls, no tower-http.
- Do not trust a forwarded header from an unnamed peer, and do not walk a multi-hop
  `X-Forwarded-For` chain: the daemon takes the left-most entry from a single trusted
  hop and nothing else.
- Do not add a second connection path, a second service or a second accept loop; the
  peer stamp goes into the landed front-end.
- Do not add rate limiting, IP allow-listing or a new authorization policy. The
  trusted-proxy list decides only whether forwarded information is read.
- Do not change the wire contract, the identity headers, the credential, the event
  stream, the ABI, or the plugin crates.

### Seams

1. **Configuration seam.** Absent and empty mean no trusted proxies;
   comma-separated literals parse with whitespace tolerated; a malformed entry is
   `ConfigError::InvalidTrustedProxy` naming variable and value; `contains` is
   exact-address and does not trust loopback implicitly.
2. **Forwarded-rule seam (criteria 1 and 3).** With the peer named and
   `X-Forwarded-For: 203.0.113.7`, the client address is `Forwarded(203.0.113.7)`;
   with the same header from an unnamed peer it is `Peer(peer)`; an unnamed peer's
   `X-Forwarded-Proto: https` leaves the scheme `Plaintext` while a named peer's
   `https` makes it `Encrypted`; with no peer at all the answer is `None` and nothing
   is forwarded.
3. **Peer-stamp seam.** Over a real loopback listener through the landed
   service-taking front-end, a handler reads `ctx.peer_addr()` as `Some(loopback)`,
   which is what makes the rule reachable end to end; over the Unix socket it is
   `None`.
4. **Warning seam (criterion 2).** `listener_warning` on a non-loopback address with
   no proxy named is `Some`, the same address with a proxy named is `None`, and a
   loopback address with none named is `None`; a plaintext client leg on the
   secret-bearing call is `Some` and an `Encrypted` one is `None`; and `WarnOnce`
   allows exactly one emission.
5. **Additivity seam.** The ABI and plugin crates are untouched at 73 checks, the
   contract is unchanged, the local socket path is unaffected, `Pair` behaves
   identically over a loopback listener, and the workspace suite stays green.

Test types: unit tests in `api-contracts` for the configuration, the forwarded rule,
the scheme rule and the warning decisions; integration tests over a real loopback
listener in the daemon's test suite for the peer stamp and the
accepted-versus-ignored assertion; the existing client remote test for the in-process
path; the contract and workspace gates for the rest.

### Interface contract

- `windows-named-pipe` inherits the rule as an `Option<IpAddr>` peer, so a named pipe
  passes `None` and forwards nothing unless that ticket decides otherwise. It adds no
  proxy notion of its own.
- The daemon effort starts its listener through the landed front-ends, so it inherits
  the bind-time warning and the peer stamp; it must never use a forwarded header for
  authorization, only for exposure and, if it records one, for its audit record.
- The map's deferred question of what a client should verify about a proxied endpoint
  stays deferred with no owner (ADR 0004), and this ticket does not answer it.
- Recorded residuals: the daemon cannot detect an operator who exposes the listener
  without a proxy; the stamp covers the landed TCP front-ends only; the warning is
  stderr text behind a once-per-process latch with no logging framework; a multi-hop
  chain is not walked, so the left-most entry is only the client when the first hop
  is honest; and `X-Forwarded-*` is a convention rather than a schema, so a proxy
  that sets none leaves the daemon believing plaintext.

---

## Downstream interface contracts (outline)

The only ticket still pending in this effort is `windows-named-pipe`. Its section is
added when its chain reaches the frontier, and every ticket landed so far has one;
the commitments below are what the landed tickets must already support, including
the surfaces `local-identity-and-acl`, `state-store`, `pairing-and-tokens`,
`remote-pairing`, `event-stream`, `stream-resume-and-expiry` and
`cancellation-and-slow-consumers` landed.

- `event-stream`, `stream-resume-and-expiry`, `cancellation-and-slow-consumers`:
  append `StreamEvents` and `Cancel` to `metis.v1`, with `last_event_id` in the
  request, the snapshot as the first message of a stream that carries no cursor,
  heartbeats as ordinary messages, and correlation ids in request metadata. The
  `RPC_RESPONSE_STANDARD_NAME` lint exception is added here, and the contract
  regenerates through `scripts/regenerate-contracts.sh`. `EventKind` is the
  contract's first enum, the one the buffa mount's dormant
  `clippy::derivable_impls` allow was reserved for. `event-stream` landed and
  paid the rest of this bullet: the stream-over-TCP assertion `tcp-listener`'s
  acceptance criterion 4 deferred, carried over the landed TCP front-ends because
  `serve_stream` serves any Connect service over the stream it is handed, and the
  identity declaration on the stream, because the recording
  `local-identity-and-acl` landed is per handler and a `StreamEvents` handler that
  skips the call is unrecorded rather than refused.
  `stream-resume-and-expiry` landed the cursor's meaning on top of that
  placeholder, and it narrowed the snapshot sentence above: `Ring::stream` is
  fallible at establishment, a servable cursor replays what the ring still holds
  and carries no snapshot, a cursor the ring can no longer serve is refused before
  the stream opens through `metis-cursor-error: expired`, and the client's
  `ClientError::CursorExpired` leaves the answer to the caller as a cursorless
  stream. `cancellation-and-slow-consumers` therefore consumed the fallible
  `Ring::stream`, threading the same `?` at its call sites, and kept the item
  type's `Result` for its drop policy, which that landing now uses: the `Cancel`
  RPC and its outcome enum landed, a caller's identifier travels as
  `metis-request-id` request metadata and is echoed verbatim in the response
  metadata by one interceptor, and an item now fails with the `slow-consumer` or
  `cancelled` refusal where a caller fell behind or its work was cancelled. Its
  landed section is above.
- `windows-named-pipe`, `reverse-proxy-mode`: extend
  the same `transport` module with their own bind and connector front-ends; no
  second service implementation. `tls-and-client-trust` is deprecated (ADR
  0004), so no `transport` front-end terminates TLS.
  `serve_stream` stays the only connection path
  and the landed `serve_tcp_listener` is the shape a new accept loop copies:
  build the router once, wrap it in an `Arc`, and clone the `Arc` per accepted
  stream, because `serve_stream` takes `router: impl Into<Arc<Router>>`
  (`connectrpc::Router` is not `Clone` in 0.9.1). A front-end that must report a
  kernel-assigned port takes an already bound listener, as `serve_tcp_listener`
  does; a front-end that must serve an interceptor-carrying service takes a
  `ConnectRpcService<Router>` instead, the shape `serve_uds_service` landed, and
  `serve-tcp-service` landed that TCP counterpart as
  `serve_tcp_listener_service` and `serve_tcp_service`, so the earlier exception
  is closed. A front-end that resolves a non-default authority
  names it through a helper beside `local_authority` and `remote_authority`; new
  settings and bind failures join the non-exhaustive `ConfigError`. Because
  `Client` builds its own transport and exposes no injection point, each of
  these tickets extends `client` with a constructor for its own transport
  instead of injecting one from outside; on non-Unix there is no local
  constructor and no `serve_uds`
  ACL until `windows-named-pipe` lands, because the landed socket rule is
  Unix-only and a named pipe carries its own access rule rather than
  `METIS_SOCKET_GROUP`. Both inherit the cursor refusal unchanged: it is an
  ordinary Connect error raised by `Ring::stream` at stream establishment, so a
  front-end carries it without a line of its own.
- `pairing-and-tokens` landed the credential path this bullet reserved, and
  `remote-pairing` consumes it: request metadata and RPCs under `metis.v1`,
  credentials as `Authorization` metadata on every call, including the stream, on
  the channel `local-identity-and-acl` landed (the three identity headers plus the
  `metis-supported-versions` refusal header under `api_contracts::identity`) and
  now the credential vocabulary under `api_contracts::auth`
  (`AUTHORIZATION_HEADER`, `AUTH_ERROR_HEADER`, `CredentialRefusal`, `bearer`,
  `bearer_token`). It owns no second credential path. That constraint is closed:
  the landed `serve-tcp-service` mounts the interceptor-carrying
  `ConnectRpcService` that `service(...)` returns, through `serve_tcp_service`
  and `serve_tcp_listener_service`. The token path the landed ticket built is
  the commitment it kept: `Store::open`, `mint`,
  `insert_token(surface, &TokenHash)`, `verify_token` and `revoke_token`
  with the surface name as a `&str`, `Verify::UnknownSurface` and
  `Verify::Mismatch` mapped onto `RejectionCause::Revoked` and
  `RejectionCause::Invalid`, `TokenError::Malformed` from `Token::parse` turned
  into a typed refusal, and the presented credential hashed exactly as the store
  does: SHA-256 over the token's canonical lowercase hex text, the rule the
  known-answer digest in the `state-store` section pins. A client that hashed the
  decoded bytes would pair successfully and then fail every later call, so the
  rule is named here as well as there.

## Architecture notes

- The two landed tickets were independent (no `blocked_by` edge between them),
  so they ran in level 0 sequentially on the shared repo cwd, in the order
  `tw_dependency_levels` returned, and `plugin-abi-core` landed on
  `contract-pipeline`'s landing branch.
- `contract-gate-hardening` is `blocked_by` `contract-pipeline` and edits the
  pipeline that ticket landed, so it landed on the same chain (merge `3612e0f`),
  on top of the two level-0 tickets.
- Both tickets touch the workspace root (`Cargo.toml`, `devenv.nix`,
  `.github/workflows/ci.yml`) and `crates/`, which is why they could not run in
  parallel and why the later chains landed on the first chain's landing branch.
- `contract-pipeline` adds the first committed generated tree in the repository,
  so the fmt and clippy allowances it establishes, and the
  `normalize_comments=true` rustfmt post-step, are the precedent every later
  generated tree follows.
- `plugin-abi-core` is the first place the repository compiles C. The `cc`
  build-dependency plus `-std=c11` is the measured minimum; the C compiler is
  already in the devenv toolchain.
- `contract-gate-hardening` moved the buf configuration rather than adding a
  second one, so the plugin pins, the lint policy, the committed generated
  trees, and the verdict for an unchanged contract are all unchanged. Its seams
  are command-level, so the acceptance evidence lives in
  `scripts/probe-contract-gates.sh` rather than in a new Rust test.
- The plugin fixture adds a workspace member that exists only to prove the
  boundary. That is deliberate and is the shape the effort spec's plugin ABI
  seam describes. The harness is the owner of the host-callback panic probe and
  of the library rebuild, so no consumer of the fixture can load a stale or
  unwrapped library.
- `local-transport-rpc-skeleton` was the third level-1 chain and has landed
  (merge `d886567`), so the tickets then still pending were
  `plugin-authoring-trait` and `plugin-compatibility`, each level 1 and blocked
  only by the landed `plugin-abi-core`. Both edit
  `crates/plugin-fixture/src/lib.rs`, its harness, and the fixture manifest, so
  they cannot run in parallel on the shared cwd: they land in one order and the
  later one rebases onto the earlier one. The local-transport chain added the
  workspace `tokio`, `tower`, `hyper`, and `hyper-util` entries and filled
  `crates/client`, which the two remaining chains do not read.
- `local-transport-rpc-skeleton` deviates from the spike in exactly one place,
  forced by the dependency: `Router` is not `Clone` in connectrpc 0.9.1, so the
  accept loop shares one router through an `Arc` and the service is built with
  `ConnectRpcService::from_arc`. The landed section above records that and the
  three other measured divergences (two additive public functions, no transport
  injection point, the fixed endpoint literals).
- `plugin-contract-v1` landed as the merge commit `83c0660` on
  `task/plugin-contract-v1`, on top of every earlier landing merge, plus
  `0008ba4` carrying the ticket note and the committed deviation report. It
  revises two landed surfaces rather than adding a crate, so after it the only
  ticket still pending in the plugin chain was `plugin-compatibility`; the other
  pending tickets in the effort (`event-stream`, `local-identity-and-acl`,
  `tcp-listener`, `windows-named-pipe`) read the contract and transport
  surfaces, which this ticket left unchanged.
- `plugin-compatibility` landed as the merge commit `c9227a4` on
  `task/plugin-compatibility`, with the landing record and this section's update
  in `c1d95d2`. It relaxes the minor comparison to one direction
  (`prefix.abi_minor < METIS_ABI_MINOR`), drives the major seam with a second
  compiled library, and takes the fixture to eight capabilities with
  `tool.newer` gated at descriptor version 2, which moved the harness baseline to
  73 checks. The tickets still pending in the effort (`event-stream`,
  `local-identity-and-acl`, `tcp-listener`, `windows-named-pipe`) read the
  contract and transport surfaces, which this ticket left unchanged, so no
  pending chain builds against the changed plugin surfaces.
- `tcp-listener` landed as the merge commit `c2e8684` on `task/tcp-listener`,
  with the ticket note and the committed deviation report in `6bbf8a7`. It
  extends the landed transport module with the TCP front-ends and adds no second
  serve loop, so the three tickets still pending in the effort (`event-stream`,
  `local-identity-and-acl`, `windows-named-pipe`) read the same `serve_stream`
  seam; `event-stream` additionally reads the landed TCP front-ends for the
  stream-over-TCP assertion criterion 4 deferred. The earlier notes in this
  section list `tcp-listener` as still pending at their own landing moment; this
  note carries the current set.
- `local-identity-and-acl` landed as the merge commit `694fe0c` on
  `task/local-identity-and-acl`, with the ticket note and the committed deviation
  report in `8e56620`. It moves the endpoint default off `/tmp`, applies the
  socket ACL in `serve_uds` to the socket file as well as its directory, and adds
  the identity declaration and the version-refusal channel; it adds no credential
  and changes no contract, generated tree, ABI layout, or plugin surface. The
  tickets still pending in the effort are `event-stream` and
  `windows-named-pipe`, both of which read the landed transport and identity
  surfaces; the earlier notes in this section list `local-identity-and-acl` as
  still pending at their own landing moment, and this note carries the current
  set.
- `state-store` landed as the merge commit `925371a` on `task/state-store`, with
  the ticket note and the committed deviation report in the docs commit
  `e824546`. It gives `crates/daemon` a library target and the store and token
  modules beside its binary, keeps the store decoupled from
  `api_contracts::identity` by taking surface names as `&str`, pins the
  hash-input rule the pairing chain must match, and generalises the landed
  private-path rule into `transport::ensure_private_dir` and
  `transport::restrict_file`. The tickets still pending in the effort are
  `pairing-and-tokens` (unblocked by this landing), `event-stream`,
  `reverse-proxy-mode`, `windows-named-pipe` and `remote-pairing`; the earlier
  notes in this section carry the pending set at their own landing moment, and
  this note carries the current set.
- `pairing-and-tokens` landed as the merge commit `1c89660` on
  `task/pairing-and-tokens`, with the ticket note and the committed deviation
  report in the docs commit `3c91ea0`. It is the first ticket to append to the
  wire contract since the skeleton and the first to carry a credential, and it
  settles the listener question for the remaining transport tickets: because
  connectrpc 0.9.1 keeps an interceptor chain on `ConnectRpcService` and offers
  no conversion back to a `Router`, a listener mounts a service rather than a
  router, so the Unix front-end is `transport::serve_uds_service` and the landed
  TCP front-ends, which take a bare `Router`, cannot carry the policy. That gap
  is the `serve-tcp-service` ticket on `main` (`97e0a91`), which `remote-pairing`
  is blocked by; the deviation report named it before the landing. The tickets
  still pending in the effort at that landing moment were `serve-tcp-service`,
  `event-stream`, `reverse-proxy-mode`, `windows-named-pipe` and `remote-pairing`;
  the earlier notes in this section carry the pending set at their own landing
  moment, and this note carries the set then.
- `serve-tcp-service` landed as the merge commit `4f57d3d` on
  `task/serve-tcp-service`, parents `fb36073` (the landing branch, created at
  `main`'s tip) and `c4c2c16` (the working branch, seven `wip:` commits), with the
  ticket note and the committed deviation report in the docs commit `5ef1042`.
  It closes the hole the pairing note records: a TCP listener can now mount the
  interceptor-carrying service through `serve_tcp_listener_service` and
  `serve_tcp_service`, so `remote-pairing` is unblocked. The tickets still
  pending in the effort are `event-stream`, `reverse-proxy-mode`,
  `windows-named-pipe` and `remote-pairing`; the earlier notes in this section
  carry the pending set at their own landing moment, and this note carries the
  current set.
- `remote-pairing` landed as the merge commit `adb4c5d` on `task/remote-pairing`,
  parents `fd65ccc` (the landing branch, created at `main`'s tip) and `4651798`
  (the working branch `ticket/remote-pairing`, four `wip:` commits off `main` at
  `fd65ccc`, deleted after the merge), with the ticket note and the committed
  deviation report in the docs commit `1d29511`. It closes the residual the
  `pairing-and-tokens` note recorded: the client reads a typed pairing refusal
  from the `metis-pair-error` response header, so a wrong preshared key no longer
  arrives as a `Call` carrying `PermissionDenied`. The tickets still pending in
  the effort are `event-stream`, `reverse-proxy-mode` and `windows-named-pipe`;
  the earlier notes in this section carry the pending set at their own landing
  moment, and this note carries the current set.
- `event-stream` landed as the merge commit `0727964` on `task/event-stream`,
  parents `4235803` (the landing branch, created at `main`'s tip because it did
  not exist before this landing) and `64bd7d5` (the working branch
  `ticket/event-stream`, seven `wip:` commits off `main` at `4235803`, deleted
  after the merge), with the ticket note and the committed deviation report in
  the docs commit `f765728`. It adds the contract's first streaming RPC and its
  first enum, the daemon-wide event ring in `api-contracts`, and the client's
  stream. It pays two landed debts at the wire: the streaming half of the
  interceptor, which `pairing-and-tokens` could prove only by construction, and
  the stream-over-TCP assertion `tcp-listener`'s criterion 4 deferred, both
  asserted in `crates/daemon/tests/stream.rs` over a real TCP listener. Its
  handlers check the identity declaration themselves, so the per-handler recording
  obligation `local-identity-and-acl` recorded is paid, and its payload is JSON
  text in a `string` field, which the two realtime tickets still to come inherit.
  `Service::new` gained the heartbeat interval, which moved
  `crates/daemon/tests/auth.rs` in the same commit. The tickets still pending in
  the effort are `reverse-proxy-mode` and `windows-named-pipe`; the earlier notes
  in this section carry the pending set at their own landing moment, and this note
  carries the current set.
- `stream-resume-and-expiry` landed as the merge commit `b43d7dd` on
  `task/stream-resume-and-expiry`, parents `11b6a45` (the landing branch, created
  at `main`'s tip because it did not exist before this landing) and `8c5b41d`
  (the working branch `ticket/stream-resume-and-expiry`, seven `wip:` commits off
  `main` at `11b6a45`, deleted after the merge), with the ticket note and the
  committed deviation report in the docs commit `4097e49`. It replaces the cursor
  placeholder `event-stream` landed with the real rule and makes `Ring::stream`
  fallible at establishment, so the daemon's handler and the skeleton's helper
  each map a refusal through their own private `refuse_cursor`. It adds no proto
  field: the contract change is four corrected comments, and the two generated
  trees moved with them. The tickets still pending in the effort are
  `cancellation-and-slow-consumers`, `reverse-proxy-mode` and
  `windows-named-pipe`; the earlier notes in this section carry the pending set at
  their own landing moment, and this note carries the current set.
- `cancellation-and-slow-consumers` landed as the merge commit `9465e0d` on
  `task/cancellation-and-slow-consumers`, parents `bb1bd07` (the landing branch,
  created at `main`'s tip because it did not exist before this landing) and
  `e960712` (the working branch `ticket/cancellation-and-slow-consumers`, eleven
  `wip:` commits off `main` at `bb1bd07`, deleted after the merge), with the ticket
  note and the committed deviation report in the docs commit `ad7839e`. It adds the
  contract's `Cancel` unary RPC, the request-identifier channel with its echo
  interceptor, the in-flight registry with its RAII guard, and the
  `metis-stream-error` trailer vocabulary, and it makes `Subscription::recv` report
  a lag as the new public `events::Live` enum rather than hiding it. The one service
  pair it leaves disagreeing answers a malformed identifier differently, which its
  section records beside the seams. The tickets still pending in the effort are
  `reverse-proxy-mode` and `windows-named-pipe`, both of which inherit this
  unchanged: cancellation is a handler-level RPC and the drop is an ordinary
  handler-level stream failure. The earlier notes in this section carry the pending
  set at their own landing moment, and this note carries the current set.

---

## Landed ticket: `plugin-contract-v1`

Landed on `task/plugin-contract-v1` as the merge commit `83c0660`, parents
`c1650a5` (the landing branch, created at this ticket's arch spec commit, which
was also `main`'s tip) and `af09b33` (the working branch
`ticket/plugin-contract-v1`, deleted after the merge), followed by `0008ba4`,
which carries the ticket note and the committed deviation report. The merge was
clean, and apart from those two files the landing tree is byte-identical to
`af09b33`, so no gate was re-run at landing; all of them were green on that
tree: the full workspace `cargo test` (46 passed), `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`,
`cargo test -p plugin-contract --features authoring` (33 passed),
`cargo test -p plugin-runtime` (6 passed), the harness `[result] PASS
(62 checks)`, the gcc and clang C builds, `cargo check -p plugin-contract
--no-default-features`, and `bash scripts/regenerate-abi-header.sh --check`.

Implements the approved contract,
`docs/plugin-contract.md`: the revised v1 ABI where a future is the only call
path, one capability-name-routed authoring method, and the host driver and
registry. It supersedes the `invoke`/`complete` half of `plugin-abi-core` and the
four-arm authoring surface of `plugin-authoring-trait`, both landed, and it takes
over the presence predicate the `plugin-compatibility` section used to own. The
decision record is `docs/adr/0002-plugin-contract-futures-only.md`.

The two divergences the implementation itself folded in, both measured, are
named here: the presence predicates could not be emitted as functions, so the
header carries the slot constants the predicate is written in terms of; and
`plugin-runtime` parses the capability declaration with its own type rather than
turning the `authoring` feature on, so the host keeps the primitives and the
envelope alone. The rest of the deviation report's divergences (`Accounting`'s
field set, `MetisFutureHandle`'s `Copy`, the named runtime surface, and the C
build mechanism) are reconciled at their entries below.

### Exports

- `crates/plugin-contract/src/abi/mod.rs`, the revised tables.
  - Plugin table, eight pointer entries: `size`, `abi_major`, `abi_minor`,
    `create_instance`, `destroy_instance`, `list_capabilities`, `invoke_future`,
    `cancel`, `release_buffer`, `shutdown`, `service_complete`. 80 bytes on
    64-bit. `MetisInvokeFn` and the plugin table's `invoke` slot are deleted, the
    type stays `MetisPluginTableV1`, the symbol stays `metis_plugin_entry_v1`,
    `METIS_ABI_MAJOR` stays 1, and `METIS_ABI_MINOR` becomes 1.
  - Host table, five pointer entries: `size`, `user_data`, `progress`,
    `emit_event`, `log`, `service_request`. 48 bytes on 64-bit. `complete` and
    `MetisCompleteFn` are deleted.
  - `MetisFutureHandle { ctx, vtable }`, `MetisFutureVtable { poll, drop }` with
    `MetisFuturePollFn` and `MetisFutureDropFn`, `MetisWaker { data, vtable }`,
    and `MetisWakerVtable { clone, wake, wake_by_ref, drop }` with its four
    aliases. All `repr(C)`, all fields pointers or function pointers, and
    neither vtable carries a `size` field. `MetisFutureHandle` derives `Clone`
    and `Copy`, documented in the crate as what a two-field C struct already is;
    the derive is additive, no consumer changes, and the exactly-once drop rule
    is unchanged.
  - `MetisServiceRequestFn` and the per-entry slot constants
    (`METIS_PLUGIN_TABLE_V1_*_SLOT`, `METIS_HOST_API_V1_*_SLOT`, and the two
    `*_SLOTS` counts), plus the `const fn` presence predicates
    (`metis_plugin_table_has_invoke_future`, `metis_plugin_table_has_service_complete`,
    `metis_host_api_has_service_request`) and the byte thresholds they compare
    against.
- `crates/plugin-contract/src/abi/version.rs`: `MetisAbiError` with
  `Unreadable`, `MajorMismatch { expected, found }`, `MinorMismatch { known,
  found }` and `EntryMissing { entry }`, plus `metis_read_table`, which reads the
  prefix first, refuses a major that is not 1, refuses a minor that is not 1 in
  either direction, and then names the first entry the reported `size` does not
  cover.
- `crates/plugin-contract/src/abi/layout.rs` and
  `c/layout_static_asserts.c`: the same numbers, asserted on every build, for
  the revised plugin and host tables and for all four new types. The C side also
  asserts the presence rule in the form the header documents
  (`size >= sizeof(prefix) + (slot + 1) * sizeof(void *)`).
- `crates/plugin-contract/include/metis_plugin_abi.h`: regenerated, never hand
  edited. It gains the new types, the six payload envelope constants (which
  moved out of the cbindgen `exclude` list because the contract puts them in the
  header), and the slot constants. `bash scripts/regenerate-abi-header.sh
  --check` exits 0.
- `crates/plugin-contract/src/authoring/`, under the `authoring` feature.
  - `Plugin` is `capabilities`, `call`, `cancel`, `started`, `stopped`, with
    `Request<'a> { capability, body }` and `Response { body }`. The four arms,
    their four capability constants, their four associated `Shape` types and the
    `Shape` trait are deleted.
  - `future.rs` is the boxed-future helper that replaces `async-ffi`: it erases
    the author's future, bridges the host waker into `std::task::Waker`
    (`OwnedWaker`, one vtable clone per poll), contains a panic in both `poll`
    and `drop`, records a double drop and a second poll after `Ok` as
    violations, and returns `Internal` for a panic rather than reaching
    `process::abort`.
  - `capability.rs`: `CapabilityDescriptor` carries `request_schema` and
    `response_schema` inline, so a host needs no second call to learn what a
    capability accepts and an author derives them through the re-export below.
    The envelope is unchanged, because a schema and the JSON text it serialises
    to are the same JSON object.

    **Amended after landing.** The declaration was then unified into one ungated
    type in `crates/plugin-contract/src/capability.rs`, with
    `serde_json::Value` schemas and a `kind: String` plus the `CapabilityKind`
    helper. `authoring` re-exports it and adds `schema_of::<T>()`, so `schemars`
    stays optional. `plugin-runtime` deleted its own `Capability`,
    `DeclaredCapability` and `CapabilityList` and parses the shared type, so the
    drift test is gone and the two sides cannot drift. A kind this build does
    not know is refused per capability through `RouteError::UnknownKind` with
    the `Unsupported` status, which is why `kind` is a string rather than an
    enum: a strict enum would fail the whole declaration. The wire is
    byte-identical.
  - `call.rs` adds the service route: `Call::service_request` builds the
    plugin-owned request buffer, registers a slot, calls the host's
    `service_request`, and returns a `ServiceAnswer<R>` future the author
    awaits. The adapter's `service_complete` entry resolves that slot and wakes
    it. Presence is checked with `metis_host_api_has_service_request` before the
    route is used.
  - Deleted: `authoring/executor.rs` with its bounded pool and thread-per-call
    fallback, and `authoring/host_adapter.rs` with its pending-slot machinery and
    its `Await` type. Nothing in the authoring module spawns or blocks.
  - `schemars` joins the feature's dependencies (`dep:schemars`), and the
    authoring module re-exports it so a plugin needs no direct dependency.
- `crates/plugin-runtime/src/lib.rs`: the host side, no longer empty.
  - `Loaded::load` (unsafe: the library is native code), `abi()`, `table()`,
    `has_service_complete()`, and `create_instance`, which binds the plugin's
    `release_buffer` to the host and reads the declaration once.
  - `Host`: the callback table with all four callbacks installed and each one
    catching its own panic, plus `logs()`, `events()`, `pending_services()`,
    `take_service()`, `waker_counts()` and `ordering()`. A pending service is a
    `PendingService`, the service request `pending_services()` lists and
    `take_service()` hands to the host so it can answer it through
    `Instance::complete_service`.
  - `Instance`: `capabilities()`, `invoke` (which refuses a quarantined instance
    and an undeclared capability), `cancel`, `complete_service`, `shutdown`,
    `quarantine`, `is_quarantined`, `in_flight`, and `unload`, which refuses
    while any future is alive and records both refusals in the ordering ledger.
  - `PluginCall`: a `Future<Output = Result<Vec<u8>, CallError>>` over the plugin
    future, with a host-side waker (`CallState`, counted clones and releases, a
    recorded wake after a drop) and a `Drop` that calls the plugin's `drop`
    exactly once. `CallError` carries the version failure as
    `Version(MetisAbiError)` with a `From<MetisAbiError>`, so the runtime
    inherits the crate's version error rather than inventing its own, which is
    the inheritance the `plugin-compatibility` section planned for it.
  - `registry.rs`: `Entry`, `Registry`, `RegistryError`, `RouteError`,
    `parse_capabilities`, `validate_request`, `validate_response`, and the
    `call` launch-and-await method. The declaration it parses and routes with is
    the shared `plugin_contract::capability::CapabilityDescriptor` (see the
    amendment above), so the host holds no second declaration type and no test
    is needed to keep two of them in step. Routing is by
    instance-qualified name
    (`<instance>.<tool>`), duplicate names are refused, `enable` and `disable`
    toggle an entry, `unregister` removes it and its routes, `gate` is the
    per-capability version check, and validation runs in both directions with
    `jsonschema` (`default-features = false`, because the defaults pull in
    `reqwest` and `rustls`).
  - `CallError`, `RouteError` and `RegistryError` cover the host path's failures
    today. They realise part of the effort's error taxonomy ahead of the open
    item that would decide it, so that item stays open for a deliberate review
    rather than being settled by this ticket.
- `crates/plugin-fixture`: the fixture rewritten through the routed trait with
  seven capabilities (`tool.echo`, `tool.pending`, `tool.cancellable`,
  `tool.stubborn`, `tool.panic`, `tool.panic_drop`, `tool.blocking`), the
  allocation ledger `Accounting { allocs, frees, live, violations }`, and the
  same one exported `metis_*` symbol. The `suppressed` field `plugin-abi-core`
  shipped is gone, because the rule it counted died with the completion
  callback; only the fixture and the harness ever built or destructured it, and
  both were updated.
- `crates/plugin-fixture/src/bin/abi-harness.rs`: the harness drives the futures
  path end to end and prints one `[ok]`/`[fail]` line per check plus the
  teardown ordering, with the repository's `[result] PASS`/`FAIL` summary line
  carrying the count, flushing per line and exiting non-zero on any failure. The
  Rust path was recorded at 43 checks while the C section was being added; the
  landing run's total is 62, measured as `[result] PASS (62 checks)`.
- `crates/plugin-fixture/c/c-plugin.c` and `scripts/build-c-plugin.sh`: the C
  implementer of the table, compiled with `-std=c11 -Wall -Wextra -Werror` under
  both gcc and clang into `target/c-plugin/libcplugin.so`. It is built by a
  script plus a CI step rather than by a `build.rs`, because it is a second
  cdylib the harness loads and not something the Rust build links. It publishes
  four capabilities (`c.ready`, `c.timer`, `c.cancel`, `c.drop_in_flight`),
  keeps its own allocation ledger (allocs, frees, live, double frees, bad
  frees), its own waker-clone ledger, and a reply-after-drop count, and reports
  all of them through the host's `log` callback at `shutdown`. Everything it
  needs comes from the committed header, including the presence rule, which is
  what the header's slot constants exist for.
- The harness gains the C section: it runs the build script, loads the gcc
  build, drives the ready path, the timer round trip, cancel, and a drop with
  its service reply in flight, then requires zero live buffers, zero double
  frees, a balanced waker count on both sides, and one recorded reply after a
  drop. That section brings the harness's total to the 62 checks measured at
  landing.
- CI gains two steps: the C build (`scripts/build-c-plugin.sh` under both gcc
  and clang) and `devenv shell -- cargo check -p plugin-contract
  --no-default-features`, the crate alone compiled that way because the fixture
  otherwise turns the `authoring` feature on. `docs/testing.md` carries the run
  commands for the harness, the C build, and the feature-gate check.
- `crates/plugin-fixture/Cargo.toml`: `schemars` for the derived inline schemas.
  `crates/plugin-runtime/Cargo.toml`: `libloading`, `serde_json`, `serde`,
  `jsonschema`, and `plugin-fixture` as a dev-dependency for the integration
  tests that drive the real cdylib.

### Existing abstractions to use

- `crate::payload` for the envelope in both directions, and `metis_read_prefix`
  as the first half of the version gate, which `metis_read_table` completes.
- `authoring/alloc.rs` (`Accounting { allocs, frees, live, violations }`,
  `accounting`) for the ledger, and `crates/plugin-fixture/src/payload.rs` for
  the harness's JSON.
- The harness's own conventions: rebuild the cdylib first, load the real
  library, one `[ok]`/`[fail]` line per check, a `[result] PASS`/`FAIL` summary
  line carrying the count, a non-zero exit on failure, and `env!("CARGO")` for
  the rebuild.
- `libloading` for loading, `scripts/regenerate-abi-header.sh` for the header
  drift check, and the devenv shell for cargo, cbindgen, gcc and clang.

### Do NOT reimplement

- The payload envelope, the allocation accounting, the harness's check
  printing, or the contract pipeline.
- No `async-ffi`, no plugin-side executor, and no async runtime anywhere in
  `plugin-contract`. `plugin-runtime` starts no runtime either: it hands back a
  `Future` and the daemon polls it.

### Seams

1. **ABI through the real cdylib.** `Loaded::load`, the version gate, instance
   creation, the declaration, `invoke_future`, `poll`, `release_buffer`, `drop`,
   the timer round trip through `service_request` and `service_complete`, and the
   unload ordering. Driven by `crates/plugin-runtime/tests/registry.rs` and
   `tests/concurrency.rs` against the fixture, and by the harness.
2. **Layout and presence.** The Rust const assertions, the C translation unit,
   `c_layout.rs`, and `bash scripts/regenerate-abi-header.sh --check`.
3. **The routed trait.** A fixture with seven capabilities dispatches every one
   by name, asserted in `crates/plugin-contract/tests/authoring.rs`.
4. **The registry.** Routing, version gating, duplicate refusal, enable and
   disable, unregister, and both-direction schema validation, asserted against
   the fixture's own declaration.
5. **Quarantine, reclaim and unload.** The ordering ledger, the reclaim the
   caller performs on the poll return, and the two refusals, asserted in
   `tests/registry.rs` and printed by the harness.
6. **The hazard set.** A panic in a poll, a panic in a destructor, a panic in a
   host callback, a blocking poll under the watchdog, a cancel that resolves as
   `Cancelled`, a missed cancel deadline, and a double drop.
7. **The C implementer.** Compiled by both compilers, loaded, driven through the
   ready path, a timer, cancel and a drop with its reply in flight, with its own
   ledger at zero and both sides' waker counts balanced.
8. **No plugin thread.** A capability driven to completion starts no thread,
   asserted by a thread count in `tests/no_plugin_thread.rs` (its own binary, so
   the counts are not the harness's own threads), and by counting eight
   concurrent plugin futures on one host loop in `tests/concurrency.rs`.
9. **The authoring feature gate.** `plugin-contract` still builds with
   `--no-default-features` and no ordinary dependencies.

Test types: the integration tests and the harness binary for seams 1, 4, 5, 6
and 7; unit tests for seam 2 and for the routed trait in seam 3; the
thread-count tests for seam 8; the feature-gate test for seam 9.

### Residual, not landed by this ticket

- A wake after a future is dropped is recorded by the runtime's waker and by the
  harness's, and the counter exists, but no fixture capability produces one on
  purpose, so no check drives it. A drop after unload is likewise structural
  rather than driven: the ordering ledger records that nothing is dropped after
  the unload, and no test deliberately drops a future into an unloaded library,
  because that is the use-after-unload bug the ordering exists to prevent.
- The harness did not refuse a capability by descriptor version: the fixture
  advertised version 1 only, so that check could not fail. The gate itself is
  implemented and unit-tested in `plugin-runtime`, and no check drove the
  version gate across the ABI. `plugin-compatibility` closed that gap: the
  fixture's `tool.newer` is descriptor version 2, the harness applies its own
  gate policy, and four harness checks drive the refusal.
- The missed-cancel check covers a capability that ignores its cancellation
  token, not one that never answers at all.
- `plugin-runtime`'s integration tests and the harness both rebuild the fixture
  cdylib, so those targets serialise on the cargo locks.
- The runtime reads and releases the event and progress payloads but only logs
  them: streaming them to a consumer is the daemon effort's work.
- The C plugin keeps its ledgers in file-scope globals and assumes one instance
  per loaded library.

### Interface contract

- `plugin-compatibility` consumes the `abi_major` and `abi_minor` fields, the
  exact-minor refusal in `metis_read_table`, which it relaxes to additive
  tolerance (`found >= known`), the typed `MetisAbiError`, the presence
  predicates and the slot constants, and the per-capability descriptor gate. It
  landed needing no change to the table shape, which is the surface it inherited
  intact.
- `metis-plugins` consumes `plugin-runtime`'s loader, registry and quarantine.
  `metis-plugin-sdk` re-exports the routed trait and adds typed facades.
  `metis-daemon` owns the real poll loop and the reaper thread, and it is the
  runtime that polls a `PluginCall`, because this crate starts none.

### Provisional decisions

- **Inline capability schemas as two explicit descriptor fields**,
  `request_schema` and `response_schema`, over a by-id fetch. Chosen because the
  prototype measured inline smaller in total bytes (6,014 against 7,915 for a
  listing plus a fetch) and simpler (no second entry point, no cache, no
  fetch-failure path). The operator's position on 2026-10-13 was that this cannot
  be judged before it is used: go with it and revisit if it causes problems. Only
  the descriptor payload shape and the `schemars` dependency depend on the choice,
  so changing it later is cheap.
- **Second shape used deliberately:** the descriptor is a JSON payload returned
  in a `MetisBuffer`, not a C struct, so none of this moves the C ABI, the layout
  asserts, or the generated header.
