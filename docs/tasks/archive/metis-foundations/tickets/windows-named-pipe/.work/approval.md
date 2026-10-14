# Approval: `windows-named-pipe` architecture spec

- Approved: 2026-10-14, in chat, by the operator via the batched decision
  questionnaire that followed the draft.
- Approved artifact: `docs/tasks/metis-foundations/arch-spec.md`, section
  `## Pending ticket: windows-named-pipe`.
- Ticket: `docs/tasks/metis-foundations/tickets/windows-named-pipe/ticket.md`.

## Decisions taken at approval

1. **The Windows CI job compiles only.** It checks the pipe path for the Windows
   target and runs nothing. The consequence is recorded in the section rather than
   hidden: no pipe code path is executed anywhere in this effort's CI, the
   serve-and-connect test is compiled but never run, and the map's "compile-gated
   evidence only" note stands rather than being superseded by a Windows runner that
   does not run anything.
2. **The compile gate covers `api-contracts` and `daemon` with `--all-targets`**,
   so the pipe, the descriptor, the name rule and the transport's own test binaries
   are compiled for Windows, while `client` stays out because its credential store
   is Unix-only today.
3. **The descriptor is the findings' string**: `D:P(D;;GA;;;NU)(A;;GA;;;<daemon-user-SID>)(A;;GA;;;SY)`,
   a protected DACL, the network-logon deny ACE, the daemon's own account, and
   SYSTEM for system tooling and inspection.
4. **The client side stays out of scope.** `pipe_transport` and `serve_pipe` live in
   `api-contracts` with their tests, criterion 1 is met by a test that drives the
   pipe directly, and the client crate's missing Windows credential path is recorded
   as the named gap a follow-up closes rather than solved here.

## State verified before approval

- `windows-sys` is already in `Cargo.lock` at 0.61.2 through tokio and mio, and no
  manifest declares it, so this ticket adds one target-gated declaration. The map
  already decided tokio plus `windows-sys` and no `interprocess`.
- tokio is pinned at 1.53.2 and its `net` feature is already enabled. Its Windows
  named-pipe module has `ServerOptions::reject_remote_clients` (defaulting to
  `true`), `first_pipe_instance`, `create`, and the `unsafe
  create_with_security_attributes_raw` the findings prescribe, so the descriptor
  needs no raw Win32 fallback.
- `crates/api-contracts/src/transport.rs` already shares one private
  `serve_connection` between the Unix and TCP front-ends, and `NamedPipeServer` is
  `AsyncRead + AsyncWrite`, so the pipe becomes a third caller of that body rather
  than a new connection path.
- The client crate's credential module is `#[cfg(unix)]` and `Client`'s
  configuration calls it unconditionally, so `client` does not compile for Windows
  today. That is what makes decision 4 a scope decision rather than a wiring
  detail, and it is why the gate is narrower than the workspace.
- `devenv` provides rust through nix at the `nightly` channel with no `rustup`,
  which is why the Windows compile gate is a `windows-latest` job with its own
  toolchain rather than a cross-check on the existing `ubuntu-latest` job.

## Not settled here, by design

Running the pipe tests anywhere in CI, a Windows credential store for the client,
the logon-SID principal, the `GetNamedPipeServerProcessId` squatter check, a
low-integrity same-user probe, the exact error a remote client observes, and the
descriptor read-back with `GetSecurityInfo` and the pipe-mode check with
`GetNamedPipeInfo`. Each is recorded as a gap in the section, with the check that
would close it.
