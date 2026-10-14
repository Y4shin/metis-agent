---
type: ticket
subtype: feature
title: Windows named pipe transport
status: stable
workflow_state: ready
blocked_by: [local-transport-rpc-skeleton]
size: s
---

## What to build

The Windows local transport, restricted to the intended account, so that a
Windows desktop client reaches the daemon without a network listener.

## Acceptance criteria

- [ ] The transport module serves and connects a Connect service over a duplex
      byte-mode named pipe.
- [ ] The pipe name comes from the platform default with an environment variable
      override, and the override is rejected unless it stays a local pipe path.
- [ ] The pipe descriptor restricts access to the daemon's account plus SYSTEM,
      and remote clients are rejected.
- [ ] The Windows path compiles for the Windows target in CI, and local test runs
      skip it with an explicit skip rather than a silent pass.
- [ ] A rejected connection from another account or a remote attempt is recorded
      as a documented gap if no Windows runner is available, naming the check
      that would close it.

## Blocked by

- `local-transport-rpc-skeleton`, because the pipe implementation shares the
  transport module and the service it carries.

## Implementation notes

Landed on `task/windows-named-pipe` as the merge commit `84bdc61`
("ticket(windows-named-pipe): Windows named pipe transport"), parents `4d1b59c`
and `e3f35db`. The landing branch did not exist when this ticket was landed, so
it was created at `main`'s tip (`4d1b59c`, "docs(windows-named-pipe): approve the
arch spec"), the convention the shared conventions section of
`docs/tasks/metis-foundations/arch-spec.md` records and the one the earlier
landings used. `e3f35db` is the working branch `ticket/windows-named-pipe`, six
`wip:` commits off `main` at `4d1b59c`. The merge was clean with no conflicts,
and the landing tree is byte-identical to the verified tip (both trees hash to
`5321156e8e`), so every source file landed exactly as verified and nothing was
re-run at landing. The branch `ticket/windows-named-pipe` was deleted after the
merge, and the deviation report is committed alongside this note at
`docs/tasks/metis-foundations/tickets/windows-named-pipe/deviation-reports/windows-named-pipe.md`.

The gates recorded against `e3f35db` therefore hold for the landed tree, all of
them through `devenv shell --`: `cargo fmt --check` exit 0; `cargo clippy
--all-targets -- -D warnings` exit 0; the workspace `cargo test` exit 0 with 45
suites and 219 passed, 0 failed; `cargo test -p api-contracts --lib pipe` at 6
passed, 0 failed; and `cargo test -p api-contracts --test pipe` at 1 passed, 0
failed, printing the `[skip]` line that names the Windows host. The cross-target
check `cargo check -p api-contracts --all-targets --target
x86_64-pc-windows-msvc` also exits 0, compiled by the rustup
`nightly-2026-10-12` toolchain (`080cc2714`, the rustc the devenv shell reports)
because the nix toolchain carries only the Linux target.

What landed, 9 files and +903/-6 against `main`: the Windows half of the local
transport in `crates/api-contracts/src/transport.rs` (`PIPE_PREFIX`,
`DEFAULT_PIPE_NAME`, `PipeName`, `resolve_pipe_name`, `pipe_descriptor_sddl`, and
under `#[cfg(windows)]` `serve_pipe`, `serve_pipe_service`, `pipe_transport`,
the private `create_pipe_instance`, `PipeSecurityDescriptor` and
`user_sid_string`), all of it serving the shared private `serve_connection` so
the pipe is a third front-end rather than a second connection path;
`ConfigError::InvalidPipeName` on the non-exhaustive enum; the new
`crates/api-contracts/tests/pipe.rs` (the `#[cfg(windows)]` serve-and-connect test
and the `#[cfg(not(windows))]` explicit skip); `windows-sys` target-gated in
`crates/api-contracts/Cargo.toml`, `Cargo.toml` and one `Cargo.lock` edge; the
`windows` job in `.github/workflows/ci.yml` on `windows-latest`; and the Windows
sections in `docs/dev-env.md` and `docs/testing.md`.

The acceptance criteria stand as the deviation report and the verifier record
them. Criterion 2 (the default name and the validated override) and the string
half of criterion 3 (the assembled descriptor) are executed on Linux. Criterion
4 (the Windows compile job and the explicit local skip) rests on the workflow
file plus the reproduced `api-contracts` cross-target compile, with the `daemon`
half of the job's command unverified locally. Criterion 5's gaps are in
`docs/testing.md`, seven of them, each naming its closing check. Criterion 1
(serve and connect) and the application half of criterion 3 rest on construction
and compilation only, because the approved Windows job compiles and runs
nothing.

Residual risks carried into the effort, all recorded by the tdd worker, the
deviation report, and the verifier. The serve-and-connect test has never executed
on any host, since no Windows runner or host was available, and `cargo test -p
api-contracts --test pipe` on a Windows host is the closing invocation. The
`daemon` half of the CI job's command cannot be cross-checked here, because
`plugin-contract`'s C build step needs an MSVC toolchain (`lib.exe`, or
`clang: unsupported option '-fPIC' for target 'x86_64-pc-windows-msvc'`), an
environment limit on a file this ticket does not touch. The descriptor is applied
by construction and never read back, so `GetSecurityInfo` and
`GetNamedPipeInfo` are recorded gaps. `PipeName::parse` accepts the bare prefix
`\\.\pipe\`, which `CreateNamedPipeW` then refuses at creation, so the failure
is loud rather than silent. The pipe holds one instance at a time, so a second
client in the window between an accept and the next instance creation can see
`ERROR_PIPE_BUSY`.

The deviation report raised two items for attention, and neither changes this
ticket's scope. Criterion 1 and half of criterion 3 have no executed evidence
anywhere by the approval decision that the Windows job compiles only, so the
operator should confirm that is the intended end state for the effort, because
nothing in the pipeline will run the pipe. The approved rationale for excluding
`client` from the Windows gate is measurably wrong: `crates/client/src/credential.rs`
carries a `#[cfg(not(unix))]` fallback and `cargo check -p client --all-targets
--target x86_64-pc-windows-msvc` succeeds, so the narrower gate is a choice
rather than a necessity; the job scope was left exactly as approved.

One recommendation of the deviation report was not applied, because this landing
holds no source or documentation edit beyond this ticket doc. The report asks for
the cross-target invocation and its environment caveat to sit in
`docs/testing.md` beside the Windows commands, since that check fails with
`E0463` from `devenv shell` alone and needs the rustup `nightly-2026-10-12`
toolchain on `PATH` first. `docs/testing.md` currently names the commands and the
CI job but not that requirement, so a verifier following the doc alone cannot run
the Windows compile check without discovering the toolchain gap. The reconciler or
the knowledge harvest can carry that line.

`docs/tasks/state.yaml` is modified in the working tree only (the harness wrote
`task: windows-named-pipe`), and it is not committed on either branch, so it is
not touched by this landing commit, which holds only this ticket doc and the
deviation report. Its final value stays with `finalize-task` together with the
ticket's `workflow_state` and the archive.
