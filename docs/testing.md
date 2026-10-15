# Testing

> Status: the commands below are the protocol the effort's chains and CI run.
> Every one named there exists and is green on `main` except the Windows
> compile-only job and the gaps `docs/testing.md` records in the Windows local
> transport block.

## Test protocol

Run these always, in this order:

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`

Run if asked (by number):

1. `buf lint`
2. `buf breaking --against '.git#branch=main' --against-config buf.yaml`
3. `bash scripts/regenerate-contracts.sh --check`
4. `bash scripts/build-c-plugin.sh`
5. `cargo check -p plugin-contract --no-default-features`
6. `cargo run -p plugin-fixture --bin abi-harness`
7. `bash scripts/regenerate-abi-header.sh --check`
8. `cargo run -p client --example echo`
9. `cargo run -p client --example events`

Every command runs inside the devenv shell as `devenv shell -- <command>`, which
is where `cargo`, `buf` and the rest exist. The three always-run commands are what
`.github/workflows/ci.yml` gates on first, and numbers 1 to 7 are the contract and
plugin ABI commands it runs after them; numbers 3 and 2 resolve their pinned
plugins from the BSR, so they need the credential described in `docs/dev-env.md`.
Numbers 8 and 9 are the two runnable walkthroughs, which CI does not run. The
`## Run commands` section below documents the same commands per area, with what
each one proves.

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

Event stream, documented in `crates/api-contracts/src/events.rs`,
`crates/daemon` and `crates/client`:

- `cargo test -p api-contracts --lib` -- the ring's unit tests: identifiers are
  daemon-wide and start at one, the window drops its oldest event past
  capacity, `earliest_resumable` follows the oldest event still held, the
  snapshot is the head of each selected channel, `since` returns what follows a
  cursor, a subscription delivers what is published after it, and the serving
  stream opens with the snapshot and then follows the ring. The same file
  asserts the cursor rule: a servable cursor replays only what follows it, typed
  as ordinary events and with no snapshot, the edges of the window are servable
  (a cursor one below the oldest held identifier, and the newest identifier), a
  cursor below the window or beyond the newest identifier is expired carrying
  both bounds, a cursor of zero is never expired, and the refusal crosses the
  wire as the `metis-cursor-error` header, which an unknown value does not parse
  into.
- `cargo test -p daemon --test stream` -- over a temporary socket, the daemon's
  own service is served with its ring: the stream's first message is the
  snapshot, an event published while the stream is open arrives with a greater
  identifier, an unselected channel contributes nothing, an idle stream carries
  heartbeats as ordinary messages, and the identifiers never decrease. The same
  test file asserts resume and expiry: a stream opened with a cursor replays
  only the events that follow it and carries no snapshot, a cursor older than
  the ring is refused at the call that opened the stream as
  `ClientError::CursorExpired` and the cursorless resync then opens with the
  snapshot, a disconnect and a reconnect with the last seen identifier deliver
  exactly the missed events with no gap and no duplicate, and a restarted
  service (a second `Service` over the same store on a second socket) expires
  every cursor while a cursorless stream still opens with its new, empty ring's
  snapshot. It also asserts the stream's authorization: with
  `AuthPolicy::TokenRequired` mounted, a stream without a credential is refused
  and the same call with a paired token is served.
- `cargo test -p daemon --test cancel` -- over a temporary socket, the
  daemon's own service is served and the cancel and slow-consumer properties are
  asserted. A declared request identifier comes back in the response metadata of
  a unary call and of the stream alike, and a call that declares none is served
  exactly as it was before identifiers existed. A `Cancel` naming the identifier
  a stream declared stops it, the client reads the stop as the typed
  `ClientError::StreamCancelled`, and a second call, a call for work that was
  never in flight, and a call by another surface for the same identifier are all
  answered as `AlreadyFinished`; a malformed identifier is refused as an
  `InvalidArgument`. A client that disconnects cancels nothing: the stream that
  is still open keeps carrying, the ring keeps what it holds, the registry's
  count of cancellations is unchanged, and the disconnected stream's identifier
  is answered as already finished. A reader that stops reading is dropped: the
  stream ends with `ResourceExhausted` carrying `metis-stream-error:
  slow-consumer`, the message names how many events were lost, and nothing
  follows it.
- `cargo test -p client --test stream` -- the wrapper's half over the
  skeleton's own socket: the stream is opened and drained through `Client`
  without naming a transport or a generated stub, and an expired cursor is the
  wrapper's typed `ClientError::CursorExpired` whose resync opens with the
  snapshot. It also asserts the two terminal refusals the wrapper answers with
  its own types: a stalled reader earns `ClientError::StreamDropped` and a
  cancelled stream `ClientError::StreamCancelled`, neither of which is a clean
  end or a transport failure, so the gap is never presented as current state and
  the answer is the same cursorless resync. The typed errors are read from the
  stream's trailing metadata, which is where a failure that happens after the
  stream was established carries the daemon's vocabulary.
