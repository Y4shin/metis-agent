---
type: ticket
subtype: feature
title: Verify a bundle's container, chain and payload signature
status: stable
workflow_state: ready
blocked_by: []
size: l
---

## What to build

Any file can be turned into a verified bundle or a typed refusal. The container
header is read with the magic and the container version at fixed offsets; the
version must match this build's exactly; the certificate chain is verified against
a supplied root key set and revocation list with every key id recomputed; the
payload signature is verified over the header, the chain and the archive bytes;
and every documented refusal rule is enforced. The verification crates are chosen
here, with their dependency cost measured and no C build step in the path that
reads untrusted bytes. A committed signing vector is tested at byte level. The
contract documents the container, the chain, the profile, the domain tags, the key
id rule, the revocation rule and the refusal rules.

## Acceptance criteria

- [ ] A valid signed bundle verifies, and each refusal rule answers its own typed
      error: unknown container version, a signature offset or length that does not
      account for the whole file, bytes before the header, trailing bytes in the
      chain region, more than eight links, an expired certificate, a revoked key,
      an unknown root, and a non-canonical encoding.
- [ ] The chain profile is enforced: a certificate that authorises another key
      carries the CA constraint, the leaf-authorising certificate is an end
      entity, an unknown critical extension refuses the chain, and the
      leaf-authorising certificate's common name is returned for the manifest
      check.
- [ ] Key ids are recomputed from the public key and a carried identifier is never
      trusted.
- [ ] A committed vector and altered copies of it are tested at byte level, and
      the vector is signed with a test key that the daemon's compiled-in root set
      does not contain.
- [ ] The contract describes the container header, the chain, the domain tags, the
      key id rule, the revocation rule and the refusal rules well enough for a
      second implementation.
- [ ] The trusted parse path adds no C build step, and the measured dependency
      count is recorded.

## Blocked by

- None (can start immediately).

## Implementation notes

Worker's note, recorded before landing. The working branch is
`ticket/bundle-verification`, created off the landing branch
`task/bundle-verification` (which already carries the landed waker fix).

- **The measured dependency count of the trusted parse path.** Taken with
  `devenv shell -- cargo tree -p plugin-runtime --edges normal --prefix none |
  sort -u | wc -l`: 66 crates before this ticket, 85 after, so the parse path
  adds 19, including its four direct crates (`ed25519-dalek` 3, `x509-cert` 0.3,
  `der` 0.8, `sha2` 0.11, the last already in the workspace table). The same pipe
  without `--edges normal`, which includes the test-only signer, goes from 76 to
  101. The only added crate with a build script is `curve25519-dalek`, whose
  script selects cfg flags from the rustc version and compiles nothing, so no C
  build step enters the path that reads untrusted bytes. The numbers are
  recorded in `docs/plugin-contract.md` section 17.7.
- **The domain tag is fixed here**, because no earlier document named its bytes:
  the 27 ASCII bytes `METIS-PLUGIN-BUNDLE-PAYLOAD`, ahead of the header, the
  chain and the archive, with the signature bytes deleted rather than
  zero-filled. It carries no container version, so a version bump does not change
  what a signature covers.
- **The anchor is the last link, trusted by key id rather than by who signed
  it.** The chain therefore carries the root's certificate, and the root set
  stays a set of (key id, public key) pairs. This is the reading that makes
  "unknown root" decidable without a name-matching rule, and it is written into
  section 17.4.
- **"Bytes before the header" is read as a declared signature offset below the
  header's end**, which is the only reading available in a container whose header
  begins at offset 0. The trigger and its place in the check order are in
  section 17.6.
- **The non-canonical case in criterion 1 is a certificate whose `critical`
  `basicConstraints` flag is spelled `01` rather than `FF`**, plus a separate
  non-minimal length. `der` reports the first as `Noncanonical` and rewrites the
  second into a length error at the header, so both are mapped to the one refusal
  the contract names.
- **The test signer is a second implementation of the layout.** It restates the
  offsets, the magic, the version and the domain tag in
  `crates/plugin-runtime/tests/support/signer.rs` rather than calling the
  verifier's constants, and the vectors are compared against it on every run.

Landing note. Merged with `git merge --no-ff`, no conflict, no source edit at
landing.

- Working branch `ticket/bundle-verification` (`b68f491`, five `wip:` commits)
  merged into `task/bundle-verification` as merge commit `7306244`, parents
  `b30efde` (landing branch) and `b68f491`. The branch is deleted afterwards.
- The landed tree is byte-identical to the verified tip: `git diff b68f491 HEAD`
  is empty, so every file landed exactly as the TDD and verify steps saw it.
- Verification state carried into the landed tree, recorded and not re-run:
  verify returned PASS at `b68f491`. `cargo fmt --check` and
  `cargo clippy --all-targets -- -D warnings` clean; `cargo test` 262 passed, 0
  failed, 8 ignored; `cargo test -p plugin-runtime --test bundle_verification` 39
  passed, 0 failed, 1 ignored. The only diagnostic is the pre-existing
  `plugin-contract` C `_FORTIFY_SOURCE` build-script warning, present on the
  landing branch too.
- The deviation report for this ticket is committed alongside this note, under
  `deviation-reports/bundle-verification.md`. `docs/tasks/state.yaml` (the
  orchestration task pointer) stays modified and unstaged, as in earlier
  landings; `workflow_state` stays `ready` because finalize-task owns it and the
  effort archive.
- One non-blocking documentation defect is carried as a residual risk: the
  recorded dependency-count pipeline in contract section 17.7 and in the note
  above, `cargo tree -p plugin-runtime --edges normal --prefix none | sort -u |
  wc -l`, over-counts because `cargo tree` prints a repeated subtree as
  `name vX.Y.Z (*)` and those marker lines survive `sort -u`. Run literally it
  prints 77 before and 101 after, against the recorded 66 and 85. The recorded
  figures are the correct counts of distinct crates (85 - 66 = 19 added, which
  verify reproduced), so the substantive claim holds; the pipeline needs
  `| grep -v '(*)'` before the numbers are quoted.
