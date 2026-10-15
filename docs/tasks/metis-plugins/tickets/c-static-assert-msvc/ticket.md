---
type: ticket
subtype: bug
title: Fix the C static assertion so the Windows target compiles
status: stable
workflow_state: ready
blocked_by: [abi-initialisation-entry]
size: s
---

## What to build

The contract's C translation unit compiles under MSVC's C mode, so the runtime
and the daemon can be built for the Windows target instead of only the API
contracts crate. The assertion's intent is preserved: a layout mismatch still
fails the build, in a form MSVC accepts. The Windows CI job widens to the daemon.

## Acceptance criteria

- [ ] The pinned MSVC invocation compiles the C translation unit with warnings
      denied.
- [ ] A deliberate layout mismatch still fails the build under both gcc and
      clang, so the fix does not trade the check for compilation.
- [ ] The Windows CI job compiles the daemon, not only the API contracts crate.
- [ ] The recorded testing gap is narrowed or closed in `docs/testing.md`.
- [ ] The bug file moves to the archive with its resolution recorded.

## Blocked by

- `abi-initialisation-entry`

## Implementation notes

Landed on `task/c-static-assert-msvc` as the merge commit `e1099f5`
("ticket(c-static-assert-msvc): Fix the C static assertion so the Windows target
compiles"), parents `d1a86a2` (the landing branch tip, `docs(metis-plugin-sdk):
record the authoring trait break it inherits`) and `963227f` (the working branch,
`wip: c-static-assert-msvc the Windows measurement is recorded exactly`). The
merge was clean with no conflicts, and `git diff 963227f HEAD` is empty, so the
landed tree is byte-identical to the verified tip. The branch
`ticket/c-static-assert-msvc` was deleted after the merge, and no source, test or
config file was changed by the landing step. `docs/tasks/state.yaml` is left
uncommitted, matching the `475f1c0` precedent, because `finalize-task` owns the
workflow state.

The implementation is 8 files, 489 insertions and 230 deletions against
`d1a86a2`, over four `wip:` commits plus the archive commit: the CI workflow,
`crates/plugin-contract/build.rs`, `c/layout_static_asserts.c`,
`crates/plugin-contract/tests/c_layout.rs`, `docs/testing.md`,
`scripts/check-c-layout-asserts.sh`, and the bug file moved from
`docs/bugs/` to `docs/bugs/archive/`.

On the landed tip, `devenv shell -- bash scripts/check-c-layout-asserts.sh` exits
0: gcc and clang both accept the unit in strict ISO C89 and in `-std=c11` with
warnings denied, and both reject the deliberately broken copy in both modes
(`size of array 'metis_c_layout_assert_MetisStatus_size' is negative` under gcc,
`declared as an array with a negative size` under clang), so the negative half
of criterion 2 is measured on the landed tree and not only on the working
branch. The verifier's full run against `963227f` was `cargo fmt --all --check`
and `cargo clippy --all-targets -- -D warnings` clean, `cargo test` green across
48 targets with 0 failures, `scripts/regenerate-abi-header.sh --check` reporting
no drift, `scripts/build-c-plugin.sh` accepting the committed header under gcc
and clang, and `cargo run -p plugin-fixture --bin abi-harness` printing
`[result] PASS (82 checks)`. Because the landed tree is byte-identical to that
tip, those results carry to the landing branch unchanged.

Two items are handed to `finalize-task`, not done here. First,
`docs/bugs/archive/plugin-contract-c-static-assert-msvc.md` records
`fix_commit: 9cef2c0`, the third commit on the working branch rather than the
landing commit; the bug-closure step must read the archive path (the pre-archive
path no longer exists) and replace it with `git rev-parse task/c-static-assert-msvc`,
which is `e1099f5`. Second, three documents still describe the bug as open at its
pre-archive path, and each belongs to a later step in the chain:
`docs/tasks/metis-plugins/map.md:223`, `docs/tasks/metis-plugins/arch-spec.md:627`
(a dangling path in the seam the spec-reconciler owns) and
`.work/handoff/metis-plugins.md:205`.

Residual risk carried into finalize: no `cl.exe` exists on this host, so
criterion 1's exact MSVC invocation and criterion 3's widened `windows` CI job
have never executed. The first run of that job on a runner with MSVC is the only
proof of those two halves. The strict-C89 compile under gcc and clang, plus the
byte-identical tree, is the strongest evidence available here.
