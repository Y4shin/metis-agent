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

## Daemon state

The daemon keeps its state in one SQLite database. `METIS_STATE_DB` names the
file when it is set and not empty, otherwise the file is
`$XDG_DATA_HOME/metis/metis.db`, otherwise `$HOME/.local/share/metis/metis.db`;
with none of the three set, the daemon has nowhere to keep its state and
there is no guess to fall back on. Opening the database creates its directory
at mode 700 and the file at mode 600, and applies the schema embedded in the
binary, so a fresh install needs no preparation step and no `sqlx-cli` call.

The database carries the table the per-surface token hashes live in. A token
itself is never written to it: only the SHA-256 of the token's text.

## Pairing and credentials

A surface pairs once and then holds its own credential. The daemon keeps only a
token's hash, so the token text lives on the surface that paired it.

`METIS_PRESHARED_KEY` is the daemon's pairing secret, written as a secret
reference:

- `literal:<value>` is the secret itself, and a value with no prefix is the same
  thing;
- `env:<VARIABLE>` reads an environment variable;
- `keystore:<account>` reads the platform keychain through its command line tool
  (`secret-tool` on Linux, `security` on macOS), under the service name
  `metis-daemon`;
- `file:<path>` reads a file. The file must be readable by its owner alone: a
  mode that grants anything to group or others is refused rather than read.

An unset or empty `METIS_PRESHARED_KEY` closes pairing: there is no key to pair
with, and the refusal names the variable. Pairing is reachable on both
transports, and the key travels in the pairing request body, so it never appears
in a URL or a log line.

The local socket needs no credential, because the operating system's access
control on the socket's directory and file is what authenticates a local caller.
The remote listener requires one: every call carries its token as an
`Authorization: Bearer` header, and a call without one is refused with a typed
error that says whether pairing is needed or the access is gone. Each listener
mounts its own policy (`AuthPolicy::LocalAcl` or `AuthPolicy::TokenRequired`) in
`daemon::service::service`; nothing inspects the peer address to decide.

The client keeps the token in the platform keychain when that command line tool
is on `PATH`, and otherwise in `$XDG_DATA_HOME/metis/client/<surface>.token`
at mode 600 inside a directory at mode 700. Pairing reports which backend took
it. A keychain that is present but refuses is an error rather than a silent
fallback to the file: a credential that is stored and cannot be read must not
look like one that was never stored.

## Reproduction

The first runnable path is the local transport skeleton:
`cargo run -p client --example echo` serves the skeleton on the resolved local
endpoint, calls the echo RPC through the client wrapper, and prints
`[result] PASS`. The default endpoint is `$XDG_RUNTIME_DIR/metis/daemon.sock`
when that variable is set, otherwise `$TMPDIR/metis/daemon.sock`, and there is
no fallback to a shared `/tmp` path: a host with neither variable needs
`METIS_ENDPOINT` to name one. The daemon creates the socket's directory and
restricts it to its own user, so the example needs no credential. The daemon's
binary is still a thin entry point: its own service (`daemon::service::service`)
is what a listener serves, and the daemon effort wires the listeners into
`main`.
