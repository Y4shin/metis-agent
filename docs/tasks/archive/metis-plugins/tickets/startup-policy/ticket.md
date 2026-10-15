---
type: ticket
subtype: feature
title: Start-up outcomes and the disabled placeholder
status: deprecated
workflow_state: done
blocked_by:
- async-initialisation
size: m
---

## What to build

The daemon's start-up outcomes for a directory of bundles. A trust failure, an
absent or invalid manifest and a refused unload stop start-up. An extraction
refusal, a missing target library, an ABI mismatch, a manifest that disagrees with
the library, and an unknown container version disable that one plugin and leave
the daemon running. A disabled plugin appears as a placeholder with its reason: it
is built from the manifest when one was readable, and from the bundle's file name
with no capabilities when it was not. It is listable, it is excluded from the
capabilities an agent would see, and invoking it answers the existing disabled
route error with its reason.

## Acceptance criteria

- [ ] Each start-up outcome is produced by a test that names it, including the
      unknown container version, which is the only refused bundle that degrades
      without ever being parsed.
- [ ] A disabled plugin is listable with its name, its reason and its capabilities
      when the manifest was readable, and with the file's name and no capabilities
      when it was not.
- [ ] Disabled capabilities are excluded from what an agent would be offered, and
      invoking one answers the disabled route error carrying the reason.
- [ ] A plugin whose initialisation fails is unloaded and replaced by its
      placeholder, and a refused unload at that point stops start-up.
- [ ] One broken bundle among several leaves the others loaded and serving.
- [ ] The start-up policy is documented as the rule, not only implemented.

## Blocked by

- `async-initialisation`

## Implementation notes

Landed on `task/startup-policy` as the merge commit `c13e314`
("ticket(startup-policy): Start-up outcomes and the disabled placeholder"),
parents `3159576` (the landing branch tip, the `callback-sink` landing) and
`636f874` (the working branch `ticket/startup-policy`, nine `wip:` commits from
`add6501` to `636f874`). The merge was clean with no conflicts and the landing
tree is byte-identical to the verified `636f874` (`git diff 636f874 HEAD` is
empty), so every source file landed exactly as verified. The branch
`ticket/startup-policy` was deleted after the merge.

