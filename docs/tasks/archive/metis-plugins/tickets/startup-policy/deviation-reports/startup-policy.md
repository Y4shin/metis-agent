---
type: deviation report
title: Deviation report for startup-policy
status: stable
---

## Deviation report: startup-policy

Compared: `docs/tasks/metis-plugins/arch-spec.md` section `startup-policy`, the
ticket doc and its six acceptance criteria, the recorded resolution at
`docs/tasks/metis-plugins/tickets/startup-policy/.work/resolution.md`, and the
implementation on `ticket/startup-policy` (`git diff
task/startup-policy..ticket/startup-policy`: twelve files, 1897 insertions, 11
deletions, nine `wip:` commits from `add6501` to `636f874`).

### API surface changes

**1. `start_plugins` returns a `Result`, and the stop carries the refusing
bundle.**
- **Planned:** `start_plugins(dir, config, trust) -> StartupReport`.
- **Actual:** `pub async fn start_plugins(dir: impl AsRef<Path>, config:
  &Map<String, Value>, trust: &TrustStore) -> Result<StartupReport, StartupStop>`.
  `StartupStop` exposes `path()` and `error()` and carries the bundle that stopped
  the pass plus the `LoadError` it answered. `StartupReport` exposes `host()`,
  `serving()`, `disabled()`, `offered_capabilities()` and `async
  call(qualified, body, request_id)`. `DisabledPlugin` exposes `name()`,
  `reason() -> &LoadError` and `capabilities()`.
- **Impact:** the spec's signature could not express the ticket's own stop rule,
  because a stop has to return in place of a report, and the `async` follows from
  awaiting every load. `teardown-and-release` and any daemon composition must
  handle the `Result`, read `Serving::registry()` per serving plugin rather than
  one shared registry, and treat a placeholder's reason as a typed `LoadError`
  whose string form appears only in the route error.

**2. `LoadError` gains a tenth variant, `UnloadRefused { plugin, step, status }`.**
- **Planned:** "ONE additive `LoadError` outcome that names the plugin and the
  refused step" (the resolution).
- **Actual:** as planned, plus a third field, the `MetisStatus` the step answered,
  because a step name without its status cannot be told from a success. The record
  that feeds it (`RefusedUnload`, `HostInner::refuse_unload`,
  `Host::take_refused_unload`) is `pub(crate)`, so the public `Host` surface did
  not grow; a caller that drives `PluginInit` by hand still reads the refusal only
  from the ordering ledger.
- **Impact:** the arch spec's level-3 sentence "nine typed refusals" is stale and
  is the reconciler's to fix. `teardown-and-release` reads the same recovery: a
  refused `destroy_instance` now has a typed outcome on the load path, and no
  outstanding-clone gate is implied.

**3. `RouteError::Disabled(String)` is now `Disabled { name, reason }`.**
- **Planned:** `RouteError::Disabled(String)`, with the ticket asking for "the
  existing disabled route error with its reason".
- **Actual:** widened to two fields, as the resolution directs. The registry's own
  switch-off carries the fixed reason `"the host turned this entry off"`, and a
  placeholder carries its load refusal's `Display` text. `RouteError::status()`
  still answers `PermissionDenied` for it.
- **Impact:** a breaking change to a landed public enum. The one landed assertion
  that matched the tuple form (`crates/plugin-runtime/tests/registry.rs`) was
  adapted mechanically and is the only foreign test touched; a later consumer that
  matched `Disabled(_)` must adapt.

**4. The manifest's classification is finer than the arch spec's level-3 table.**
- **Planned (level 3):** `Trust`, `Unreadable` and `Manifest` stop start-up;
  `Stage`, `Call`, `CapabilityMismatch` and `Registration` disable.
- **Actual:** `Manifest(Absent | Unreadable | Invalid | NameMismatch)` stops, and
  `Manifest(NoTargetLibrary | LibraryNotStaged)` disables, so the four library
  refusals, two from extraction and two from the manifest, share one disable
  reason with four distinguishable names. This follows the ticket's own "What to
  build" list and the `startup-policy` section's own disable list; the coarse
  level-3 sentence was the outlier, and a RED cycle caught the contradiction.
- **Impact:** the arch spec needs the reconcile. No dependent is affected:
  `stops_startup` is a private predicate and contract section 19 states the rule.

**5. A placeholder is not registered anywhere; the report answers it.**
- **Planned:** "its disabled placeholders need a registry of their own, because a
  failed load returns no `Serving`".
- **Actual:** placeholders live in `StartupReport::disabled()`, and
  `StartupReport::call` answers a declared capability of a disabled plugin from
  `disabled_capability()` before it delegates to each serving plugin's own
  registry. That lookup matches the placeholder's name as a prefix and then
  requires the remainder to be a declared capability, so a dotted placeholder name
  stays unambiguous and an undeclared name under it still answers `Unknown`. There
  is no placeholder registry and the policy does not aggregate registries.
- **Impact:** a daemon that wants one registry for everything registers
  placeholders itself. `teardown-and-release` must not expect to find a placeholder
  in a `Registry`, and nothing here unloads a placeholder, because a failed load
  never produced a library to unload.

**6. One `Host` for the whole pass, one registry per plugin, no aggregation.**
- **Planned:** the arch spec leaves the choice to the ticket, and the landed host
  rule is one host per library.
- **Actual:** one `Host::new()` per pass, passed to every load; each `Serving`
  keeps the registry its own load built, so a duplicate plugin name across two
  bundles is not refused here.