- `cargo run -p client --example events` -- serves the skeleton over a ring
  holding one event, opens the stream through the wrapper, prints `[snapshot]`
  for the message the stream opens with, publishes a second event into the ring,
  prints `[event]` for the event that arrives, opens a second stream with that
  event's identifier as its cursor, publishes a third event, prints `[resume]`
  for the event the resumed stream carries, cancels that stream by the
  identifier its handle reports and prints `[cancel]` for the outcome, and exits
  non-zero on failure.

Remote transport, off unless `METIS_TCP_ADDR` names an `IP:port` literal. There
is no default bind address, so remote access cannot be enabled by accident:

- `cargo test -p client --test remote` -- the remote test starts the TCP
  listener on the port the kernel assigns (`METIS_TCP_ADDR=127.0.0.1:0`), takes
  the bound address from `skeleton::serve_remote`, and calls echo through
  `Client::with_remote_addr`; it also asserts that an unset variable binds no
  socket, and that the listener task releases its port when it is dropped. TCP
  is the first front-end every target has, so the test is not Unix-gated.
- `cargo test -p daemon --test auth` pairs over that same listener (below), so a
  device with no access to the local socket is set up the way the remote
  transport is reached: the daemon's own service mounted with
  `AuthPolicy::TokenRequired`, a client built with `Client::with_remote_addr` and
  a declared surface, and the preshared key presented on the pairing call.
- The configuration rule is unit-tested in
  `crates/api-contracts/src/transport.rs` (`cargo test -p api-contracts`): unset
  and empty resolve to `None`, an `IP:port` literal resolves to `Some`, and
  anything else is a typed `ConfigError` naming the variable and the value
  rather than a silent disable.

Reverse-proxy mode, documented in `crates/api-contracts/src/proxy.rs`,
`crates/api-contracts/src/transport.rs` and `crates/daemon/tests/proxy.rs`.
`METIS_TRUSTED_PROXIES` names the exact addresses whose forwarded headers are
read, and there is no default: an absent or empty value trusts nobody.

- `cargo test -p api-contracts --lib proxy` -- the rule's unit tests: an absent
  or empty value names no proxy, a comma-separated list parses with whitespace
  tolerated and does not trust loopback implicitly, and a malformed entry is the
  typed `ConfigError::InvalidTrustedProxy` naming the variable and the value.
  With the peer named, `x-forwarded-for` gives `Forwarded` with its left-most
  entry and `x-forwarded-proto: https` gives `Encrypted`; the same headers from
  an unnamed peer leave `Peer` and `Plaintext`. A missing peer gives no client
  address and forwards nothing. `listener_warning` is `Some` on a non-loopback
  address with no proxy named and `None` with a proxy named or on loopback,
  `secret_warning` is `Some` on a plaintext client leg and `None` on an
  encrypted one, and `WarnOnce` emits exactly one warning. The same run asserts
  the TCP front-end refuses to bind when the configured set cannot be read.
- `cargo test -p daemon --test proxy` -- over a real listener: the peer that
  opened a TCP connection reaches a handler as `ctx.peer_addr()`, the local
  socket reports no peer and forwards nothing even when the caller sends
  forwarded headers, and the same forwarded header is accepted from a named
  proxy and ignored from an unnamed one.

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
  thread; `tests/waker.rs` covers the waker lifecycle, and the crate's own
  `#[cfg(test)]` module drives one consumed clone's second `wake`, which only a
  caller that bypasses the authoring bridge can provoke. The waker tests assert
  the accounting rather than a log line: the host releases the one owning
  reference it hands each poll as soon as that poll returns, a wake that arrives
  after the future was dropped is recorded against the request id of the call it
  belongs to, and the fixture's `tool.deferred` and `tool.wake_deferred`
  capabilities are what provoke that late wake. The runtime starts no runtime of
  its own: it hands back a `Future` and the test's small loop polls it.
- `cargo test -p plugin-runtime --test bundle_verification` -- the bundle verifier
  at `verify_bundle`, over real bytes. The committed vectors under
  `crates/plugin-runtime/tests/vectors/` are rebuilt from the test signer and
  compared byte for byte on every run, so a changed offset or a changed domain
  tag fails the suite rather than passing quietly. The same run asserts the
  altered copy of the vector, every container refusal, every trust refusal, the
  chain profile, the recomputed key id, and that the compiled-in root set trusts
  nothing. The vectors are regenerated with `cargo test -p plugin-runtime --test
  bundle_verification -- --ignored`, which rewrites them from the signer after a
  deliberate format change; no other test writes a committed file.
