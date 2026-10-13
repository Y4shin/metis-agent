# Contracts

`contracts/` is the single source of truth for the daemon's wire contract. The
Protobuf documents here are hand-written; the Rust they produce lives in
`crates/api-contracts/generated/` and is committed.

## Layout

- `proto/metis/v1/daemon.proto` -- package `metis.v1`, service
  `DaemonService`. `Echo`, `Pair`, and the server-streaming `StreamEvents` with
  its `Event` message and `EventKind` enum. Later tickets append cancellation
  calls to the same service and package.
- `../buf.yaml` -- the repository's one buf configuration. It declares the
  module at `contracts/proto`, `buf lint` in STANDARD mode, and
  `buf breaking` in FILE mode, and every buf command reads it.
- `../buf.gen.yaml` -- the two pinned BSR remote plugins that generate the Rust.
- `../scripts/regenerate-contracts.sh` -- the regeneration command.
- `../crates/api-contracts/generated/` -- the plugin output, after the
  formatting step below. Never edit these files by hand; change the Protobuf
  and regenerate.

## Regeneration

One command, from the repository root:

```sh
bash scripts/regenerate-contracts.sh
```

It runs `buf generate` from the repository root, where the module and the
plugin template both live, then formats both generated trees.
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

The CI job runs, in the existing `devenv shell --` style, all from the
repository root:

- `buf lint`: the contract is lint-clean in STANDARD mode. The root `buf.yaml`
  declares the module, so lint covers exactly `contracts/proto`. One rule is
  excepted, and only for the file that needs it: `RPC_RESPONSE_STANDARD_NAME`
  for `contracts/proto/metis/v1/daemon.proto`, because `StreamEvents` answers
  with the `Event` the stream carries rather than with a named response
  wrapper. The exception is the `ignore_only` entry of the root `buf.yaml`.
- `buf breaking --against '.git#branch=main' --against-config buf.yaml`: the
  contract compared with the branch is not broken. Adding RPCs and messages is
  allowed; changing or removing what is already declared is not.
  `--against-config` scopes the branch side of the comparison with the same
  root configuration as the working tree, so both sides resolve the module at
  `contracts/proto` rather than comparing a module with a whole tree. The mode
  that governs the gate is the `breaking` block of the root `buf.yaml`, and
  editing that block changes the gate's verdict.
- `bash scripts/regenerate-contracts.sh --check`: the committed generated trees
  match the contract, byte for byte.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  cover the generated Rust as well, because it is compiled as part of
  `api-contracts`. It carries two module-level allows rather than being excluded
  from the gates, and both are load-bearing: `clippy::match_single_binding` from
  the first service, and `clippy::derivable_impls` from the contract's first
  enum, `EventKind`, whose generated `Default` impl is a write-out of the derive.

## Caveats

- Both gates that generate or compare resolve the two plugins from the BSR over
  the network on every run. CI supplies `BUF_TOKEN` from `secrets.BUF_TOKEN` on
  the step that resolves them (`Contract regeneration check`), so anonymous
  rate limiting cannot fail a run for a reason that has nothing to do with the
  change under review. An absent secret arrives in CI as the empty string, and
  measured on buf 1.73.0 an empty `BUF_TOKEN` reads as anonymous, whereas a
  non-empty token the BSR cannot verify is refused before any plugin resolves.
  The step does not rely on that distinction: it unsets the variable when the
  secret carries nothing, so buf is either given a real token or no variable at
  all, and the gate keeps working without authentication. `buf lint` and
  `buf breaking` resolve no plugins, so neither takes a token.

## Toolchain

Generation needs `buf` and nothing else, `rustfmt` for the formatting step, and
nothing for the everyday build or test: the generated Rust is committed, so
building and testing the workspace uses no generator, no `protoc`, no local
plugin binary, and no JVM. Both plugins are addressed as BSR remote plugins and
pinned by tag in the root `buf.gen.yaml`.
