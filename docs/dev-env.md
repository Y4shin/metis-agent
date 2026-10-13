# Development environment

This repository uses [devenv](https://devenv.sh/) for a reproducible Rust
shell. Enter it with:

```bash
devenv shell
```

The shell provides the current Rust nightly toolchain (rustc, cargo, clippy,
rustfmt, rust-analyzer) plus Git, `buf`, Node.js, and npm. The nightly version is
pinned by `devenv.lock`, and `channel = "nightly"` is set in `devenv.nix`.
`buf` generates the wire contract; both codegen plugins are BSR remote plugins,
so no `protoc`, local plugin binary, or JVM is installed.

Run a one-off command inside the shell with:

```bash
devenv shell -- cargo --version
```

The Rust code is a Cargo workspace rooted at the repo root (`Cargo.toml`),
with crates under `crates/` listed explicitly in `members`.

The SvelteKit frontend lives in `apps/ui` and uses npm. Run its checks from
that directory with `npm run check` and `npm run build`.

## Reproduction

There is no runnable product code yet, so there is nothing to reproduce.
Once crates exist, record the reproduction command here.
