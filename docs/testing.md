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

Remote transport, off unless `METIS_TCP_ADDR` names an `IP:port` literal. There
is no default bind address, so remote access cannot be enabled by accident:

- `cargo test -p client --test remote` -- the remote test starts the TCP
  listener on the port the kernel assigns (`METIS_TCP_ADDR=127.0.0.1:0`), takes
  the bound address from `skeleton::serve_remote`, and calls echo through
  `Client::with_remote_addr`; it also asserts that an unset variable binds no
  socket, and that the listener task releases its port when it is dropped. TCP
  is the first front-end every target has, so the test is not Unix-gated.
- The configuration rule is unit-tested in
  `crates/api-contracts/src/transport.rs` (`cargo test -p api-contracts`): unset
  and empty resolve to `None`, an `IP:port` literal resolves to `Some`, and
  anything else is a typed `ConfigError` naming the variable and the value
  rather than a silent disable.

Local access control and the identity declaration, documented in
`crates/api-contracts` and `crates/client`. The access control is the operating
system's, so a local client carries no credential; the declaration says which
surface did what and is not authentication:

- The socket ACL is unit-tested in `crates/api-contracts/src/transport.rs`
  (`cargo test -p api-contracts`): `ensure_socket_dir` creates a fresh socket
  directory with mode 700 owned by the daemon's user, applies the group and mode
  770 when `METIS_SOCKET_GROUP` names a group that user belongs to, and refuses
  a pre-existing directory that is a symlink, is not a directory, is not owned
  by that user, or is wider than the rule allows, without chmodding it.
  `serve_uds` leaves the same mode on the socket file it binds. An unknown group
  name is a typed `ConfigError` rather than a silent fallback to the primary
  group.
- The endpoint rule is unit-tested in `crates/client/src/lib.rs`
  (`cargo test -p client --lib`): `$XDG_RUNTIME_DIR/metis/daemon.sock` when that
  variable is set, else `$TMPDIR/metis/daemon.sock`, else the typed
  `EndpointError::NoPrivateBase`. There is no `/tmp` fallback, because a shared
  directory cannot carry the ACL, and `METIS_ENDPOINT` still wins over both.
- `cargo test -p client --test acl` points `METIS_ENDPOINT` at a directory the
  daemon must create, asserts the directory and the socket are private, and
  asserts a shared directory is refused and left as it was.
- `cargo test -p client --test identity` asserts the daemon records what a
  wrapper call declares (`metis-protocol-version`, `metis-surface`,
  `metis-client-version`, carried as request metadata on every call), and that a
  client declaring another, or no, protocol version is refused with an
  `InvalidArgument` error whose `metis-supported-versions` response header names
  what the daemon speaks.

Plugin ABI, documented in `crates/plugin-contract/src/abi` and
`crates/plugin-fixture`:

- `cargo run -p plugin-fixture --bin abi-harness` -- loads the fixture `cdylib`
  through `plugin-contract` and drives the revised table: the version gate, the
  capability declaration, the futures path, the service round trip, cancel, and
  the hazard set, printing one `[ok]` or `[fail]` line per check, the teardown
  ordering as it happens, and a `[result] PASS`/`FAIL` line; it exits non-zero on
  any failure. The harness rebuilds the fixture library first, so a stale `.so`
  is never loaded. It also carries the compatibility checks: the entry symbol
  resolved for this major against a library built for another one, the
  per-capability version gate, a request body carrying a field this build does
  not know, and, on Linux, tables placed against a page with no access, so a
  reader that touched an entry past the reported size would fault rather than
  quietly read it.
- `bash scripts/build-c-plugin.sh` -- compile the C implementer of the table
  (`crates/plugin-fixture/c/c-plugin.c`) with `-std=c11 -Wall -Wextra -Werror`
  under both gcc and clang, and leave the gcc build at
  `target/c-plugin/libcplugin.so`. The ABI harness runs this script itself and
  then drives that library end to end, so the header is proven usable by a
  compiler other than rustc and by a plugin that is not the fixture. The same
  script compiles `crates/plugin-fixture/c/other-major-plugin.c` into
  `target/c-plugin/libother-major.so`: a library whose only `metis_*` symbol is
  `metis_plugin_entry_v2`, which the harness resolves the v1 entry symbol
  against to require a typed major refusal.
