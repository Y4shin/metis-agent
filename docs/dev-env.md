# Development environment

This repository uses [devenv](https://devenv.sh/) for a reproducible Rust
shell. Enter it with:

```bash
devenv shell
```

The shell provides the current Rust nightly toolchain (rustc, cargo, clippy,
rustfmt, rust-analyzer) plus Git, `buf`, `cbindgen`, Node.js, and npm. The
nightly version is pinned by `devenv.lock`, and `channel = "nightly"` is set in
`devenv.nix`. `buf` generates the wire contract; both codegen plugins are BSR
remote plugins, so no `protoc`, local plugin binary, or JVM is installed.
`cbindgen` (nixpkgs attribute `rust-cbindgen`, 0.29.4) generates the plugin ABI
header, and the C compiler on the shell path compiles the translation unit that
static-asserts the ABI layout.

Run a one-off command inside the shell with:

```bash
devenv shell -- cargo --version
```

The Rust code is a Cargo workspace rooted at the repo root (`Cargo.toml`),
with crates under `crates/` listed explicitly in `members`.

The SvelteKit frontend lives in `apps/ui` and uses npm. Run its checks from
that directory with `npm run check` and `npm run build`.

## Reproduction

The first runnable path is the local transport skeleton:
`cargo run -p client --example echo` serves the skeleton on the resolved local
endpoint, calls the echo RPC through the client wrapper, and prints
`[result] PASS`. The default endpoint is `$XDG_RUNTIME_DIR/metis/daemon.sock`
when that variable is set, otherwise `$TMPDIR/metis/daemon.sock`, and there is
no fallback to a shared `/tmp` path: a host with neither variable needs
`METIS_ENDPOINT` to name one. The daemon creates the socket's directory and
restricts it to its own user, so the example needs no credential. There is no
product daemon yet.
