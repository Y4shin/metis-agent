---
type: deviation report
title: Deviation report for windows-named-pipe
status: stable
---

## Deviation report: windows-named-pipe

Implemented on `ticket/windows-named-pipe` (tip `e3f35db`, six `wip:` commits off
the approved `main` at `4d1b59c`). Compared against the
`## Pending ticket: windows-named-pipe` section of
`docs/tasks/metis-foundations/arch-spec.md` (approval record
`.work/approval.md`, committed as `4d1b59c`) and the ticket's five acceptance
criteria. Nine files changed: `Cargo.toml`, `Cargo.lock`,
`crates/api-contracts/Cargo.toml`, `crates/api-contracts/src/{lib,transport}.rs`,
`crates/api-contracts/tests/pipe.rs`, `.github/workflows/ci.yml`,
`docs/dev-env.md` and `docs/testing.md`.

I re-ran the evidence on the branch myself rather than reading it:

- `cargo test -p api-contracts --lib pipe` through `devenv shell --`: 6 passed,
  0 failed (default name, case-insensitive prefix, remote path and bare name
  refused, over-long name and interior NUL refused, absent-or-empty default, the
  assembled descriptor).
- `cargo test -p api-contracts --test pipe -- --nocapture` through
  `devenv shell --`: 1 passed, printing `[skip] the Windows named-pipe
  serve-and-connect test ... this host is linux. Run cargo test -p
  api-contracts --test pipe there to execute it.`
- `cargo check -p api-contracts --all-targets --target x86_64-pc-windows-msvc`,
  after a forced rebuild: exit 0, compiling the `#[cfg(windows)]` pipe, the
  descriptor and token calls, and the Windows-target test binary. This needs the
  rustup `nightly-2026-10-12` toolchain first on `PATH`. Run from
  `devenv shell` alone, the same command fails with `E0463` (`can't find crate
  for std`), because that `PATH`'s `rustc` is the nix one whose sysroot carries no
  Windows std. Both results are reproducible, and the caveat is worth a line in
  `docs/testing.md` beside the command, because every other documented command
  works from `devenv shell` alone.

### API surface changes

**No planned public surface differs from the actual one.** The actuals below are
the small details the section did not pin, plus one inherited wording error.

- **Planned:** `ConvertSidToStringW` for the daemon's own account SID.
  **Actual:** `ConvertSidToStringSidW` from
  `windows_sys::Win32::Security::Authorization`, which is the real Win32 and
  `windows-sys` name. **Impact:** none on dependents. The approved section names a
  symbol that does not exist, so the reconciler should correct that line, because
  a later ticket reading the spec would search for the wrong function. The
  `windows-pipe-security` findings task had the correct name.

- **Planned:** `windows-sys` features foundation, security, authorization, memory
  and threading. **Actual:** foundation, security, authorization and threading.
  `LocalFree` lives in `Win32::Foundation` in 0.61.2 and `Win32::System::Memory`
  has nothing this code names, while `OpenProcessToken` needs
  `Win32::System::Threading`. **Impact:** none. `windows-sys` is declared only
  under `[target.'cfg(windows)'.dependencies]`, so the Linux dependency tree is
  unchanged, and the lock gains one edge to a package that was already present
  through tokio and mio.

- **Planned:** the spec's Exports bullet places the `#[cfg(windows)]` test and the
  explicit-skip test in `crates/api-contracts/src/transport.rs` tests, while its
  own gap seam names `cargo test -p api-contracts --test pipe`.
  **Actual:** the serve-and-connect test and the non-Windows skip test live in
  `crates/api-contracts/tests/pipe.rs`, a real integration test, and the
  platform-independent rule tests stay in `src/transport.rs` as the bullet says.
  **Impact:** the invocation the seam and the gap list name exists, which is what
  criterion 4's skip and criterion 5's closing check need. The Exports bullet's
  placement wording is stale; the reconciler should follow the seam.

- **Actual:** two private items the section did not name, both inside
  `#[cfg(windows)]`. `create_pipe_instance` exists because building
  `SECURITY_ATTRIBUTES` inside the accept loop makes the served future non-`Send`
  and `tokio::spawn` refuses it (measured, not theoretical). `unsafe impl Send
  for PipeSecurityDescriptor` exists because the descriptor pointer does outlive
  awaits; it is deliberately not `Sync`, and carries a `SAFETY` comment.
  **Impact:** none public. The shared `serve_connection` was not relaxed to
  accommodate either.

- **Actual:** `PIPE_NAME_ENV` is a private `const` in `transport` holding
  `METIS_ENDPOINT`, beside the `client` crate's own private literal for the same
  variable, so the variable name now exists twice in the tree. The section did not
  ask for it to be exported. **Impact:** the map's fog entry about the endpoint
  variable being a private constant now covers two crates; one export removes
  both.

- **Planned and approved:** no `client` change, on the stated rationale that
  `client` does not compile for Windows because its credential store is
  Unix-only. **Actual:** no `client` change, and the rationale does not hold:
  `crates/client/src/credential.rs` carries a `#[cfg(not(unix))]` fallback and
  `cargo check -p client --all-targets --target x86_64-pc-windows-msvc` succeeds.
  **Impact:** the narrower gate is a choice rather than a necessity. Nothing in
  this ticket's surface depends on it, and the job scope was left exactly as
  approved. `pipe_transport` is the connector a later ticket wraps, and the
  missing client constructor stays a recorded gap.

### Abstraction usage

- Used/was specified: yes.

- The private `serve_connection` is the pipe's connection body, exactly as the
  spec required: `serve_pipe_service` calls it for every accepted instance, so the
  pipe is a third front-end rather than a second connection path, and no part of
  the body was changed for it.