- `bash scripts/regenerate-abi-header.sh` -- regenerate the committed C header
  from the `repr(C)` Rust; `--check` regenerates into a scratch file and fails
  on any difference.
- The C translation unit that static-asserts the ABI layout
  (`crates/plugin-contract/c/layout_static_asserts.c`) is compiled by
  `crates/plugin-contract/build.rs` on every build, so `cargo build`,
  `cargo clippy` and `cargo test` all carry that check; the numbers it
  measured are compared with Rust's by
  `crates/plugin-contract/tests/c_layout.rs`.
- The authoring surface (the capability-name-routed `Plugin` trait and the
  boxed-future helper that replaces `async-ffi`) is behind `plugin-contract`'s
  `authoring` feature, which is off by default so the host sees the primitives
  and the payload envelope alone. Its tests live in
  `crates/plugin-contract/tests/authoring.rs` under that feature, and the
  fixture turns the feature on, so the workspace `cargo test` build unifies it
  in and runs them. `cargo test -p plugin-contract --features authoring` runs
  them on their own, and `cargo test -p plugin-contract` runs the feature-off
  path (the gate checks in `tests/feature_gate.rs`). The revision added the
  version gate and the presence rule in `crates/plugin-contract/tests/abi.rs`,
  and the no-executor check in `crates/plugin-contract/tests/no_plugin_thread.rs`,
  which has its own binary so the thread count it takes is not the harness's own
  threads.

Plugin host, documented in `crates/plugin-runtime`:

- `cargo test -p plugin-runtime` -- the loader, the registry and the driver,
  driven against the real fixture cdylib. The tests build the fixture first, then
  load it through the version gate. `tests/registry.rs` covers routing, version
  gating, duplicate refusal, enable and disable, unregister, both-direction
  schema validation, quarantine and the unload ordering; `tests/concurrency.rs`
  runs eight plugin futures on one host loop and asserts the plugin started no
  thread. The runtime starts no runtime of its own: it hands back a `Future` and
  the test's small loop polls it.

Daemon state, documented in `crates/daemon`:

- `cargo test -p daemon` -- the daemon's store and the token surface.
  `crates/daemon/tests/path.rs` covers the path rule (`METIS_STATE_DB`, then
  `$XDG_DATA_HOME/metis/metis.db`, then `$HOME/.local/share/metis/metis.db`,
  then the typed refusal when none of them is set);
  `crates/daemon/tests/state.rs` opens a store on a temporary file and asserts
  the directory at 700 and the file at 600, the committed migration applied on
  open, the token round trip, per-surface revocation, and the typed refusal of
  pairing a surface twice; `crates/daemon/tests/token.rs` covers minting, the
  SHA-256 known answer and parsing. The store operations run on
  `sqlite::memory:`; the creation and migration seams run on a temporary file.
  There is no database server, no `DATABASE_URL`, no committed `.sqlx` offline
  data and no `sqlx-cli` step: the store uses `sqlx`'s runtime queries and the
  migrator embedded in the binary, so the existing `cargo test` step carries it.
- The private-path rule the state directory and file share with the socket is
  unit-tested in `crates/api-contracts/src/transport.rs`
  (`cargo test -p api-contracts`): `ensure_private_dir` takes its mode, refuses a
  directory wider than that mode without chmodding it, and `restrict_file`
  narrows a file to the mode it is given.

## CI

`.github/workflows/ci.yml` runs the Rust commands above through the devenv
shell on push to `main` and on pull requests, followed by the three contract
commands and the four plugin ABI commands (the C plugin build, the
authoring-feature-off check, the harness, and the header drift check). The
frontend commands are not in CI yet; add them when the frontend
has behaviour worth gating.

## Mock conventions

- To be decided. Nothing is mocked yet.
