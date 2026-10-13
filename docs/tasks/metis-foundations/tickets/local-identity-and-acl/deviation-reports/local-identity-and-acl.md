---
type: deviation report
title: Deviation report for local-identity-and-acl
status: stable
---

## Deviation report: local-identity-and-acl

Implemented on `ticket/local-identity-and-acl` off `main` at `9f5fde2`, HEAD
`fd2057d` (six checkpoints, four `wip:` plus one `docs:`). Compared against the
`## Pending ticket: local-identity-and-acl` section of
`docs/tasks/metis-foundations/arch-spec.md` and the ticket's three acceptance
criteria. I re-ran the gates on `fd2057d` through `devenv shell --`:
`cargo fmt --check` (`FMT-OK`), `cargo clippy --all-targets -- -D warnings`
(clean), `cargo test -p api-contracts -p client` (19 + 2 + 3 + 3 + 5 + 5 passed,
0 failed), `cargo run -p client --example echo` (`[result] PASS`, printing
`endpoint=/run/user/1000/metis/daemon.sock`, which exercises the new default),
and `bash scripts/regenerate-abi-header.sh --check` (no drift).

### API surface changes

- **Planned:** `ensure_socket_dir` (the spec's only ACL export) creates the
  socket's directory and sets its mode; the ACL is a property of the directory.
  **Actual:** `serve_uds` resolves the ACL once from the environment, ensures
  the directory, removes the stale socket, binds, and then applies the same rule
  to the bound socket file (`crates/api-contracts/src/transport.rs`). The rule is
  applied by the listener rather than by the caller, as planned, and it now
  covers the socket inode as well.
  **Impact:** the ticket's criterion 1 says "the socket and its directory", and
  on Linux `connect(2)` checks write permission on the socket inode, so a 770
  directory with a 700 socket would silently defeat the configured group. No
  signature moved. `windows-named-pipe` has its own rule and is unaffected.
- **Planned:** `IdentityError` had two variants, `MissingHeader` and
  `UnsupportedVersion`.
  **Actual:** a third variant, `MalformedVersion { value: String }`, for a
  version header that is present but not a number. The enum is
  `#[non_exhaustive]`.
  **Impact:** additive. `pairing-and-tokens` and the remote tickets must match
  non-exhaustively, which the enum already requires.
- **Planned:** `ConfigError::SocketAcl { path, source }` was the one new variant.
  **Actual:** `ConfigError::UnknownSocketGroup { variable, value }` joins it, so a
  `METIS_SOCKET_GROUP` naming no group on the host is refused instead of falling
  back to the primary group. The enum stays `#[non_exhaustive]`.
  **Impact:** additive; the same non-exhaustive match rule applies.
- **Planned:** the typed refusal crosses the wire as the refusal's
  `metis-supported-versions` header, with no named constant.
  **Actual:** `api_contracts::identity` now owns the whole channel:
  `SUPPORTED_VERSIONS_HEADER`, `refusal_headers(&IdentityError) -> HeaderMap`,
  `supported_versions_value() -> String`, and
  `parse_supported_versions(&str) -> Vec<u32>`, and the wrapper's mapping reads
  the header rather than parsing message text.
  **Impact:** four additive helpers the spec did not name, but they close a real
  drift risk by putting both ends of the format in one module. Downstream
  tickets must use these names rather than inventing their own header spelling.
- **Planned:** `Client::with_surface` "sets the surface through `config_mut()`".
  **Actual:** `connectrpc` 0.9.1 exposes only consuming
  `with_default_header`/`with_default_headers` on `ClientConfig` and no mutable
  default-header accessor, so `with_surface(mut self, Surface) -> Self` rebuilds
  the config, which holds only the authority and the declaration.
  **Impact:** none observable; the spec sentence described an API the
  dependency does not have. The method signature is as specified.
- **Planned:** `ClientError::ProtocolVersion { found, supported }`, with
  `supported` from the refusal header.
  **Actual:** as specified, and `found` is
  `identity::PROTOCOL_VERSION`, the constant the wrapper declares, rather than a
  read-back from the built config.
  **Impact:** none; a single source for the declared version.
- **Planned:** no new dependency anywhere in the spec.
  **Actual:** `libc` is added as a workspace dependency and to
  `api-contracts`, Unix targets only, for `chown(2)` (group only, with
  `uid = (uid_t::MAX)`, so the owner never changes) and for
  `getgrnam_r`/`getgrgid_r`.
  **Impact:** the first new third-party dependency this line adds, and it widens
  `api-contracts`' unix closure. `std` has neither the group database nor
  `chown`, and the alternatives (a `chgrp` subprocess, parsing `/etc/group`) are
  worse. The spec's phrase "no chown, and no privileged requirement" is now
  imprecise: the owner never changes and no privilege is needed, but the group
  half does call `chown(2)`.
- **Planned:** the endpoint rule moves to `$XDG_RUNTIME_DIR`, then `$TMPDIR`,
  then `EndpointError::NoPrivateBase`, with both resolver signatures gaining a
  `Result`.
  **Actual:** exactly that, observed end to end: the example resolves
  `/run/user/1000/metis/daemon.sock` and completes.

### Abstraction usage

- Used/was specified: yes.
  - `serve_stream` and `serve_uds`'s accept-loop shape are unchanged; the ACL is
    slotted in before bind, not layered over the loop.
  - The `connectrpc` metadata path as specified: default headers on
    `ClientConfig`, `RequestContext::header` on the server, a `ConnectError` with
    an attached response header for the refusal. No new RPC and no generated-tree
    change: the contract, the generated trees, the ABI layout and the plugin
    crates are untouched, so the drift, layout and harness gaps stay as they
    were.
  - The non-exhaustive `ConfigError` and `ClientError` were extended, not
    replaced. `EndpointOverride`, `TempSocket`, `Client::with_endpoint`,
    `skeleton::router()` and `SkeletonDaemon` are reused; `router()` itself is
    unchanged and `router_recording()` sits beside it.
  - No credential, no token, no authorization check, and no logging framework
    appear anywhere in the diff.
- One process note rather than a scope note: the `identity` module landed with
  its unit tests in the same step, so its RED evidence is the client crate's
  earlier `unresolved import api_contracts::identity` rather than a per-test RED
  run. Every other seam has a real RED run.

### Out-of-scope changes

- Two landed tests were edited because this ticket's rule moves under them:
  `crates/client/tests/echo.rs`'s `TempSocket` now creates its base directory
  private (the ACL correctly refuses the 755 base the landed suite created), and
  `default_endpoint_is_the_platform_default` was deleted because the
  `/tmp/metis-daemon.sock` literal no longer exists. The rule moved to unit tests
  in `crates/client/src/lib.rs`, where the RAII environment helper lives. The
  example `crates/client/examples/echo.rs` and the client tests that call the
  resolver were updated for the new `Result`.
- `docs/dev-env.md` gains the new default endpoint and the socket-directory rule.
  The spec named `docs/testing.md` only; `docs/testing.md` was also updated, with
  the access-control and identity run commands.
- `crates/api-contracts/Cargo.toml` and the workspace `Cargo.toml` gained the
  `libc` dependency described above.
- Nothing was removed from the transport surface or from the plugin crates.

### Ticket doc update needed?

Yes. The ticket has no `## Implementation notes` section yet (it ends at
`## Blocked by`), so the land-worker must create the heading and record:

- the landing paragraph (merge commit, landing branch, working branch), as the
  three earlier landings do;
- the endpoint rule that replaced `/tmp/metis-daemon.sock`, and that
  `default_endpoint()`/`resolve_endpoint()` now return `Result`;
- that the ACL covers the socket file as well as its directory, and that
  `serve_uds` applies it so every local bind carries it;
- the group-only `chown` and the `libc` dependency, with the reason `std`
  cannot do it;
- the additive surfaces: `IdentityError::MalformedVersion`,
  `ConfigError::UnknownSocketGroup`, `SUPPORTED_VERSIONS_HEADER`,
  `refusal_headers`, `supported_versions_value`, `parse_supported_versions`;
- the two landed tests touched and why, and the deleted literal test;
- the residual risks below.

### User attention needed?

Yes, on two settled facts rather than on an open decision.

- The spec's "no chown" wording should be corrected to "the owner never changes
  and no privilege is needed; the configured group is applied with `chown(2)`",
  and the new `libc` dependency belongs in the spec's export list. Both are the
  approved decision implemented faithfully; the wording is what drifted.
- Acceptance criterion 1 still says "a configured owner and group". Per the
  approved decision there is no configured owner: the directory is owned by the
  daemon's user by construction, and only the group is configurable. The ticket
  text or the implementation note should say so.

### Residual risks carried forward

- The "directory not owned by this user" refusal is implemented but not
  integration-tested: creating a directory owned by another uid needs root, so
  only the symlink, non-directory, wider-mode and unknown-group refusals are
  asserted.
- The recording is per handler. `RecordingDaemon` implements the one RPC the
  contract has, so `event-stream`'s `StreamEvents` handler must call
  `identity::from_headers` too, or streaming calls are unrecorded rather than
  refused. The arch spec already records this residual.
- A daemon restarted without `METIS_SOCKET_GROUP` after a run with it meets a 770
  directory its 700 rule refuses; an operator removing the group must also remove
  the directory.
- The client's version refusal is proven in two layers (the wire refusal through
  the generated client, and the wrapper's mapping over a synthetic refusal)
  rather than one end-to-end call, because the wrapper always declares the
  version this build speaks.
- `version_refusal` accepts any failure whose response carries a parsable
  non-empty `metis-supported-versions` header, from any peer. That is the
  intended channel, but a peer setting that header for another reason would be
  reported as a version refusal.
