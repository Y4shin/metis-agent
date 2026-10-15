---
type: ticket
subtype: feature
title: One asynchronous load entry that initialises and registers a plugin
status: stable
workflow_state: done
blocked_by:
- manifest-and-library
size: l
---

## What to build

One asynchronous entry turns a bundle path into a registered, serving plugin. It
composes verification, extraction, manifest validation, library selection and the
ABI check; it delivers the configuration and the host context to the plugin's
initialisation entry and awaits that future to completion; it cross-checks the
manifest's capability list against what the library reports; and it registers the
instance's capabilities so a call dispatches through the registry and returns a
response. The caller's runtime drives the future and the runtime crate starts
nothing.

## Acceptance criteria

- [ ] One call loads a bundle and returns a registered instance with its
      capabilities, or a typed refusal that names the first thing that was wrong.
- [ ] The plugin receives the configuration and the host context before it is
      registered, and a setup failure in either of the two classes is
      distinguishable by the caller.
- [ ] A plugin whose declared capability list disagrees with its library is
      refused, and the refusal says which capability differed.
- [ ] An instance type needs no `Default`, and the plugin-side constructor is
      asynchronous.
- [ ] A capability from a bundled fixture serves a call through the registry and
      returns its response.
- [ ] The runtime crate starts no thread and no runtime of its own.

## Blocked by

- `manifest-and-library`

## Implementation notes

Landed on `task/async-initialisation` as the merge commit `f406f14`
("ticket(async-initialisation): One asynchronous load entry that initialises and
registers a plugin"), parents `28d22ae` (the landing branch tip, `ticket:
finalize manifest-and-library`) and `4cf353f` (the working branch, `refactor:
keep the load refusals in the order the steps run`). The merge was clean with no
conflicts, and `git diff 4cf353f HEAD` is empty, so the landed tree is
byte-identical to the verified tip. The branch `ticket/async-initialisation` was
deleted after the merge, and no source, test or config file was changed by the
landing step. `docs/tasks/state.yaml` is left uncommitted, matching the earlier
landings in this effort, because `finalize-task` owns the workflow state.

The implementation is 9 files, 1140 insertions and 1 deletion against
`28d22ae`, over nine `wip:` and `refactor:` commits. `plugin_runtime::load`
delivers `load_bundle(host, bundle_path, config, host_context, trust) ->
Result<Serving, LoadError>`, one asynchronous entry that composes `std::fs::read`,
`verify_bundle`, `extract`, the manifest read and this host's library selection,
the `Loaded::open` ABI gate, `Loaded::init_plugin` awaited to completion, the
capability cross-check and `Registry::register`. It answers `Serving` with
`name()`, `instance()`, `capabilities()` and `registry()`, and `LoadError` with
nine variants that keep the trust, stage, manifest, call, capability and
registration refusals typed and the two initialisation failure classes distinct.
The public surface is proven by 11 tests in
`crates/plugin-runtime/tests/load.rs` and one thread-count test in
`crates/plugin-runtime/tests/load_threads.rs`, a second binary at the same seam
because the harness starts a thread per test. The two documentation edits are
contract 17.9, which fixes the cross-check as a comparison of capability names,
and `docs/testing.md`, one bullet per new test file.

The verifier ran against `4cf353f`, inside the devenv shell: `cargo fmt --check`
clean, `cargo clippy --all-targets -- -D warnings` clean, `cargo test -p
plugin-runtime --test load` 11 passed with 0 failed, `cargo test -p
plugin-runtime --test load_threads` 1 passed with 0 failed, and the full `cargo
test` green at 316 passed, 0 failed and 1 ignored over 51 result lines. The six
acceptance criteria were read against the source, not only against the run log:
the refusal tests assert their exact `LoadError` variant and, for the two
disagreement directions, the capability named in the refusal; the tested
instance type has no `Default` and only an `async fn init`; the bundled fixture's
`tool.echo` is called by qualified name through `Serving::registry()` and returns
its own response body; and the thread count is unchanged across a whole load.

Two criteria needed no product code: the plugin-side asynchronous constructor and
the instance type without `Default` landed with `abi-initialisation-entry`, and
the crate already started no thread, so those tests passed on their first run.

Before the notes above were written this ticket doc ended at `## Blocked by`,
with no `## Implementation notes` section, unlike the other landed tickets in
this effort.

The deviation report lives at
`docs/tasks/metis-plugins/tickets/async-initialisation/deviation-reports/async-initialisation.md`
and records divergences from the arch spec. The consequential ones: `load_bundle`
takes `&Host` as its first parameter, because the sink is set when the host is
built, the drop-then-destroy ordering on a failed initialisation can only be read
from the host's ledger, and "before it is registered" is asserted from the host;
`Serving` carries the registry the entry registered into; `config` and
`host_context` are `&serde_json::Value`, because `Initialisation` and
`HostContext` live behind the `authoring` gate this crate keeps off; the
cross-check compares capability names only, since the manifest's schemas are
declared text and the library derives its own; and the fixture gained the
reserved instance name `DEFECTIVE_INITIALISATION` so a plugin's own setup failure
is reachable with a real bundle.

Residual risks carried into the effort: a manifest name that carries a dot does
not route, because `Registry::route` recovers the tool half with the first-dot
split, so the ticket's test chain uses a dotless leaf name and `startup-policy`
and `sample-bundle-ci` must allow for it; `LoadError::Registration` is
unreachable with the in-tree fixture and has no test; and the cross-check is
order-insensitive by name set, so a declaration with several differences reports
one of them.

### Knowledge harvest

The CI gate was re-run on the landing branch at `06b6ea0` and passes: `cargo
fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `buf
lint`, `buf breaking`, `bash scripts/regenerate-contracts.sh --check`, `bash
scripts/build-c-plugin.sh`, `bash scripts/check-c-layout-asserts.sh`, `cargo
check -p plugin-contract --no-default-features`, `cargo run -p plugin-fixture
--bin abi-harness` and `bash scripts/regenerate-abi-header.sh --check`.

One contradiction this landing exposed is fixed in the effort map, so the two
artifacts agree: the map's abort list read as if a plugin defect aborts start-up,
while this effort's spec (stories 41 and 42) and the `startup-policy` ticket both
make a failed initialisation degrade to a disabled placeholder, with a refused
unload during that recovery as the abort. The map now says so, and the arch spec
carries the same split with the classification `startup-policy` applies.

Durable lessons for the tickets that follow. `load_bundle` takes the host first,
which is what lets `callback-sink` set a sink before a load and `sample-bundle-ci`
observe an event after one; a failed load returns no `Serving`, so the host
parameter is also the only way to observe the drop-then-destroy ordering. The
registry the entry builds is per load and reachable through `Serving::registry()`,
which `teardown-and-release` uses and `startup-policy` must decide whether to
aggregate. Manifest names must carry no dot, and the capability cross-check
compares names only, both of which the later tickets inherit rather than
re-decide.
