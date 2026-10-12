# Development environment

This repository uses [devenv](https://devenv.sh/) for a reproducible Rust
shell. Enter it with:

```bash
devenv shell
```

The shell provides the current Rust nightly toolchain (rustc, cargo, clippy,
rustfmt, rust-analyzer) plus Git. The nightly version is pinned by
`devenv.lock`, and `channel = "nightly"` is set in `devenv.nix`.

Run a one-off command inside the shell with:

```bash
devenv shell -- cargo --version
```

The Rust code is a Cargo workspace rooted at the repo root (`Cargo.toml`).
Crates are added to the workspace `members` as they appear.

## Reproduction

There is no runnable product code yet, so there is nothing to reproduce.
Once crates exist, record the reproduction command here.
