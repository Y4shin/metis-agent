---
type: ticket
subtype: feature
title: Replace instance creation with asynchronous initialisation in the plugin table
status: deprecated
workflow_state: done
blocked_by:
- waker-lifecycle-fix
size: l
---

## What to build

The plugin table carries asynchronous initialisation instead of instance
creation, and no counters entry. Initialisation receives one payload carrying the
configuration and the host context, and returns the instance plus a future that
must resolve successfully for the instance to be valid. The ABI minor is bumped
because the table changed, and the contract text, the generated C header, the slot
constants, the layout definitions, the C static assertions, the Rust fixture, the
C plugin and the ABI harness all move in this ticket, so the C ABI cannot drift
from its description. The new entry is proven by the ABI harness driving the C
plugin as a real host.

## Acceptance criteria

- [ ] The initialisation entry replaces instance creation and the counters entry
      is gone, with the minor bumped and the entry symbol carrying the unchanged
      major.
- [ ] The contract text, the generated header, the slot constants, the layout
      definitions and the C static assertions agree with the table, and the
      header regeneration check passes.
- [ ] Both the Rust fixture and the C plugin implement the new entry, including
      the failure case where the configuration does not match what the plugin
      declared.
- [ ] The ABI harness hand-drives initialisation through the new entry with a
      configuration and host context payload, observes an instance and a resolved
      future, and prints one line per check.
- [ ] The version tolerance checks still pass: a newer minor is read and stops at
      this build's layout, an older minor is refused before any entry is read, and
      another major is refused at symbol resolution.
- [ ] No counter of any kind is reachable through the table.

## Blocked by

- `waker-lifecycle-fix`

## Implementation notes

Landed on `task/abi-initialisation-entry` as the merge commit `9c9fd2a`
("ticket(abi-initialisation-entry): Replace instance creation with asynchronous
initialisation in the plugin table"), parents `ae96284` (the landing branch tip,
`docs(ticket): land bundle-extraction`) and `c6666ff` (the working branch,
`wip: abi-initialisation-entry the format gate and the recorded harness revision
passing`). The merge was clean with no conflicts, and `git diff c6666ff HEAD` is
empty, so the landed tree is byte-identical to the verified tip. The branch
`ticket/abi-initialisation-entry` was deleted after the merge, and no source,
test or config file was changed by the landing step. `docs/tasks/state.yaml` is
left uncommitted, matching the `475f1c0` precedent, because `finalize-task` owns
the workflow state.

The implementation is 25 files, 1733 insertions and 351 deletions against
`ae96284`, over four `wip:` commits. The verifier ran against `c6666ff`: `cargo
fmt --check` and `cargo clippy --all-targets -- -D warnings` clean, `cargo test`
293 passed with 0 failed and 8 ignored, the ABI harness `[result] PASS` with 82
checks and no `[fail]` line, `scripts/regenerate-abi-header.sh --check` reporting
no drift, and `scripts/build-c-plugin.sh` accepting the committed header under
gcc and clang. The six acceptance criteria were read against the source, not
only against the run log: `METIS_ABI_MAJOR` is 1, `METIS_ABI_MINOR` is 2 and
`metis_plugin_entry_v1` is unchanged; `init_plugin` is slot 0 of the table and no
counter member, slot constant, contract row or generated define survives; the
C static assertions and `tests/c_layout.rs` measure the new layout; both the
fixture and the C plugin refuse a configuration that does not match what they
declared with `invalid_argument`; and the harness hand-drives the entry through
`init.entry_starts`, `init.future_resolves`, `init.configuration_delivered`,
`init.configuration_mismatch_refused` and
`init.failed_initialisation_instance_destroyed`.

The deviation report lives at
`docs/tasks/metis-plugins/tickets/abi-initialisation-entry/deviation-reports/abi-initialisation-entry.md`
and records six divergences from the arch spec. The consequential ones: the
runtime gained a public hand-off (`Loaded::init_plugin` and a public `PluginInit`
future, plus `CallError::Create` renamed `CallError::Init`) that the spec had
assigned to `async-initialisation`; the published authoring `Plugin` trait broke
for consumers (`Default` and `started` removed, a fallible asynchronous `init`
added); `HostContext` and `Initialisation` sit behind the crate's `authoring`
feature, which `plugin-runtime` deliberately keeps off, so
`async-initialisation` must either build the `{"config", "host_context"}` body
itself or change the dependency posture; and the fixture gained
`FixtureConfig`, its own configuration declaration.

The carried risk from `waker-lifecycle-fix` is closed here: the harness now
releases the owning waker reference as soon as each poll returns, asserts
handed-out equals released, and records a late wake as a request id rather than a
count. Residual risks carried into the effort: the runtime failure path (a
future that never resolves `Ok`, so the host drops it and then destroys the
instance) has no test at the runtime seam and is covered only at the raw ABI
seams, with `async-initialisation` the natural owner; two foreign runtime tests
were adapted (`tests/registry.rs` expects ABI `(1, 2)`, `tests/waker.rs` measures
the call's waker references as a delta after initialisation, still pinning the
call's own two); and nothing here gates the minor bump at bundle-load time, so
`manifest-and-library` owns the `min_abi_minor` refusal.

### Knowledge harvest

The CI gate was re-run on the landing branch at `a9703fb`, after `main` was
merged in, and passes: `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, `cargo test`, `buf lint`, `buf breaking`, `bash
scripts/regenerate-contracts.sh --check`, `bash scripts/build-c-plugin.sh`,
`cargo check -p plugin-contract --no-default-features`, `cargo run -p
plugin-fixture --bin abi-harness` (82 checks, `[result] PASS`) and `bash
scripts/regenerate-abi-header.sh --check`.

Durable lessons folded into the project docs. `docs/plugin-contract.md` carries
the amended table in section 5 and the initialisation payload in section 18, so a
plugin author and the SDK read one source. `docs/testing.md` records the
harness's new checks. The effort's arch spec now says the host-side hand-off
lives in `plugin-runtime` as `Loaded::init_plugin` plus `PluginInit`, not in
`async-initialisation`, and `docs/tasks/metis-plugins/map.md` records the
authoring break as SDK-facing work with no owner in this effort, so
`metis-plugin-sdk` picks the migration up before its tickets are cut. The one
thing a later chain must not rediscover is that a revision bump reaches past the
ABI files: the fixture's initialisation consumes waker references, so a foreign
runtime test that pinned an absolute count had to become a delta.
