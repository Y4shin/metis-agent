---
type: ticket
subtype: feature
title: Contract pipeline with one RPC
status: stable
workflow_state: ready
blocked_by: []
size: m
---

## What to build

The contract pipeline, end to end. A hand-written Protobuf document describes the
daemon service under the versioned package, one ordinary echo RPC for now,
generated with pinned plugins into committed Rust, with the regeneration command
and the review gates wired into CI. A maintainer edits the document, regenerates,
and both the build and the gates reflect the change.

## Acceptance criteria

- [ ] The document declares the versioned package and one echo RPC with request
      and response messages, and `buf lint` passes in STANDARD mode with only
      justified exceptions.
- [ ] Generation runs through pinned plugins resolved as remote BSR plugins, and
      the generated message, client, and server stub code is committed.
- [ ] One documented regeneration command reproduces the committed output byte
      for byte, and the CI regenerate-and-diff check fails when it does not.
- [ ] `buf breaking` against main runs in CI and fails on a breaking change to
      the declared contract.
- [ ] The workspace builds, and the generated code passes formatting and clippy
      with warnings denied, using the documented formatting step and allowances
      rather than excluding the generated trees.
- [ ] No JVM or other extra runtime is needed for the everyday build or test.

## Blocked by

- None (can start immediately).

## Implementation notes

Landed on `task/contract-pipeline` as the merge commit `adea161`
("ticket(contract-pipeline): Contract pipeline with one RPC"), parents `5b39655`
(`main`) and `26fcb48` (working branch `ticket/contract-pipeline`). The landing
branch was created off `main` at landing time (it did not exist while the
working branch was built). The merge was clean with no conflicts, the landing
tree is byte-identical to `26fcb48`, and `ticket/contract-pipeline` was deleted
after the merge.

At landing, `cargo test` (whole workspace) and `cargo test -p api-contracts`
(3 contract tests pass, 6 generated doctests ignored) both exit 0 through
`devenv shell --`. The full acceptance probe record lives in the verify report;
the deviations below were measured by the implementing and verifying workers and
are carried forward for the later contract tickets.

1. The rustfmt post-step applies `normalize_comments=true` on top of the
   repository config. Plain rustfmt re-indents buffa's multi-line `/** */` doc
   comments, rustdoc then reads that prose as an indented Markdown code block
   and collects doctests that cannot compile, which fails `cargo test`. With the
   option, `rustfmt --check` still exits 0 on both committed trees, and the
   trees are not excluded from fmt, clippy, or test.
2. `buf breaking` runs from the repository root against buf's built-in default
   configuration, so `contracts/buf.yaml`'s `breaking` block does not govern the
   CI gate. From `contracts/` the `.git#branch=main` target cannot resolve
   (`contracts/.git` does not exist). `buf lint` does run from `contracts/` as
   planned. Follow-up decision for a later contract ticket: move the buf
   configuration to a root `buf.yaml`, or run the breaking gate from
   `contracts/` against an explicit repository path.
3. Verify finding on acceptance criterion 4: while `main` carries no
   `contracts/proto/metis/v1/daemon.proto`, every change to the declared contract
   is an addition on the comparison's input side, so the CI command cannot fail
   on a breaking change to the declared contract from the working branch
   (measured: the CI command exits 0 with a breaking field rename in the working
   tree, while `--against '.git#branch=ticket/contract-pipeline'` exits 100).
   The gate arms itself on this merge, because `main` then holds the contract,
   so the criterion holds on the landing branch going forward and not
   retroactively on the working branch.
4. The regeneration and drift check resolve both plugins from the BSR on every
   run; anonymous rate limiting (`resource_exhausted: too many requests`) can
   fail the check for a non-code reason, and no `BUF_TOKEN` is pinned anywhere in
   the repository. A token in CI, or vendored pinned plugins, would remove that
   dependency.
5. The `clippy::derivable_impls` mount allow is dormant until the contract
   declares its first enum (`event-stream` adds `EventKind`); only
   `clippy::match_single_binding` is load-bearing today (removing it fails clippy
   with three errors).
6. `docs/dev-env.md` and `docs/testing.md` were updated alongside the ticket, as
   recorded in the review artifact `deviation-reports/contract-pipeline.md`
   (committed with this note).
