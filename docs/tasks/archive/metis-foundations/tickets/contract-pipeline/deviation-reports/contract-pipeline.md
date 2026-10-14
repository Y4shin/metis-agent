---
type: deviation report
title: Deviation report for contract-pipeline
status: stable
---

## Deviation report: contract-pipeline

Reviewed: `main..ticket/contract-pipeline` (17 files, +2458/-3). The landing
branch `task/contract-pipeline` did not exist at report time, so the working
branch was diffed against `main` directly.

### API surface changes

**The exported proto and Rust surface matches the plan.**
- **Planned:** package `metis.v1`, `service DaemonService` with one `rpc Echo`,
  `EchoRequest { string message = 1; }`,
  `EchoResponse { string message = 1; string received_at = 2; }`; committed
  trees at `crates/api-contracts/generated/{buffa,connect}/metis.v1.rs`; mount
  namespaces `api_contracts::proto::metis::v1` and
  `api_contracts::connect::metis::v1`; dependencies `connectrpc = "=0.9.1"`
  (`client`, `server`, `json`), `buffa = "=0.9.2"`, `serde`, `http`,
  `http-body`; `scripts/regenerate-contracts.sh`; the three CI gates;
  `pkgs.buf` in `devenv.nix`.
- **Actual:** exactly that, name for name. The generated connect tree exposes
  `DaemonService`, `DaemonServiceExt` (`register`), `DaemonServiceServer<T>`,
  `DaemonServiceClient<T>`, and `DAEMON_SERVICE_SERVICE_NAME`, which is the
  generator's own surface, not a renamed hand-written alias.
- **Impact:** none on dependents. `local-transport-rpc-skeleton` can import the
  names the interface contract promised, and the `#[allow(refining_impl_trait)]`
  requirement on any `DaemonService` impl is real and demonstrated by the test.

**Gate deviation: the rustfmt post-step carries `normalize_comments=true`.**
- **Planned:** "apply the rustfmt post-step to both generated trees".
- **Actual:** `rustfmt --edition 2024 --config-path rustfmt.toml --config
  normalize_comments=true`, an extra option beyond the repository config.
- **Impact:** low, and justified by measurement. buffa emits multi-line `/** */`
  doc comments; plain rustfmt re-indents their prose to the item's indentation,
  rustdoc then reads that as an indented code block and collects uncompilable
  doctests, so `cargo test` fails. I reproduced the re-indentation mechanism
  independently: plain rustfmt on the spike's raw buffa output moves the
  continuation lines to the item indentation, while `normalize_comments` turns
  them into `///` lines. The committed trees are byte-stable under the script's
  config (`rustfmt --check` exits 0 with it). No dependent is affected; the next
  generated tree inherits the same step.

**Gate deviation: `buf breaking` runs from the repository root, so
`contracts/buf.yaml`'s `breaking: use: [FILE]` does not govern the CI gate.**
- **Planned:** `buf breaking` in FILE mode against `main`, with the mode declared
  in `contracts/buf.yaml`; the CI step was written as
  `buf breaking --against '.git#branch=main'` without a working directory.
