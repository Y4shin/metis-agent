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

## Implementation notes

Landed on `task/contract-gate-hardening` as the merge commit `3612e0f`
("ticket(contract-gate-hardening): Contract gates read one configuration and
authenticate to the BSR"), parents `08a5be8` (the landing branch, which already
carried the first attempt's two checkpoints behind the preservation merge) and
`81c6207` (the working branch `ticket/contract-gate-hardening`). The merge was
clean with no conflicts, the landing tree is byte-identical to `81c6207`
(`git diff 81c6207 HEAD` is empty), and `ticket/contract-gate-hardening` was
deleted after the merge. The review artifact
`deviation-reports/contract-gate-hardening.md` is committed with this note.

1. The buf configuration now lives at the repository root: `buf.yaml` declares
   `modules: [{path: contracts/proto}]` with `lint` STANDARD and `breaking`
   FILE, `contracts/buf.yaml` is gone, and root `buf.gen.yaml` carries the same
   two BSR pins (`buffa:v0.9.2`, `connectrpc/rust:v0.9.0`), the same options and
   the same committed output trees, so `buf lint`, `buf breaking` and
   `bash scripts/regenerate-contracts.sh` all run from the repository root.
2. The breaking gate requires `--against-config buf.yaml`; without it the same
   command from the repository root exits 100 with spurious
   `Previously present file "contracts/proto/metis/v1/daemon.proto" was deleted`
   reports, because the working side resolves the module at `contracts/proto`
   while the branch side resolves whole-tree paths. The arch spec's command
   line, `buf breaking --against '.git#branch=main'`, is therefore incomplete,
   and a later contract ticket that copies it will see those spurious failures.
   CI, `contracts/README.md` and `docs/testing.md` all carry the flag.
3. Measured on buf 1.73.0 for acceptance criterion 4: both an unset and an empty
   `BUF_TOKEN` reach the BSR anonymously, while a non-empty token the BSR cannot
   verify is refused before any request is sent. The CI step unsets an empty
   value regardless, so the gate does not depend on that buf-version behaviour
   rather than on documentation. `buf lint` and `buf breaking` resolve no
   plugins and pass even under a token buf refuses, so only the
   `Contract regeneration check` step carries `BUF_TOKEN`.
4. The ticket's acceptance test now lives at
   `scripts/probe-contract-gates.sh`, one section per arch-spec seam, restoring
   every file it touches through a `trap` and reporting a rate-limited probe as
   skipped rather than as a pass. It was written inside the ticket's transient
   `.work/` channel; the effort owner moved it to the scripts directory at
   finalization so the evidence survives without putting scaffolding in the
   ticket's uncertainty channel. It is not wired into CI, and `docs/testing.md`
   documents it as an on-demand probe.
5. `crates/api-contracts/src/lib.rs` had a doc comment pointing generated code
   at `contracts/buf.gen.yaml`, a location this ticket deletes; it now names the
   root `buf.gen.yaml` (comment only, no API change).
6. Residual risks: the negative real-generation drift probe is skipped under BSR
   anonymous rate limiting, so only the offline stub-backed twin and the
   clean-tree passes exercise the drift logic; GitHub Actions cannot be executed
   here, so the CI wiring is asserted by text probes plus the measured buf
   behaviour. The landing tree is byte-identical to the tree the verify worker
   tested, so no gate was re-run at landing.
7. Open item: resolved at finalization. The review artifact and the land note
   both flagged that the merge carried `.work/probe.sh` onto the landing branch
   and that the root `/.work/` gitignore pattern does not cover a nested
   `.work/`. The probe has a durable home at `scripts/probe-contract-gates.sh`
   with a line in `docs/testing.md`, the `.work/` directory is gone, and the
   committed path was re-run green from its new location (2 network-only probes
   skipped under anonymous BSR rate limiting).
