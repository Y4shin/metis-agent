---
type: deviation report
title: Deviation report for bundle-extraction
status: stable
---

## Deviation report: bundle-extraction

Compared: the arch spec's `bundle-extraction` section, the ticket doc and its
five acceptance criteria, and the implementation on `ticket/bundle-extraction` at
`475f1c0` against the landing branch `task/bundle-extraction` at `bfb27bf` (eight
files, 1546 insertions, two deletions).

Provenance, because it changes how this report should be read. The first chain
for this ticket timed out on its 30 minute budget after the gate, the tdd step
and the verify step, and before this step ran, so the deviation report was never
written. This report is written by the re-dispatched chain over the same,
unchanged tip, and it also reads the first attempt's tdd result. The
implementation was authored in the first attempt; nothing in it changed in the
retry.

### API surface changes

**1. The extraction limits are four times the numbers the approved arch spec
named.**
- **Planned:** 64 entries, 32 MiB per entry, 64 MiB of uncompressed bytes in
  total, recorded in the contract and in the "Limits" sweep the arch spec
  proposed and the user approved.
- **Actual:** `MAX_ENTRIES` 256, `MAX_ENTRY_SIZE` 64 MiB and
  `MAX_UNCOMPRESSED_SIZE` 256 MiB, exported from `plugin_runtime::bundle` and
  recorded in `docs/plugin-contract.md` 17.8.
- **Impact:** every producer has to respect 256, 64 MiB and 256 MiB, so
  `sample-bundle-ci` and the SDK signing CLI adopt the new numbers, and the arch
  spec's approved text now disagrees with both the code and the contract. No
  dependent breaks, because nothing is built against the limits yet.

**2. The host target is a plain string, and nothing exports its derivation.**
- **Planned:** `extract(verified, target)`, with a target derived from
  `std::env::consts::OS` and `std::env::consts::ARCH` joined by a hyphen.
- **Actual:** `extract(&VerifiedBundle<'_>, target: &str) -> Result<StagedBundle,
  StageError>` as planned, with the spelling pinned in contract 17.8, and no
  `host_target()` helper anywhere in `plugin-runtime`. The tests pass explicit
  target strings, which is what the plan asked for in the seam.
- **Impact:** `async-initialisation` owns the derivation, and the loader that
  selects a library and the producer that writes a bundle must spell the target
  the same way. Two derivations that disagree would refuse a bundle that carries
  the library the host needs, so it belongs in one place.

**3. The public surface is wider than the plan, additively.**
- **Planned:** `bundle::extract`, `StagedBundle`, `StageError`, `root()`,
  `manifest_path()`, `library_path()` and `Drop`. The arch spec named the error
  families, not the variant set.
- **Actual:** all planned items exist with the planned names and shapes, plus
  `STAGING_PREFIX` and the three limit constants, and `StageError` carries 15
  variants: the seven ticket-list refusals plus `ArchiveUnreadable`,
  `EntryUnreadable`, `UnusableEntryName`, `EntryIsNotRegularFile`,
  `AbsoluteEntryName`, `AmbiguousTargetLibrary`, `MissingTargetLibrary` and `Io`.
- **Impact:** additive only, so no dependent is broken. A dependent must
  classify `StageError` rather than match it exhaustively, exactly as the
  verifier ticket's report already says for `BundleError`'s 26 variants.

**4. Two refusals land a ticket earlier than the interface contract placed
them.**
- **Planned:** "a missing target library" and "no library for this target" are
  `manifest-and-library`'s refusals.
- **Actual:** extraction decides both, because `StagedBundle::library_path()`
  returns one path and only the archive can say whether one file is there. They
  are `MissingTargetLibrary` and `AmbiguousTargetLibrary`, contract 17.8 rule 8.
- **Impact:** `manifest-and-library` keeps its own refusal for a manifest that
  declares no library for this host, so the two remain distinguishable and
  `startup-policy` must map both to its disable reason rather than collapse them.

**5. `manifest_path()` can name a file that is not there.**
- **Planned:** the arch spec listed the path in the interface contract without
  saying whether extraction refuses an absent root manifest; the ticket's refusal
  list does not include it either.
- **Actual:** a bundle with no root manifest still stages, and the refusal is
  deliberately left to `manifest-and-library`. Pinned by
  `a_bundle_without_a_root_manifest_still_stages` and stated in contract 17.8.
- **Impact:** the manifest step must test existence rather than assume
  `manifest_path()` resolves, and its "absent manifest" typed error stays
  reachable.

**6. One file per target directory is a rule this ticket added.**
- **Planned:** not required by the arch spec or the ticket.
- **Actual:** contract 17.8 rule 8 refuses an archive that holds two files
  directly under `lib/<target>/`.
- **Impact:** `sample-bundle-ci`'s packaging step and the SDK CLI write exactly
  one library per target. It also narrows the map's still-open fog about whether
  a v1 bundle may carry assets, which a later ticket can relax.

**7. Staging has a recorded location, mode and name.**
- **Planned:** a temporary directory the `StagedBundle` owns and removes when it
  is dropped; the plan did not fix where it is or what it is called.
