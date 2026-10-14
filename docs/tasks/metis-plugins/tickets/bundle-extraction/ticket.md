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
