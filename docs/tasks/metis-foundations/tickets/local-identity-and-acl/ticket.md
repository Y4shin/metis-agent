---
type: ticket
subtype: feature
title: Local access control and surface identity
status: stable
workflow_state: done
blocked_by:
- local-transport-rpc-skeleton
size: s
---

## What to build

Local access control from the operating system, and the identity a client
declares when it connects, so that local clients need no credential and the
daemon's logs say which surface did what.

## Acceptance criteria

- [ ] The socket and its directory are created with a configured owner and group
      and mode 700 or 770, asserted by a test that reads the resulting
      permissions.
- [ ] The connection handshake carries the protocol version, a surface name, and
      a client version, and the daemon records them; the handshake is not
      authentication.
- [ ] A mismatched protocol version is rejected with a typed error that lists
      what is supported.

## Blocked by

- `local-transport-rpc-skeleton`, because the handshake rides the local transport
  connection.

## Implementation notes

Landed on `task/local-identity-and-acl` as the merge commit `694fe0c`
("ticket(local-identity-and-acl): Local access control and surface identity"),
parents `9f5fde2` (the landing branch, created at this ticket's arch spec commit,
which is also `main`'s tip and carries every earlier landing merge) and `fd2057d`
(working branch `ticket/local-identity-and-acl`, six checkpoints: four `wip:`
commits, one `docs:` commit, on top of the approved spec commit). The merge was
clean with no conflicts and `git diff fd2057d HEAD` is empty, so the landing tree
is byte for byte the tree the verify worker gated. `ticket/local-identity-and-acl`
was deleted after the merge. The review artifact
`deviation-reports/local-identity-and-acl.md` is committed with this note.

The gates were not re-run here, because the landing tree is identical to the
verified tree. On `fd2057d`, through `devenv shell --`: `cargo fmt --check`
(`FMT-OK`), `cargo clippy --all-targets -- -D warnings` (exit 0, the only output
being the pre-existing clang `_FORTIFY_SOURCE` warning from the
`plugin-contract` C translation unit), `cargo test -p api-contracts -p client`
(19 unit + 2 `acl` + 3 `echo` + 5 `identity` + 3 `remote` on the client side, 0
failed), the workspace `cargo test` (88 passed, 0 failed), `buf lint`, `buf
breaking --against '.git#branch=main'`, both `--check` regeneration scripts, the
C plugin build, and the ABI harness (73 checks). The example
`cargo run -p client --example echo` printed
`endpoint=/run/user/1000/metis/daemon.sock`, so the new default endpoint rule is
exercised end to end.

What landed, and which criterion each piece serves:

1. **Criterion 1, the ACL.** `crates/api-contracts/src/transport.rs` holds
   `SOCKET_MODE_PRIVATE` (0o700), `SOCKET_MODE_GROUP` (0o770), `SOCKET_GROUP_ENV`
   (`METIS_SOCKET_GROUP`), the private `SocketAcl` type and its
   `ensure_dir`/`check_dir`/`apply_acl`/`resolve_group` helpers, the public
   `ensure_socket_dir`, and the `serve_uds` wiring. The metadata is asserted by
   reading it back: mode 700 and `uid == geteuid()` for a fresh directory, mode
   770 and the configured gid when `METIS_SOCKET_GROUP` names a group the process
   belongs to, the same mode on the bound socket file, and refusal with no chmod
   for a directory that is a symlink, not a directory, wider than the rule
   allows, or owned by another user. `crates/client/tests/acl.rs` reads the modes
   of the directory and the socket that a served `uds_transport` daemon creates,
   independently of the unit tests. Measured by hand as well: the example leaves
   `drwx------` and `srwx------` with no group, and
   `drwxrwx---`/`srwxrwx---` with `METIS_SOCKET_GROUP=wheel`. One wording caveat:
   the criterion says "a configured owner and group", but only the group is
   configurable. The directory is owned by the daemon's user by construction, and
   the group half of the ACL is applied with `chown(2)` with `uid = uid_t::MAX`,
   so the owner never moves.
2. **Criterion 2, the declaration.** `crates/api-contracts/src/identity.rs` (new)
   holds `PROTOCOL_VERSION`, `SUPPORTED_PROTOCOL_VERSIONS`, the three request
   header names, `SUPPORTED_VERSIONS_HEADER`, `ClientIdentity` with its
   accessors and `from_headers`, and the non-exhaustive `IdentityError`.
   `crates/client/src/lib.rs` sets the three default headers in its one config
   builder and adds `Surface`/`with_surface`. `crates/client/src/skeleton.rs`
   adds `IdentityRecorder` and `router_recording`, which records what a call
   declared; `router()` is unchanged. `crates/client/tests/identity.rs` drives a
   wrapper call over a real socket against the recording router and asserts the
   recorder saw version 1, surface `cli`, and the client version, plus the
   `unspecified` default. No credential, token, or authorization check exists
   anywhere on the path; the doc comment on `from_headers` states that reading
   the declaration grants nothing.
3. **Criterion 3, the version refusal.** `from_headers` returns
   `IdentityError::UnsupportedVersion { found, supported }`, whose `Display` names
   the supported versions, and `refusal_headers` puts
   `SUPPORTED_VERSIONS_HEADER` (`metis-supported-versions`) on the refusal. The
   integration tests assert that a version-2 client and a client with no version
   header are both refused with `InvalidArgument` carrying that header, and the
   wrapper maps the header to `ClientError::ProtocolVersion { found, supported }`
   without parsing message text.

Divergences from the approved spec, from `deviation-reports/local-identity-and-acl.md`
and the implementation report. All are additive, or prose, or a correction of a
spec sentence that named an API the dependency does not have:

1. `IdentityError::MalformedVersion { value }` is a third variant beside
   `MissingHeader` and `UnsupportedVersion`. A version header present but not a
   number is reachable peer input and neither specified variant can describe it.
   The enum is `#[non_exhaustive]`, so the growth is not breaking.
2. `ConfigError::UnknownSocketGroup { variable, value }` joins
   `ConfigError::SocketAcl { path, source }`. A group name the host does not have
   must not silently fall back to the primary group, and `SocketAcl` has no path
   to name before the directory is resolved. The enum stays `#[non_exhaustive]`.
3. The ACL covers the bound socket file as well as its directory. The spec's
   export list named only `ensure_socket_dir`, but criterion 1 says "the socket
   and its directory", and on Linux a connect checks write permission on the
   socket inode, so a 770 directory with a 700 socket would silently defeat the
   configured group. `serve_uds` resolves the rule once from the environment and
   applies it to both, so every local bind carries it and no caller can skip it.
   No signature moved.
4. `api_contracts::identity` owns the whole refusal channel:
   `SUPPORTED_VERSIONS_HEADER`, `refusal_headers`, `supported_versions_value`,
   and `parse_supported_versions`. The spec said the typed error crosses the wire
   as a header but named neither the header nor its format, so one module now
   owns both ends and the server and client cannot drift. Downstream tickets must
   use these names rather than inventing their own spelling.
5. `Client::with_surface` rebuilds `ClientConfig` instead of mutating it.
   `connectrpc` 0.9.1 exposes only consuming `with_default_header` and
   `with_default_headers` and no mutable default-header accessor, and appending
   would leave two `metis-surface` headers with the stale one read first. The
   config the wrapper builds holds only the authority and the declaration, so the
   rebuild is lossless. The method signature is as specified.
6. `found` in `ClientError::ProtocolVersion` is `identity::PROTOCOL_VERSION`, the
   constant the wrapper declares, rather than a read-back from the built config.
   The spec only required `supported` to come from the refusal header.
7. `libc` is a new workspace dependency and a dependency of `api-contracts`,
   `[target.'cfg(unix)'.dependencies]` only. `std` exposes neither `chown` nor
   `getgrnam_r`, and a `chgrp` subprocess or hand-parsed `/etc/group` would be
   worse. This is the first new third-party dependency this line adds, and it
   widens `api-contracts`' unix closure. The spec's "no chown" phrasing is now
   imprecise: the owner never changes and no privilege is needed, but the group
   half does call `chown(2)`.
8. `default_endpoint()`/`resolve_endpoint()` return `Result` and the endpoint
   rule replaces the `/tmp/metis-daemon.sock` literal with `$XDG_RUNTIME_DIR`,
   then `$TMPDIR`, then `EndpointError::NoPrivateBase`. Observed end to end by
   the example.

Two landed tests were changed because this ticket's rule moves under them, and
both were reviewed as justified rather than gate-shopping:

- `crates/client/tests/echo.rs`'s `TempSocket` now creates its base directory
  mode 700. The landed helper created it with the umask default of 755, and the
  new rule correctly refused a directory wider than 700. The assertion the test
  makes is unchanged, so no served-path coverage was weakened. The comment at the
  top of the file records why the endpoint rule is not re-tested in that binary:
  mutating `TMPDIR` there would race the temp-dir helper.
- `default_endpoint_is_the_platform_default` was deleted, because its literal no
  longer exists. The rule is re-tested as unit tests in `crates/client/src/lib.rs`,
  where the arch spec places it and where the RAII environment helper lives.

`docs/testing.md` gained the access-control and identity run commands, and
`docs/dev-env.md` records the new default endpoint and that the daemon creates
and restricts the socket directory. The contract, the generated trees, the ABI
layout, and the plugin crates are untouched, so no drift, layout, or harness gap
moved.

Residual risks carried forward:

- The "directory not owned by this user" refusal is implemented but not
  integration-tested, because creating a directory owned by another uid needs
  root. The symlink, non-directory, wider-mode, and unknown-group refusals are
  asserted.
- The recording is per handler. `RecordingDaemon` implements the one RPC the
  contract has today, so when `event-stream` adds `StreamEvents` that handler
  must call `identity::from_headers` too, or those calls are unrecorded rather
  than refused. The arch spec already carries this residual for the daemon
  effort.
- A daemon restarted without `METIS_SOCKET_GROUP` after a run with it finds a 770
  directory that its 700 rule refuses. Refusing is what the spec asks for, so an
  operator who removes the group must also remove the directory.
- The client's version refusal is proven in two layers, the wire refusal through
  the generated client and the wrapper's mapping over a synthetic refusal, rather
  than one end-to-end call: the wrapper always declares the version this build
  speaks, so a mismatch needs a peer that refuses it, which no API in this ticket
  provides.
- `version_refusal` accepts any failure whose response carries a parsable,
  non-empty `metis-supported-versions` header, from any peer. That is the
  intended channel, but a peer that sets the header for another reason would be
  reported as a version refusal.
- TDD ordering gap: the `identity` module landed with its unit tests in the same
  step, so its RED evidence is the client crate's earlier `unresolved import
  api_contracts::identity` rather than a per-test RED run. Every other seam has a
  real RED run.

Knowledge harvested at finalize: the effort map gained the decision bullet for
the ACL, the identity declaration and the endpoint rule, and the fog entry that
recorded the `/tmp` conflict is removed because the rule settles it.
`docs/testing.md` and `docs/dev-env.md` already carried the run commands, the new
default endpoint and the socket-directory rule from the implementation, so they
needed no further change. Known wording debt, recorded rather than fixed:
acceptance criterion 1 says "a configured owner and group", but only the group is
configurable, and the arch spec's residual list states the actual rule.