- **Actual:** `std::env::temp_dir()/metis-plugin-stage-<pid>-<nanos>-<counter>`,
  directory `0700`, staged file `0600`, removed by a refusal before it is
  answered and by `Drop` on the success path. One small inaccuracy: contract
  17.8 describes the name as the process id and a counter and does not mention
  the timestamp component.
- **Impact:** a crash sweep that globs the exported `STAGING_PREFIX` is
  unaffected, and no dependent reads the name. A second implementation that
  reproduced the name exactly would not match until the contract names all three
  components.

**8. The refusal set is wider than the ticket's list.**
- **Planned (ticket):** an absolute entry name, a name carrying `..`, a symlink
  or hardlink entry, entry permissions, an entry over the size limit, and an
  archive over the entry count or uncompressed size limits.
- **Actual:** also refuses a directory entry, a FIFO, a device and a socket, a
  name that is empty after its separators are removed or carries a NUL byte, a
  non-ZIP archive, and staged entries whose bytes or checksum do not read.
- **Impact:** none negative, and it matches the arch spec's "refuse a symlink,
  hardlink or directory entry". A producer that writes directory entries, which
  much ZIP tooling does, is refused, so the packaging step must not write them.

### Abstraction usage

- Used/was specified: **yes**. `zip` 9 is in the workspace dependency table with
  `default-features = false` and only `deflate-flate2-zlib-rs`, referenced as
  `zip.workspace = true` with the reason in a comment; `std::fs` writes the
  host's own modes rather than the archive's; `StagedBundle`'s `Drop` owns the
  cleanup rule; `StageError` follows the crate's existing error style; and
  `docs/testing.md` gained the command bullet the protocol rule requires for a
  new test file. I re-measured the dependency claim on the branch: `cargo tree -p
  plugin-runtime --edges normal --prefix none | grep -v '(*)' | sort -u | wc -l`
  prints 91, against the 85 that contract 17.7 records for the verifier ticket,
  so six crates were added and none of them builds C (`zlib-rs` 0.6.8 and
  `flate2` have no build script, and `crc32fast`'s script only prints cfg flags).
- The do-not list held in full: no hand-rolled ZIP reader, no `bzip2`, `zstd`,
  `lzma`, `ppmd` or AES feature, no signature re-verification, no manifest
  parsing, and no ABI change. The one difference from the plan's letter is the
  backend's name: the plan said "pure-Rust deflate" and the implementation
  reaches it through `flate2`'s `zlib-rs` backend, which is the same guarantee.
- The `bundle/` module split is the choice the arch spec left to this ticket:
  `bundle.rs` stays the verifier and `bundle/extract.rs` is reached through
  re-exports, so `bundle::extract` is a function and no file was renamed.

### Out-of-scope changes

- None in product code. The 13 lines added to `bundle.rs` are the module doc, the
  private `mod extract;` and the re-exports; no verifier behaviour changed, which
  the 39-test verifier suite and the 73-check ABI harness confirm by still
  passing.
- Documentation: `docs/plugin-contract.md` gains section 17.8 (85 lines) and
  `docs/testing.md` gains one command bullet. Both are inside the plan's doc
  duties for this ticket.
- History: the branch's net diff does not touch `docs/tasks/state.yaml`. A commit
  in the first attempt accidentally carried the orchestration pointer, and
  `475f1c0` reverts it, so the branch leaves that file alone.
- Dependencies: `zip` is the only added direct dependency, and its transitive set
  is the six crates measured above.

### Ticket doc update needed?

Yes, and this one is substantive. The ticket doc still carries no
`## Implementation notes` section and no criterion evidence, because the first
chain's tdd step wrote its result to the artifact reference and the
re-dispatched tdd step made no doc change. The landing step should create the
section and append:

- The criterion-to-test map: eleven refusal tests through one helper that asserts
  both the exact `StageError` variant and an unchanged staging-directory set;
  target selection over three targets; staging ownership and cleanup on both the
  refusal and the success path; the three limits at their edges; and hostile
  archives built, signed and byte-patched inside the test.
- The eight divergences above, in the landing note's own words.
- The residual risks: `StageError::Io` has no test because provoking a real
  filesystem failure needs mocking; the cleanup assertion cannot watch the
  staging directory during a refusal; the strict declared-mode rule constrains
  packaging; two error-mapping branches are unreachable in practice; and the
  process-global staging-count assertion holds only because every staging test in
  the binary takes one file-wide mutex.
- Two process facts: criterion 2's tests were green on arrival from the tracer
  bullet and are regression guards rather than RED-first cycles, and criterion
  1's suite was flaky on its first run until every staging test took that lock.

### User attention needed?

Yes, one item. The extraction limits are four times the numbers the approved arch
spec named: 256 entries, 64 MiB per entry and 256 MiB total, against 64, 32 MiB
and 64 MiB. They are recorded in contract 17.8 and no dependent breaks, but the
arch spec's approved text now disagrees with the code and with the contract, and
the spec-reconciler pass should correct it before `sample-bundle-ci` and the SDK
CLI adopt the numbers. One packaging consequence is worth carrying forward with
it: contract 17.8 rule 5 requires a producer to declare mode `0600`, because the
`zip` writer writes `0o644` when a producer chooses nothing, so
`sample-bundle-ci` has to set `unix_permissions(0o600)` on every entry.