- **Actual:** the CI step runs from the repository root against buf's built-in
  default configuration, because the root has no `buf.yaml`. The category
  coincides today (buf's default is FILE, and a field rename is caught), but the
  declared contract config is inert for the gate. The root run is also forced:
  from `contracts/`, `.git#branch=main` resolves to `contracts/.git`, which does
  not exist (verified: exit 1, "does not appear to be a git repository").
- **Impact:** medium for later contract tickets. A future edit to
  `contracts/buf.yaml`'s breaking category, `except`, or `ignore` list will
  silently not apply to CI, so a maintainer can be left believing a weaker gate
  is in force. The `buf lint` gate does run from `contracts/` as planned, so the
  two buf gates read their configuration from different places.

**Dormant allow: `clippy::derivable_impls` on the buffa mount.**
- **Planned:** the mount carries "the measured allow set" (both
  `derivable_impls` and `match_single_binding`).
- **Actual:** both allows are present as planned, but `derivable_impls` cannot
  currently fire: this contract declares no enum, and the generated buffa tree
  has no hand-written `impl Default`. Only `match_single_binding` is load-bearing
  (removing it fails clippy with three errors).
- **Impact:** none. The allow becomes live with the first enum, which
  `event-stream` adds (`EventKind`).

### Abstraction usage

- Used/was specified: **yes.** `contracts/buf.yaml`, `contracts/buf.gen.yaml`,
  the plugin pins and option strings (`views=true`, `json=true`,
  `file_per_package=true`; bare `file_per_package` for connect), and the
  `file_per_package` plus handwritten `include!` module tree all follow
  `spikes/connectrpc/` rather than the local-only packaging plugin. There is no
  `build.rs`, no `protoc`, no JVM, no local plugin binary. The workspace was
  filled, not reshaped.

### Out-of-scope changes

- `docs/dev-env.md` updated (`buf` is now in the shell). Not in the spec's export
  list; harmless and true.
- `docs/testing.md` updated with the contract commands. Arguably in scope: the
  spec names it as the owner of the run commands.
- `Cargo.lock` grows with the `connectrpc`/`buffa` dependency tree. Expected.
- Additions only. No removals, no spike or research file touched, no new workspace
  crate, no hand edit inside `generated/`.

### Ticket doc update needed?

**Yes.** Append to `## Implementation notes` (do not alter the acceptance
criteria):

1. The rustfmt post-step uses `normalize_comments=true` in addition to the
   repository config, because buffa's multi-line `/** */` doc comments otherwise
   become indented Markdown code blocks that fail `cargo test`.
2. `buf breaking` runs from the repository root, so the effective configuration is
   buf's default and not `contracts/buf.yaml`'s `breaking` block; from
   `contracts/` the git target cannot resolve at all. `buf lint` does run from
   `contracts/`.
3. The `clippy::derivable_impls` mount allow is dormant until the contract
   declares its first enum.
4. The regeneration and drift check resolve the two plugins over the network from
   the BSR on every run. Anonymous BSR rate limiting can fail the check with
   `resource_exhausted: too many requests`; three of five `buf generate` runs from
   this host hit it before one succeeded.
5. `docs/dev-env.md` and `docs/testing.md` were updated alongside the ticket.

### Verification evidence

Independently run, not copied from the worker report:

- `cd contracts && buf lint` -> exit 0.
- `buf breaking --against '.git#branch=main'` from the repository root -> exit 0;
  from `contracts/` -> exit 1 (no `contracts/.git`).
- `bash scripts/regenerate-contracts.sh --check` -> exit 0, "generated trees
  match the contract" (so byte-for-byte reproduction holds), and exit 1 when the
  BSR is unreachable.
- `cargo test -p api-contracts` -> 3/3 contract tests pass, six generated
  doctests ignored; `cargo fmt --check` and `cargo clippy --all-targets -- -D
  warnings` -> exit 0.
- `rustfmt --check` with the script's exact config on both committed trees ->
  exit 0.
- Not independently reproduced: the drift check failing on a hand edit, and the
  breaking-change failure demo. Both require mutating tracked files while the
  chain owned the working tree, and my detached worktree attempt was blocked by
  the same BSR rate limiting. The worker measured exit 1 and exit 100
  respectively, and the script's diff logic and buf's exit codes are consistent
  with that.
- Committed generated files carry the `// @generated ... DO NOT EDIT.` marker,
  and no generated file was hand-edited (the committed trees equal the formatted
  plugin output, as the passing drift check shows).

### User attention needed?

**Yes**, three points, none of them blocking landing:

1. Confirm `normalize_comments=true` is acceptable as part of the contract
   pipeline. It is required for `cargo test` to pass and is now the precedent for
   every later generated tree.
2. Decide where the breaking-change policy should live. Today the policy in
   `contracts/buf.yaml` does not govern the CI gate; either move the buf
   configuration to a root `buf.yaml` (so the root invocation reads it) or run
   the breaking gate from `contracts/` against an explicit repository path.
3. Decide how CI reaches the BSR. The drift gate has no offline fallback, so
   anonymous rate limiting can fail a pull request for a non-code reason
   (`resource_exhausted: too many requests` was reproduced here). A BSR token in
   CI, or vendored pinned plugins, would remove that dependency; the effort spec
   chose BSR remote plugins without recording this CI-side cost.
