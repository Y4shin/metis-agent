---
type: deviation report
title: Deviation report for contract-gate-hardening
status: stable
---

## Deviation report: contract-gate-hardening

Reviewed: `main..ticket/contract-gate-hardening` (10 files, +364/-60). That range
is the whole ticket, because the landing branch already carried the first
attempt's two checkpoints (`19362b6`, `b034c2d`) plus the preservation merge
`08a5be8`, and the retry added three commits on top.

### API surface changes

**Planned:** one root `buf.yaml` declaring the module at `contracts/proto` with
`lint` STANDARD and `breaking` FILE, `contracts/buf.yaml` deleted; a root
`buf.gen.yaml` carrying the same `version: v2`, `clean: true`, plugin pins,
options and output trees; `buf lint`, `buf breaking`, and generation all run
from the repository root; CI supplies `BUF_TOKEN` from `secrets.BUF_TOKEN` to
the plugin-resolving steps; `contracts/README.md` and `docs/testing.md` updated;
the regeneration script keeps its behaviour including `normalize_comments=true`.

**Actual:** all of that holds. On the tree: root `buf.yaml` with
`modules: [{path: contracts/proto}]`, `lint.use: [STANDARD]`, `breaking.use:
[FILE]`; `contracts/buf.yaml` gone; root `buf.gen.yaml` with the same two BSR
pins (`buffa:v0.9.2`, `connectrpc/rust:v0.9.0`), the same options, and the same
committed output trees; `scripts/regenerate-contracts.sh` changed only to `cd`
to the repository root and to rewrite the scratch template path, keeping
`normalize_comments=true`.

Three additions beyond the planned surface:

1. **The breaking gate gained a required flag.** The spec names
   `buf breaking --against '.git#branch=main'`; the implementation runs
   `buf breaking --against '.git#branch=main' --against-config buf.yaml`. I
   reproduced why it is necessary: from the repository root, without the flag
   the command exits 100 with spurious
   `Previously present file "contracts/proto/metis/v1/daemon.proto" was deleted`
   reports, because the working side resolves the module at `contracts/proto`
   while the branch side resolves whole-tree paths. With the flag it exits 0.
2. **The CI step gained a two-line guard** beyond passing the secret:
   `if [ -z "${BUF_TOKEN:-}" ]; then unset BUF_TOKEN; fi`, so buf is never handed
   a present-but-empty variable.
3. **A doc comment outside the two named docs:**
   `crates/api-contracts/src/lib.rs` pointed generated code at
   `contracts/buf.gen.yaml`, which this ticket deletes; it now names the root
   `buf.gen.yaml`. Comment only, no API change.

**Impact:** the arch spec's `buf breaking` command line is now incomplete, so a
later contract ticket that copies it will see spurious failures on any branch
whose comparison side lacks the root configuration. CI, `contracts/README.md`,
and `docs/testing.md` all carry the flag, so the exposure is a ticket doc or a
worker reading the spec rather than the tree. Nothing about what is generated,
the lint policy, or the breaking category changed, so no dependent sees a
different verdict for an unchanged contract.

### Abstraction usage

- Used/was specified: yes. This moved configuration rather than rewriting the
  pipeline, as the spec's "Do NOT reimplement" list requires: the two plugin
  pins, their options, the output tree layout, `clean: true`, and the rustfmt
  post-step are unchanged, and no second generation path was added.
  `contracts/README.md` owns the developer-facing command and `docs/testing.md`
  owns the run commands; both were updated in place.
- Concrete evidence that the root config is the one buf reads: `buf ls-files`
  from the repository root lists exactly
  `contracts/proto/metis/v1/daemon.proto`, so the module comes from the root
  `buf.yaml` and lint covers the contract alone.

### Out-of-scope changes

1. `.work/probe.sh`, 268 lines, is committed inside
   `docs/tasks/metis-foundations/tickets/contract-gate-hardening/.work/`. That
   directory is the transient uncertainty channel, the land-worker's
   `git add docs/tasks/` will carry it onto `main`, and the root `/.work/`
   gitignore pattern does not cover a nested `.work/`. The script is real
   acceptance evidence (it restores every file it touches through a `trap`, and
   reports a rate-limited probe as skipped rather than as a pass), but it is
   scaffolding rather than product.
2. `docs/tasks/state.yaml` (`task: contract-gate-hardening`) sits in the diff
   because the orchestrator merged the first attempt's checkpoints into the
   landing branch and committed the state pointer. Process artifact, not product.
3. The CI token justification was rewritten rather than only added. The
   checkpoint-era text asserted buf treats an empty `BUF_TOKEN` "exactly like an
   unset one (measured)"; that sentence is now removed from both `.github/`
   and `contracts/README.md` and replaced with the measurement below. The net
   effect is a correction, flagged because the same claim had already reached a
   committed doc.

### Ticket doc update needed?

**Yes.** Append to `## Implementation notes`:

1. The breaking gate requires `--against-config buf.yaml`; without it the same
   command exits 100 with spurious deleted-file reports when run from the
   repository root (reproduced on the ticket tree).
2. Measured on buf 1.73.0: both an unset and an empty `BUF_TOKEN` reach the BSR
   anonymously, while a non-empty token the BSR cannot verify is refused before
   any request is sent. I reproduced the invalid-token half directly
   (`Failure: the BUF_TOKEN environment variable is not valid for buf.build`).
   The empty half is the worker's measurement and not mine, because the
   anonymous BSR limit was exhausted during this review.
3. `buf lint` and `buf breaking` resolve no plugins and pass even under a token
   buf refuses, so only the regeneration step carries the token.
4. The ticket's acceptance test is `.work/probe.sh`, one section per arch-spec
   seam, restoring every file it touches.
5. `crates/api-contracts/src/lib.rs` had its generated-code doc comment updated
   because it named the deleted `contracts/buf.gen.yaml`.

### User attention needed?

**Yes**, one item, and it does not block the acceptance criteria:

- Decide whether `.work/probe.sh` should land on `main`. If it should, it needs
  a durable home (for example beside the other gate scripts under `scripts/`)
  and a line in `docs/testing.md`; if it should not, it has to be removed before
  the land commit, since the land-worker's `git add docs/tasks/` will otherwise
  commit it.

### Verification evidence

Independently run during this review:

- `buf ls-files` from the repository root -> `contracts/proto/metis/v1/daemon.proto`,
  exit 0.
- `buf lint` from the repository root -> exit 0.
- `buf breaking --against '.git#branch=main' --against-config buf.yaml` from the
  repository root -> exit 0. The same command without `--against-config` -> exit
  100 with the spurious deleted-file reports quoted above.
- `devenv shell -- env BUF_TOKEN=not-a-real-token buf registry plugin info
  buf.build/anthropics/buffa` -> exit 1 with the invalid-token message and no
  request sent.
- `bash scripts/regenerate-contracts.sh --check` -> **not reproduced**: the
  anonymous BSR limit is exhausted right now
  (`Failure: resource_exhausted: too many requests`). The worker recorded this
  passing on this layout twice, and the probe's offline half covers the diff
  logic with a stubbed generator.
- The deliberate-edit verdict change for seams 1 and 2 -> **not re-run by me**:
  it mutates tracked files, and `.work/probe.sh` is the artifact that performs it
  with restoration. I read that section and the restoration `trap`.
- Read for the review: `buf.yaml`, `buf.gen.yaml`, `.github/workflows/ci.yml`,
  `contracts/README.md`, `docs/testing.md`, `scripts/regenerate-contracts.sh`,
  `crates/api-contracts/src/lib.rs`, `.work/probe.sh`, and the TDD result.
