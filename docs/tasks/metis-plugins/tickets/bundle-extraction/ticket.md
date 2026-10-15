---
type: ticket
subtype: feature
title: Extract a verified bundle under the archive rules
status: stable
workflow_state: ready
blocked_by: [bundle-verification]
size: m
---

## What to build

A verified bundle is staged into a directory, or refused. Refused: an absolute
entry name, an entry name carrying `..`, a symlink or hardlink entry, entry
permissions, an entry over the size limit, and an archive over the entry count or
uncompressed size limits. A refused extraction leaves nothing staged. The staged
directory is what the manifest and library steps read next, so extraction never
reads anything the verification step did not accept.

## Acceptance criteria

- [ ] Each refusal answers its own typed error and leaves no staged file behind.
- [ ] A bundle holding several targets extracts only the files this host's load
      needs.
- [ ] A failed extraction removes its staging directory, and a successful one
      owns it until the load finishes with it.
- [ ] The limits on entry count, uncompressed size and entry size are enforced and
      recorded in the contract.
- [ ] Extraction is tested against a hostile archive built in the test, not only
      against a well-formed bundle.

## Blocked by

- `bundle-verification`

## Implementation notes

Landed on `task/bundle-extraction` as the merge commit `d318f65`
("ticket(bundle-extraction): Extract a verified bundle under the archive
rules"), parents `bfb27bf` (the landing branch tip, `ticket: finalize
bundle-verification`) and `475f1c0` (the working branch, `chore(ticket): keep
the orchestration pointer out of this ticket's commits`). The merge was clean
with no conflicts, and `git diff 475f1c0 HEAD` is empty, so the landed tree is
byte-identical to the verified tip. The branch `ticket/bundle-extraction` was
deleted after the merge, and no source, test or config file was changed by the
landing step.

The verifier ran against `475f1c0`: `cargo fmt --check` and `cargo clippy
--all-targets -- -D warnings` clean, `cargo test -p plugin-runtime --test
bundle_extraction` 28 passed and 0 failed, the full `cargo test` 290 passed, 0
failed and 8 ignored over 48 test targets, and no staging directory left in
`/tmp` after the runs. All five acceptance criteria were checked against the
source, not only against the run log: eleven refusal tests assert their exact
`StageError` variant and an unchanged staging set, `classify` stages only
`manifest.toml` and `lib/<target>/<file>`, both `Drop` and the refusal path
remove the staging directory, the three limits are enforced at their edges and
recorded in contract 17.8, and every archive is built inside the test.

The first chain for this ticket timed out on its 30 minute budget after the
verify step and before the deviation step, so the deviation report was written
by the re-dispatched chain over the same, unchanged tip. It lives at
`docs/tasks/metis-plugins/tickets/bundle-extraction/deviation-reports/bundle-extraction.md`
and records ten divergences from the arch spec. The consequential ones: the
limits are 256 entries, 64 MiB per entry and 256 MiB in total, four times the
numbers the approved spec named; the public surface is wider than planned
(`STAGING_PREFIX` and the three limit constants are exported); `StageError`
carries 15 variants, more than the ticket's refusal list, so a dependent must
classify it rather than match it exhaustively; a target directory holds exactly
one file, a rule this ticket added; and the declared-mode rule is strict, a
declared mode must be `0600`, so the `zip` writer's `0o644` default is refused
and `sample-bundle-ci` must call `unix_permissions(0o600)`.

Residual risks carried into the effort: `StageError::Io` has no test; the size
limits are enforced on declared entry sizes rather than on decompressed output;
the staging-directory count assertion is process-global and holds only because
every staging test in that binary takes one file-wide mutex; and a missing root
manifest is deliberately not refused here, because `manifest-and-library` owns
that refusal.