The ticket diff over `3159576` is twelve files, 1966 insertions and 11
deletions: the new `crates/plugin-runtime/src/startup.rs` (374 lines,
`start_plugins`, `StartupReport`, `Serving`, `DisabledPlugin`, `StartupStop`),
the new `crates/plugin-runtime/tests/startup.rs` (1100 lines, 21 cases), the new
`crates/plugin-runtime/tests/fixtures/refusing_destroy.rs` (232 lines, the fault
cdylib compiled at test time), `crates/plugin-runtime/src/lib.rs` (+55, the
`startup` module and the widened `RouteError::Disabled { name, reason }`),
`crates/plugin-runtime/src/load.rs` (+44/-4, the tenth `LoadError` variant
`UnloadRefused { plugin, step, status }`, the `RefusedUnload` record and the
crate-private `host_target()`),
`crates/plugin-runtime/src/registry.rs` (+25/-4, the reason-carrying switch-off),
`crates/plugin-runtime/tests/support/mod.rs` (+66/-1, `compiled_fixture`),
`crates/plugin-runtime/tests/registry.rs` (one assertion adapted mechanically to
the widened `Disabled`, the only foreign test touched),
`docs/plugin-contract.md` (+38, section 19 "Start-up outcomes"),
`docs/testing.md` (+21, the new test file's command bullet), the recorded
resolution at `docs/tasks/metis-plugins/tickets/startup-policy/.work/resolution.md`
and `docs/tasks/state.yaml` (the branch's `task: startup-policy`, left for
finalize-task to own). This landing commit also adds the ticket doc's
implementation notes and the ticket's deviation report at
`docs/tasks/metis-plugins/tickets/startup-policy/deviation-reports/startup-policy.md`,
as every previous landing in this effort did.

Verification ran against `636f874`: `cargo fmt --check` clean, `cargo clippy
--all-targets -- -D warnings` clean (only the pre-existing `_FORTIFY_SOURCE`
note from `plugin-contract`'s C build script), `cargo test -p plugin-runtime
--test startup` 21 passed and 0 failed, and the full `cargo test` green with 354
passed, 0 failed and 8 pre-existing ignored across 54 binaries. The diff adds no
`#[ignore]`, so the ignored set is unchanged. All six acceptance criteria were
checked against the code and hold; criterion 6 is satisfied by prose (contract
section 19, the discovery rule, the three-outcome table and the three stated
rules), matching the landed precedent for doc criteria. One landed assertion in
`crates/plugin-runtime/tests/registry.rs`
was adapted for the widened `RouteError::Disabled`, which the resolution
explicitly allows.

Durable notes for the tickets that follow. `LoadError` now has ten variants, one
more than the arch spec's level-3 list, and `LoadError::Manifest` is classified
finer than that level-3 table: `NoTargetLibrary` and `LibraryNotStaged` disable,
while `Absent`, `Unreadable`, `Invalid` and `NameMismatch` stop. Contract section
19 states the landed rule, so the arch spec's reconcile is a documentation job
rather than a behaviour question. `start_plugins`'s configuration map and the
host context are keyed by the bundle file's stem, not by the manifest's name,
because both are built before the load reads the manifest; a caller keying by the
manifest's name needs files named after their plugins, and the arch spec should
state the key. The policy builds one host for the whole pass and passes it to
every load, so a directory holding bundles from different producers can reach the
one-host-one-library refusal, and the report keeps one registry per serving
plugin rather than aggregating them; both are recorded rather than fixed, and
aggregation belongs to `teardown-and-release`. The placeholder path reuses the
crate-private `host_target()` rather than adding a third `<os>-<arch>`
derivation.

Residual risks carried forward: `LoadError::Unreadable` for a bundle file and
`ManifestError::Unreadable` have no test of their own, because a test that may run
as root cannot portably make a regular file unreadable, so their branches are
covered only through the directory path; `LoadError::Registration` has no test and
the policy arm for it is one line; a stop mid-pass drops the `Serving`s that
already started without unloading them, because `Serving::release` belongs to
`teardown-and-release`, so a daemon that stops leaves instances whose libraries
stay mapped; the disabled placeholder's second read of its bundle repeats
`verify_bundle`, `extract` and the manifest read, which is the cost the resolution
accepted and never runs for a bundle that started; and `start_plugins`'s
configuration key remains a caller contract only until the arch spec names it.

### Knowledge harvest

The CI gate was re-run on the landing branch at `cbd8b6d`, after `main` was
merged in, and all eleven commands pass. The two unreachable criteria this
ticket stopped on are closed by the operator's recorded resolution, which is
committed at `.work/resolution.md`: criterion 4 by the tenth `LoadError` variant
and a fault library, criterion 2 by the placeholder path's second read. The
three minor decisions were taken as the resolution directs and are visible in
the landed code.

Durable lessons for `teardown-and-release` and `sample-bundle-ci`. A stop is a
`Result` rather than a report, so a caller must handle `StartupStop` and read the
`Serving` values the report carries; a stop mid-pass drops already-started
`Serving` values without unloading them, which is the release ticket's work. Each
`Serving` carries its own registry, so a daemon that wants one registry registers
each `Serving` into its own through `Registry::register`. A disabled plugin is
answered by the report rather than by the registry, and its capabilities never
appear in `offered_capabilities()`. The configuration map is keyed by the bundle
file's stem, which is a caller contract until a contract sentence names it.

One disagreement is left open rather than settled here. The resolution said the
host context would be built from the name the load reports, but the payload is
built before the manifest is read, so the landed policy builds it from the file
stem while the instance is registered under the manifest's name. The arch spec
records the landed rule and the disagreement; a bundle whose stem and manifest
name differ therefore tells its plugin the stem.
