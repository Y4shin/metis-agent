# Testing

> Status: the tooling is in place, but no product behaviour exists yet, so
> there are no meaningful tests. The conventions below are what the bootstrap
> established; revisit them when the first real behaviour lands.

## Framework

- Rust's built-in test harness (`#[test]`), run by `cargo test`.
- The SvelteKit frontend uses `svelte-check` for type and template checking.
  No test runner is configured for it yet.

## Run commands

Run these inside the devenv shell (`devenv shell`):

- `cargo fmt --check` -- formatting.
- `cargo clippy --all-targets -- -D warnings` -- linting; warnings are
  errors.
- `cargo test` -- the full suite across the workspace.

Frontend, run from `apps/ui`:

- `npm run check` -- `svelte-kit sync` plus `svelte-check`.
- `npm run build` -- the static build through `@sveltejs/adapter-static`.

Contract, documented in `contracts/README.md`, with both buf commands run from
the repository root, where the one `buf.yaml` lives:

- `bash scripts/regenerate-contracts.sh` -- regenerate the committed Rust for
  the wire contract; `--check` regenerates into a scratch directory and fails on
  any difference.
- `buf lint` -- contract lint. The root `buf.yaml` declares the module at
  `contracts/proto`, so this lints exactly the contract.
- `buf breaking --against '.git#branch=main' --against-config buf.yaml` --
  contract compatibility against the landing branch, scoped to the same module
  on both sides.
- `bash scripts/probe-contract-gates.sh` -- the on-demand acceptance probe for
  the gate configuration: root configuration, deliberate-edit verdict flips for
  lint and breaking, the `BUF_TOKEN` forms, and the drift check. It restores
  every file it touches, and it skips the network-only halves when the anonymous
  BSR rate limit is exhausted. It is not part of CI.

Local transport skeleton, documented in `crates/client`:

- `cargo test -p client` -- the socket test starts the skeleton on a temporary
  socket directory, calls echo through the wrapper, and removes the directory;
  it joins the full `cargo test` run rather than a second job.
- `cargo run -p client --example echo` -- serves the skeleton on the resolved
  endpoint (`METIS_ENDPOINT`, else the platform default), calls echo through
  the wrapper, prints `[echo]` and `[result] PASS`, and exits non-zero on
  failure.

Plugin ABI, documented in `crates/plugin-contract/src/abi` and
`crates/plugin-fixture`:

- `cargo run -p plugin-fixture --bin abi-harness` -- loads the fixture `cdylib`
  through `plugin-contract` and drives every table entry, printing one `[ok]`
  or `[fail]` line per check and a `[result] PASS`/`FAIL` line; it exits
  non-zero on any failure. The harness rebuilds the fixture library first, so
  a stale `.so` is never loaded.
- `bash scripts/regenerate-abi-header.sh` -- regenerate the committed C header
  from the `repr(C)` Rust; `--check` regenerates into a scratch file and fails
  on any difference.
- The C translation unit that static-asserts the ABI layout
  (`crates/plugin-contract/c/layout_static_asserts.c`) is compiled by
  `crates/plugin-contract/build.rs` on every build, so `cargo build`,
  `cargo clippy` and `cargo test` all carry that check; the numbers it
  measured are compared with Rust's by
  `crates/plugin-contract/tests/c_layout.rs`.
- The authoring surface (the `Plugin` trait, the export and host adapters, and
  the crate's minimal executor) is behind `plugin-contract`'s `authoring`
  feature, which is off by default so the runtime sees the primitives and the
  payload envelope alone. Its tests live in
  `crates/plugin-contract/tests/authoring.rs` under that feature, and the
  fixture turns the feature on, so the workspace `cargo test` build unifies it
  in and runs them. `cargo test -p plugin-contract --features authoring` runs
  them on their own, and `cargo test -p plugin-contract` runs the feature-off
  path (the gate checks in `tests/feature_gate.rs`).

## CI

`.github/workflows/ci.yml` runs the Rust commands above through the devenv
shell on push to `main` and on pull requests, followed by the three contract
commands and the two plugin ABI commands (the harness and the header drift
check). The frontend commands are not in CI yet; add them when the frontend
has behaviour worth gating.

## Mock conventions

- To be decided. Nothing is mocked yet.
