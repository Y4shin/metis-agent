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

Pairing is optional. An unset or empty `METIS_PRESHARED_KEY` closes pairing: the
daemon starts, every surface that already holds a token keeps being served, and
the pairing call itself is refused as disabled. A reference that is set but
cannot be resolved (a file that is not there, an environment variable that is
unset, a keychain that refuses) is a typed error instead, so a typo is loud
rather than a daemon that quietly stops pairing. Closing pairing revokes
nothing: a token minted earlier keeps serving until its row is deleted.

Pairing is reachable on both transports. The local socket and the remote
listener mount the same service and the same key, and the key travels in the
pairing request body, so it never appears in a URL or a log line. A pairing
refusal travels as the `metis-pair-error` response header, with the value
`disabled`, `wrong-key`, or `already-paired`: a client reports a typed error
(`ClientError::Pairing`) instead of parsing message text. To pair a device
that cannot reach the local socket, the remote listener mounts the same service
with `AuthPolicy::TokenRequired`, and a client reaches it through
`Client::with_remote_addr(addr)` with the surface named: the token the daemon
answers with is kept for that surface and attached to the calls that follow.

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

## Remote exposure and reverse proxies

The remote listener is off unless `METIS_TCP_ADDR` names an `IP:port` literal,
and it carries no TLS: a reverse proxy terminates TLS in front of it, which is
the deployment shape ADR 0004 chose. The daemon reads forwarded information only
from the proxies an operator names in `METIS_TRUSTED_PROXIES`, a comma-separated
list of exact IP addresses with whitespace tolerated. An absent or empty value
names no proxy, and loopback is not trusted implicitly, so the trust rule stays
visible in configuration where an operator can read it. A malformed entry is a
typed `ConfigError`, never a silent empty set.

Forwarded headers from a peer that is not one of those addresses are ignored,
and only the left-most `x-forwarded-for` entry of a single trusted hop is read:
the daemon does not walk a multi-hop chain. The daemon terminates no TLS,
presents no certificate or fingerprint, and grows no trusted-CA setting, so a
client's trust in the endpoint beyond the proxy's own certificate validation is
out of scope (ADR 0004).

Plaintext is allowed with a warning. A listener bound to an address that is not
loopback with no proxy named prints one warning before it accepts, and naming a
trusted proxy suppresses it. The pairing call carries the preshared key in its
body, so a plaintext client leg on that call prints one warning per service,
which is once rather than once per call.

## Windows local transport

On Windows the local endpoint is a named pipe rather than a socket, and
`METIS_ENDPOINT` names it the way it names a socket path on Unix. An absent or
empty value is the platform default, `\\.\pipe\metis-daemon`. A value must
start with the local pipe prefix `\\.\pipe\` (compared case-insensitively,
because Windows pipe names are not case sensitive), must fit the 256 characters
`CreateNamedPipeW` documents, and must carry no interior NUL. Anything else, a
`\\server\pipe\name` path included, is a typed `ConfigError::InvalidPipeName`
naming the variable and the value rather than a silent fall back to the default,
so a typo cannot quietly point the daemon at a remote pipe.

Every pipe instance is created with a security descriptor that grants full
access to the daemon's own account, read from the daemon process's token, and to
Local System, denies the network logon group, and carries no inherited access,
and the pipe rejects remote clients. The ACL alone would not make the pipe
local-only: a remote caller using the daemon account's credentials carries the
same user SID, so the reject flag is what closes the network path. The first
instance carries `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a squatter that already
holds the name makes the daemon fail loudly instead of sharing the pipe with it.

The pipe is the Windows counterpart of the private socket directory: it is
private to the account that owns the daemon, so any process running as that
account on the same machine can connect, exactly as mode 0700 behaves on Unix,
and a Windows client has no credential path yet.

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
