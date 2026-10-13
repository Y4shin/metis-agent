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
    the `/tmp` conflict below).
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
  `tls-and-client-trust`, and `reverse-proxy-mode` extend it with new bind and
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
  rule).
- Open conflict for `local-identity-and-acl`, recorded rather than resolved
  here: `/tmp` is world-writable and sticky, so the socket path cannot satisfy
  that ticket's requirement that the daemon own the socket's directory with a
  configured `uid:gid` and mode 700 or 770, and `serve_uds` unlinks whatever
  sits at the path before binding, so another local user can pre-create the path
  between daemon starts. That ticket must revisit the default (for example a
  per-user runtime directory such as `$XDG_RUNTIME_DIR/metis/daemon.sock` with a
  documented fallback) and decide whether its ACL tests set the
  `METIS_ENDPOINT` override. The map carries the same item as fog.
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
- Do not wire TLS, a proxy header, or a trust rule. `tls-and-client-trust` and
  `reverse-proxy-mode` own those; this listener is plaintext.
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

- `tls-and-client-trust` and `reverse-proxy-mode` add their own bind and
  connector front-ends against `serve_stream`, `serve_tcp_listener`'s shape (the
  one accept loop; `serve_tcp` is a bind plus a delegate), and
  `resolve_remote_addr()`'s rule. They name their authority through a helper
  beside `local_authority` and `remote_authority` and add their own error
  variants to the non-exhaustive `ConfigError`, and they still do not add an
  injection point to `Client`.
- `local-identity-and-acl` and `pairing-and-tokens` add the credential path.
  Until they land, an enabled listener serves the same unauthenticated surface
  the local socket serves, which is why the bind address is required and has no
  default. The ordering risk is recorded, not hidden.
- The plaintext warning the map's constraints require belongs to the daemon's
  startup policy and to `reverse-proxy-mode`; this ticket records the residual
  instead of inventing a policy surface for it.
- The realtime stream over TCP is not asserted here: no streaming RPC exists
  until `event-stream` lands, and `serve_stream` serves any Connect service over
  the stream it is handed. This is the acceptance-criterion narrowing the
  operator approved before the chain: criterion 4 asserts the echo RPC only, and
  the stream-over-TCP assertion is carried by `event-stream`, the ticket that
  owns the first stream.

### Residual, recorded rather than resolved

- Plaintext and unauthenticated until `local-identity-and-acl` and
  `pairing-and-tokens` land: an enabled listener serves the same surface the
  local socket serves, and the required, defaultless bind address is the only
  mitigation today.
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

## Downstream interface contracts (outline)

These tickets are not pending yet: no ticket has a pending section today, and
the three still pending in this effort are `event-stream`,
`local-identity-and-acl`, and `windows-named-pipe`. Their sections are added
when they reach the frontier, and the commitments below are what the landed
tickets must already support.

- `event-stream`, `stream-resume-and-expiry`, `cancellation-and-slow-consumers`:
  append `StreamEvents` and `Cancel` to `metis.v1`, with `last_event_id` in the
  request, the snapshot as the stream's first message, heartbeats as ordinary
  messages, and correlation ids in request metadata. The
  `RPC_RESPONSE_STANDARD_NAME` lint exception is added here, and the contract
  regenerates through `scripts/regenerate-contracts.sh`. `EventKind` is the
  contract's first enum, which is what makes the buffa mount's dormant
  `clippy::derivable_impls` allow load-bearing. It also carries the
  stream-over-TCP assertion `tcp-listener`'s acceptance criterion 4 deferred:
  `serve_stream` serves any Connect service over the stream it is handed, and
  the landed TCP front-ends are the first transport that can carry a real
  stream end to end.
- `windows-named-pipe`, `tls-and-client-trust`, `reverse-proxy-mode`: extend
  the same `transport` module with their own bind and connector front-ends; no
  second service implementation. `serve_stream` stays the only connection path
  and the landed `serve_tcp_listener` is the shape a new accept loop copies:
  build the router once, wrap it in an `Arc`, and clone the `Arc` per accepted
  stream, because `serve_stream` takes `router: impl Into<Arc<Router>>`
  (`connectrpc::Router` is not `Clone` in 0.9.1). A front-end that must report a
  kernel-assigned port takes an already bound listener, as `serve_tcp_listener`
  does, and a front-end that resolves a non-default authority names it through
  a helper beside `local_authority` and `remote_authority`; new settings and
  bind failures join the non-exhaustive `ConfigError`. Because `Client` builds
  its own transport and exposes no injection point, each of these tickets
  extends `client` with a constructor for its own transport instead of injecting
  one from outside; on non-Unix there is no local constructor until
  `windows-named-pipe` lands.
- `local-identity-and-acl`, `pairing-and-tokens`, `remote-pairing`: add request
  metadata and RPCs under `metis.v1`; credentials travel as
  `Authorization` metadata on every call, including the stream.
  `local-identity-and-acl` first settles the endpoint default against its
  directory-ownership requirement, the open conflict recorded in the landed
  `local-transport-rpc-skeleton` section, and decides whether its tests set
  `METIS_ENDPOINT`; the `ClientError` credential variants land there.

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
