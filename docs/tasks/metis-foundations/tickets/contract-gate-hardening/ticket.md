---
type: ticket
subtype: feature
title: Contract gates read one configuration and authenticate to the BSR
status: stable
workflow_state: ready
blocked_by: [contract-pipeline]
size: s
---

## What to build

Hardening the contract gates that `contract-pipeline` landed. Today `buf lint`
runs from `contracts/` and reads that directory's configuration, while
`buf breaking` and `buf generate` run from the repository root against buf's
built-in defaults, so an edit to the declared breaking policy never reaches the
gate. Both BSR-backed gates also resolve the pinned plugins anonymously, so
anonymous rate limiting can fail a CI run for a non-code reason.

One configuration at the repository root that every buf command reads, and a
`BUF_TOKEN` on the CI steps that need the network, fix both.

## Acceptance criteria

- [ ] The buf configuration lives at the repository root and declares the module
      at `contracts/proto` with `lint` STANDARD and `breaking` FILE, and
      `contracts/buf.yaml` is gone.
- [ ] Generation runs from the repository root through a root `buf.gen.yaml`
      that pins the same two plugin versions and writes the same committed
      trees, and `bash scripts/regenerate-contracts.sh` still reproduces them
      byte for byte with `--check` exiting 0 on a clean tree.
- [ ] `buf lint` and `buf breaking --against '.git#branch=main'`, both run from
      the repository root, read the root configuration. Proven by a deliberate
      edit to the root `breaking` block that changes the gate's verdict.
- [ ] The CI job supplies `BUF_TOKEN` from `secrets.BUF_TOKEN` to the steps that
      resolve plugins, and an absent or empty secret leaves both gates working
      without authentication (measure which form buf needs, since an empty
      variable may not be the same as an unset one).
- [ ] `contracts/README.md` and `docs/testing.md` describe the new locations of
      the commands, and the documented developer command is unchanged or
      shorter.
- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
      `cargo test`, and the two plugin ABI gates still pass.

## Blocked by

- `contract-pipeline` (done): this ticket edits the pipeline that ticket landed.
