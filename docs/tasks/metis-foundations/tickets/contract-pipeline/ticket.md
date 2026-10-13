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
