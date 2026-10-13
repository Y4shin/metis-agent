# Contracts

`contracts/` is the single source of truth for the daemon's wire contract. The
Protobuf documents here are hand-written; the Rust they produce lives in
`crates/api-contracts/generated/` and is committed.

## Layout

- `proto/metis/v1/daemon.proto` -- package `metis.v1`, service
  `DaemonService`. One ordinary RPC for now: `Echo`. Later tickets append an
  event stream and cancellation calls to the same service and package.
- `buf.yaml` -- module root `proto`, `buf lint` in STANDARD mode, `buf breaking`
  in FILE mode.
- `buf.gen.yaml` -- the two pinned BSR remote plugins that generate the Rust.
- `../scripts/regenerate-contracts.sh` -- the regeneration command.
- `../crates/api-contracts/generated/` -- the plugin output, after the
  formatting step below. Never edit these files by hand; change the Protobuf
  and regenerate.

## Regeneration

One command, from the repository root:

```sh
bash scripts/regenerate-contracts.sh
```

It runs `buf generate` from this directory, then formats both generated trees.
`bash scripts/regenerate-contracts.sh --check` regenerates into a scratch
directory, formats it the same way, and diffs it against the committed trees,
exiting non-zero on any difference. CI runs the check, so a Protobuf change that
was not regenerated fails the build.

The formatting step is rustfmt with the repository configuration plus
`normalize_comments=true`: buffa emits multi-line `/** */` doc comments, and
rustfmt's re-indenting of their prose turns it into an indented Markdown code
block that rustdoc then collects as a doctest that cannot compile. Normalizing
those comments to `///` lines keeps the generated trees inside `cargo test`.
`cargo fmt` never sees the generated files, because `src/proto.rs` and
`src/connect.rs` `include!` them rather than declaring them as modules.

## Gates

The CI job runs, in the existing `devenv shell --` style:

- `buf lint` from this directory: the contract is lint-clean in STANDARD mode.
- `buf breaking --against '.git#branch=main'` from the repository root: the
  contract compared with the branch is not broken. Adding RPCs and messages is
  allowed; changing or removing what is already declared is not. The comparison
  walks the whole tree on both sides, so the paths line up without a root
  `buf.yaml`.
- `bash scripts/regenerate-contracts.sh --check`: the committed generated trees
  match the contract, byte for byte.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  cover the generated Rust as well, because it is compiled as part of
  `api-contracts`. It carries two module-level allows rather than being excluded
  from the gates.

## Toolchain

Generation needs `buf` and nothing else, `rustfmt` for the formatting step, and
nothing for the everyday build or test: the generated Rust is committed, so
building and testing the workspace uses no generator, no `protoc`, no local
plugin binary, and no JVM. Both plugins are addressed as BSR remote plugins and
pinned by tag in `buf.gen.yaml`.