- `ConnectRpcService`, `Router` and the interceptor chain are served the way the
  Unix and TCP front-ends serve them, and `serve_pipe`/`serve_pipe_service` mirror
  the landed `serve_uds`/`serve_uds_service` pair.
- `ConfigError` gained its variant with a `Display` arm and an `Error::source`
  arm, matching the enum's existing arms, and `resolve_pipe_name` copies
  `resolve_remote_addr`'s shape (absent or empty means the platform default, a
  malformed value is loud) through the file's existing `env_value` helper.
- `pipe_transport` copies `uds_transport`'s lazy-connector shape and reuses
  `local_authority`, so the pipe is the same local IPC surface as the socket.
- The `windows-pipe-security` findings were followed rather than re-derived:
  `reject_remote_clients(true)` is stated explicitly though tokio defaults it
  true, `first_pipe_instance(true)` is set on the first instance only,
  `create_with_security_attributes_raw` carries the descriptor, byte mode is
  tokio's default, and the SDDL string is the findings' string. No `interprocess`
  and no `widestring` was added.
- No contract, ABI or plugin change: `contracts/`, the generated trees, the ABI
  header and every plugin crate are untouched, so the ten-step gate's contract,
  drift and harness steps cannot move.

### Out-of-scope changes

- The two private `#[cfg(windows)]` items above.
- Prose-only module doc updates: `crates/api-contracts/src/lib.rs` and
  `transport.rs` now name the Windows front-end and the pipe's access rule, the
  same class of change `tcp-listener` recorded for its own front-end docs.
- `docs/testing.md` beyond the additions the spec named: the `## CI` section names
  the new job, and the gap list has seven bullets where criteria 5 names two,
  adding the never-run test, the descriptor read-back, the pipe-mode inspection,
  the low-integrity probe and the missing client constructor.
- `docs/dev-env.md`: the `Windows local transport` section the spec asked for, with
  the endpoint rule and the access rule.
- `Cargo.lock`: one dependency edge for the already-present `windows-sys`.

### Acceptance criteria

1. **Criterion 1, serve and connect over a duplex byte-mode pipe.** Implemented:
   `serve_pipe`, `serve_pipe_service`, `pipe_transport`, one instance at a time,
   and `tests/pipe.rs` drives an echo round trip plus the first-instance refusal.
   The test is compiled for the Windows target and **executed nowhere**, so this
   criterion rests on construction and compilation. That is the consequence of the
   approval decision to make the Windows job compile only, which the section
   states in its opening paragraph. Residual, with
   `cargo test -p api-contracts --test pipe` on a Windows host as the closing
   invocation.
2. **Criterion 2, default plus validated override.** Verified by me on Linux: five
   tests cover the default, a case-insensitive prefix, a `\\server\pipe\name` and a
   bare name refused as the typed `ConfigError::InvalidPipeName` naming the
   variable and value, an over-long name and an interior NUL refused, and an
   absent or empty variable meaning the default. `docs/dev-env.md` states the same
   rule.
3. **Criterion 3, the descriptor and remote rejection.** The assembled SDDL is
   unit-tested (protected DACL, the network-logon deny, the daemon account, SYSTEM),
   so the string is verified. That Windows records the descriptor on the pipe and
   the pipe mode carries `PIPE_REJECT_REMOTE_CLIENTS` is by construction only, and
   both are in the gap list with the `GetSecurityInfo` and `GetNamedPipeInfo`
   checks that would close them.
4. **Criterion 4, Windows compile in CI and an explicit local skip.** The `windows`
   job exists on `windows-latest`, installs `nightly` and runs `cargo +nightly check
   -p api-contracts -p daemon --all-targets`; the Linux skip is explicit and I
   observed the `[skip]` line with the test passing; and I reproduced the
   `api-contracts` cross-target compile at exit 0. The job has not run yet, so "in
   CI" is asserted by the workflow file and will first be exercised on the next
   push. The `daemon` half of that job is unverified locally, because its
   dependency chain needs an MSVC toolchain, which only the Windows runner has.
5. **Criterion 5, the gaps recorded with their closing checks.** Verified:
   `docs/testing.md` records the other-account connect (including that the exact
   error should be recorded so it is not mistaken for "daemon not running") and the
   remote attempt, each with the check that closes it, plus five further gaps this
   ticket leaves.

### Ticket doc update needed?

Yes. Append to `## Implementation notes`:

- The six divergences above, with the `ConvertSidToStringSidW` correction first
  and the measured correction to the client-gate rationale.
- That criterion 1 and half of criterion 3 are proven by construction and
  compilation only, because the CI job compiles and runs nothing, and that the
  window between accepts holds one instance so a second client can see
  `ERROR_PIPE_BUSY`.
- That `PipeName::parse` accepts the bare prefix `\\.\pipe\`, which
  `CreateNamedPipeW` then refuses at creation: loud, not silent, and outside the
  three rules the section set.
- The environment caveat for the cross-target check: it needs the rustup toolchain
  on `PATH`, and from `devenv shell` alone it fails with `E0463`. The exact
  invocation belongs in `docs/testing.md` beside the Windows commands.

### User attention needed?

Yes, briefly, and none of it changes the ticket's scope or a caller-visible
surface. Two items deserve a look before this lands:

1. **Criterion 1 and half of criterion 3 have no executed evidence anywhere**, by
   the approval decision to make the Windows job compile only. The section says so
   in its opening paragraph and the gap list names the checks that would close it,
   so this is disclosed rather than hidden; the operator should confirm that is the
   intended end state for the effort's last ticket, because nothing in the pipeline
   will ever run the pipe.
2. **The rationale for excluding `client` from the Windows gate was measurably
   wrong.** The gate scope stays as approved, and widening it would be a one-line
   change plus a client constructor, which is exactly the gap the section records
   for a follow-up ticket.