- `cargo test -p plugin-runtime --test bundle_extraction` -- the extractor at
  `extract(verified, target)`, the seam `manifest-and-library` and
  `async-initialisation` read next. Every archive it reads is built inside the
  test: a well-formed bundle and hostile ones, so the refusals are exercised
  against archives the test wrote rather than against a committed fixture, and
  the declared sizes of a hostile archive's entries are rewritten after the
  writer produced them. The run asserts each refusal's own typed error, that an
  archive holding several targets stages only this host's files, that every
  entry is validated even when this host does not stage it, that a refused
  extraction removes its staging directory and a dropped `StagedBundle` removes
  its own, that a staged file and its directories carry the host's own modes,
  and that the entry count, per-entry size and total declared size limits are
  enforced at their edges.

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

Pairing and credentials, documented in `crates/daemon`, `crates/client` and
`crates/api-contracts`. Pairing is reachable on both transports; the local
socket needs no credential, and a caller that presents one is checked against
the store:

- `cargo test -p daemon --test secret` resolves the four secret-reference forms
  (`literal:`, `env:`, `keystore:`, `file:`, and a bare value as a literal) and
  refuses an unset variable, a missing file, a keychain tool that is there but
  refuses, and a secret file group or others can read. The keychain halves run
  against a stub tool on a narrowed `PATH`, so the machine's real keychain is
  never touched. The daemon's own configuration follows the same rule with one
  difference: an unset or empty `METIS_PRESHARED_KEY` resolves to no key, because
  pairing is optional, while a configured reference that cannot be resolved stays
  a typed error.
- `cargo test -p daemon --test auth` serves the daemon's own service over a
  real socket and drives it through the client harness: pairing mints one token
  whose hash is the only thing the daemon keeps, a wrong preshared key and a
  second pairing are refused, a call under `AuthPolicy::TokenRequired` is
  refused as missing, invalid, or revoked (each distinguishable), a valid
  credential is served, every paired surface reaches the same surface, and
  removing one row and restarting revokes that surface alone. A recording
  service proves a refused call never reached its handler. The same service is
  mounted on the service-taking TCP front-ends (`serve_tcp_service`, and
  `serve_tcp_listener_service` for a caller that must report the bound address)
  and answers over a real TCP socket: the service-taking twins exist because the
  interceptor chain lives on the service, so the `Router`-taking `serve_tcp` and
  `serve_tcp_listener` cannot carry a credential policy. The parity assertion
  mounts one service on the Unix and the TCP front-end and requires the same
  refusal and the same served answer from both. Pairing over the remote listener
  is driven the same way: the surface pairs with the preshared key over TCP, the
  minted token is what the authenticated call after it carries, and the pairing
  call reaches the handler as the procedure path alone. Removing the preshared
  key and restarting closes pairing on both transports, asserted by the same test
  family: the pairing call is refused as disabled over the Unix socket and over
  TCP, while a surface paired before the key was removed keeps being served on
  both. A wrong key and a repeated pairing reach the client as the typed
  `ClientError::Pairing` errors, read from the `metis-pair-error` response header
  and never from message text.
- `cargo test -p daemon` also covers the unit halves: the digest contract
  between the token `Pair` returns and the row the store wrote, the refusal
  headers, and the policy's exemption of the pairing procedure.
- `cargo test -p client --test credential` covers the credential store: the
  owner-only file at 600 in a directory at 700 when no keychain tool is on
  `PATH`, the keychain when a stub tool is, the reported backend, an unsafe
  surface name, and a data home that is not set.
- `cargo test -p api-contracts --test keychain` covers the platform tool's three
  operations against a stub that speaks both tools' command lines.

Windows local transport, documented in `crates/api-contracts/src/transport.rs`
and `crates/api-contracts/tests/pipe.rs`. The name rule, the resolver and the
assembled security descriptor are platform-independent and run on every host;
the pipe itself is compiled for the Windows target and executed nowhere in this
effort:

- `cargo test -p api-contracts` runs the platform-independent rules on any
  host. The default name is `\\.\pipe\metis-daemon`; an override is accepted
  only under the local pipe prefix `\\.\pipe\` (compared case-insensitively,
  because Windows pipe names are not case sensitive), and a remote path
  (`\\server\pipe\name`), a bare name, a Unix socket path, a name past the 256
  characters `CreateNamedPipeW` documents, and a name carrying an interior NUL
  are each the typed `ConfigError::InvalidPipeName` naming `METIS_ENDPOINT` and
  the value. An absent or empty variable is the platform default.
  `pipe_descriptor_sddl` assembles
  `D:P(D;;GA;;;NU)(A;;GA;;;<daemon-user-SID>)(A;;GA;;;SY)` for the SID it is
  given.
- `cargo test -p api-contracts --test pipe` on Linux runs the explicit skip test
  and passes, printing `[skip]` and naming the Windows invocation below, so a
  local run reports the missing pipe coverage instead of looking complete.
- `cargo test -p api-contracts --test pipe` on a Windows host would run the
  serve-and-connect test: it serves the test service over `serve_pipe`, reaches
  it through `pipe_transport`, requires one echo request to round-trip, and
  requires a second first-instance server to be refused. No runner in this
  effort executes it.
- The `windows` CI job runs on `windows-latest` and compiles for the Windows
  target with `cargo +nightly check -p api-contracts -p daemon --all-targets`.
  That compiles the pipe front-end, the connector, the descriptor and token
  calls, and the pipe test binary, and runs nothing.
- Reproducing that check on this Linux host needs the rustup nightly toolchain
  first on `PATH`, because the `rustc` inside `devenv shell` has no Windows std:
  `devenv shell -- bash -c 'export PATH="$HOME/.rustup/toolchains/nightly-2026-10-12-x86_64-unknown-linux-gnu/bin:$PATH"; cargo check -p api-contracts --all-targets --target x86_64-pc-windows-msvc'`.
  From `devenv shell` alone the same command fails `E0463` with "can't find crate
  for `std`". Measured at finalize: `-p api-contracts` and `-p client` both pass
  for the Windows target, and `-p daemon` stops in `plugin-contract`'s C build
  script, where the nix-wrapped clang cannot target `x86_64-pc-windows-msvc`. A
  native Windows runner with MSVC is expected to pass that step, and the job's
  first run is what proves it.

Gaps, each with the check that would close it. Nothing below was executed on
Windows in this effort, so the pipe is proven by construction, by compilation
for the Windows target, and by the platform-independent rules tested on Linux:

- The serve-and-connect test has never been run. A run of
  `cargo test -p api-contracts --test pipe` on a Windows host closes it.
- The descriptor has never been read back. `GetSecurityInfo` for
  `SE_KERNEL_OBJECT` on the pipe handle, asserting the DACL holds exactly the
  intended ACEs and none for `WD`, `AN`, or `BA`, closes it.
- The pipe mode has never been inspected. `GetNamedPipeInfo` on the server
  handle, asserting the reported mode includes `PIPE_REJECT_REMOTE_CLIENTS`,
  closes it, which is the assertion Microsoft's own HLK test makes.
- A rejected connection from another account has never been observed. A client
  running as a different local user closes it, and the exact error that client
  sees should be recorded, so it is not mistaken for "daemon not running".
- No low-integrity same-user process has been probed. An integrity-level check
  (`GetTokenInformation(TokenIntegrityLevel)` after
  `ImpersonateNamedPipeClient`) closes it, since setting a mandatory label needs
  `SeSecurityPrivilege`.
- No remote attempt has been recorded. A `\\HOSTNAME\pipe\name` attempt on the
  same machine, or a client on a second host, closes it, and the error it
  observes replaces the current inference.
- No client reaches the pipe yet. `Client::local` and `Client::with_endpoint`
  are Unix-only, so the wrapper has no Windows constructor: `pipe_transport` is
  the connector a follow-up ticket wraps. The client crate itself does compile
  for the Windows target, measured at finalize, so the missing constructor is
  the whole gap.

## CI

`.github/workflows/ci.yml` runs the Rust commands above through the devenv
shell on push to `main` and on pull requests, followed by the three contract
commands and the four plugin ABI commands (the C plugin build, the
authoring-feature-off check, the harness, and the header drift check). The
frontend commands are not in CI yet; add them when the frontend
has behaviour worth gating.

A second job, `windows`, runs on `windows-latest` and compiles the Windows local
transport for the Windows target with
`cargo +nightly check -p api-contracts --all-targets`. It executes nothing, so it
proves the Windows path compiles rather than proving a pipe works; the gaps it
leaves are listed in the Windows local transport block above, together with the
check that would close each one. It checks `api-contracts` alone because `daemon`
reaches `plugin-contract`, whose build script compiles a C file that MSVC's default
C mode rejects, so the daemon half failed on the plugin line rather than on the
transport (`docs/bugs/plugin-contract-c-static-assert-msvc.md` records that defect,
its two candidate fixes and its closing check). Measured on this repository's Linux
host with the Windows target: `api-contracts` and `client` both check clean and
`daemon` stops in that C file, so `-p client` joins the job whenever a Windows
client constructor exists.

## Mock conventions

- To be decided. Nothing is mocked yet.