- **Impact:** `teardown-and-release` owns the aggregation question. A directory
  whose bundles bind different libraries reaches the host's `bind_release`
  refusal, which warns on stderr and is recorded rather than fixed.

**7. Configuration and host context are keyed by the bundle file's stem, not by
the name the load reports.**
- **Planned (the resolution):** "per-plugin values the caller resolves through a
  name to value map, and build the host context inside `start_plugins` from the
  name the load reports".
- **Actual:** the initialisation payload is built before the load runs, so the only
  name available then is the file's stem; both the configuration lookup and
  `host_context` use it. The manifest's name is known only after the load or the
  placeholder re-read, so a placeholder can carry a manifest name that differs from
  the key its configuration would have used.
- **Impact:** a daemon must key its configuration by file stem, or name its bundle
  files after its plugins. The arch spec should state the key explicitly, and
  `sample-bundle-ci` must name its sample bundle file after the plugin if the two
  are meant to agree.

**8. `host_target()` is promoted from private to `pub(crate)` in `load.rs`.**
- **Planned:** `bundle-extraction` carried "the host-target derivation has no
  exported helper" as a residual, and the alternative here was a third copy.
- **Actual:** one `pub(crate)` derivation in `load.rs`, reused by `startup.rs`'s
  placeholder re-read.
- **Impact:** narrows the carried risk to one derivation inside the crate; nothing
  public changed.

### Abstraction usage

- Used/was specified: **yes**. `BundleError::is_trust_failure()` is called as the
  predicate for the trust half of the split rather than matched by variant,
  `LoadError` is classified by variant in `stops_startup`, `Registry`'s entry and
  route-error surfaces carry the placeholder's answer, the policy builds the
  `&Host` that `load_bundle` takes first, and the placeholder re-read reuses
  `verify_bundle`, `extract`, `Manifest::read` and `host_target` instead of
  re-implementing them.
- The do-not list held in full: no second registry and no second route error, a
  trust failure is never turned into a placeholder, a container this build cannot
  read is never turned into a stop, and discovery is the directory's own regular
  files with no recursion and no discovery file.
- One mechanism the spec suggested was not used: the "registry of their own" for
  placeholders (item 5). The replacement keeps one routing path and is smaller,
  but it is a mechanism change rather than a rename, so the spec's sentence should
  be corrected rather than read as satisfied.

### Out-of-scope changes

- `crates/plugin-runtime/src/lib.rs` (+55): the crate-private `RefusedUnload`
  record, `HostInner::refuse_unload`, `Host::take_refused_unload`, the
  `DESTROY_INSTANCE_STEP` constant, the refusal recorded in `PluginInit::Drop`, and
  the `pub mod startup;` line. No public `Host` method was added.
- `crates/plugin-runtime/tests/registry.rs`: two lines, the adapted `Disabled`
  assertion, which the resolution explicitly allowed.
- `crates/plugin-runtime/tests/support/mod.rs` (+67): `compiled_fixture(name)`
  with `repository_root`, `rustc` and `built_after`, for a fixture compiled at test
  time.
- `crates/plugin-runtime/tests/fixtures/refusing_destroy.rs` (232 lines, new): a
  second hand-written implementation of the contract's shapes whose first poll
  fails and whose `destroy_instance` answers `internal`, asserting its own field
  offsets in `const` blocks so a drifted restatement fails to compile.
- `crates/plugin-runtime/tests/startup.rs` (1031 lines, 21 tests): its own
  directory, chain and manifest harness, because `support/bundle.rs`'s manifest
  writer hardcodes one plugin name. The duplicated harness is a coherence
  candidate: parameterising the shared writer by name would remove it.
- `docs/plugin-contract.md` (new section 19) and `docs/testing.md` (the new test
  command) are the planned documentation, not out-of-scope.
- Nothing was staged; no file outside `docs/` and `crates/plugin-runtime/` changed.

### Ticket doc update needed?

Yes. The ticket doc has no `## Implementation notes` section yet, so the landing
step should create one and record: the nine `wip:` commits and the tip; the
twelve-file, 1897-insertion diff; the acceptance evidence per criterion; the two
headline deviations (the tenth `LoadError` variant and the widened
`RouteError::Disabled`); the finer `Manifest` classification against the arch
spec's stale level-3 sentence; the placeholder mechanism with no registry; the
file-stem configuration key; the promoted `host_target`; and the new fault fixture
and support helper.

The same note should carry the residual risks: `LoadError::Unreadable` for a
bundle file and `ManifestError::Unreadable` have no portable test, because
provoking an unreadable regular file is not portable in a test that may run as
root; `LoadError::Registration` stays untested and its policy arm is one line; a
stop mid-pass drops the `Serving` values that already started, which releases
those instances only through the `Loaded` lifetime rather than through an unload
path, because `Serving::release` belongs to `teardown-and-release`; criterion 6 is
satisfied by prose only, matching the landed precedent for documentation
criteria; the placeholder path re-reads verification, extraction and the manifest
for a disabled bundle, which the resolution accepted as the cost of an unchanged
load surface; and the pass shares one host, so its single refusal slot is only
sound because the loop loads sequentially, which a concurrent caller of
`load_bundle` would have to keep true for itself.

### User attention needed?

Yes, two sentences. First, `RouteError::Disabled` is a breaking change to a landed
public enum, taken as directed, and the only in-tree consumer was one assertion in
the registry tests, so nothing else in this effort needs adapting. Second, the
configuration key is the bundle file's stem rather than the plugin's manifest
name, which is forced by the initialisation payload being built before the load,
but it is a rule the daemon and `sample-bundle-ci` must follow and it should be
stated explicitly in the arch spec at the next reconcile.
