---
type: ticket
subtype: bug
title: A capability name routes by its owner, not by the first dot
status: stable
workflow_state: done
blocked_by: []
size: s
---

## What to build

`Registry::route` finds the owner through the routing map, which is keyed on the
exact `<instance>.<tool>` string, and then recovers the tool half by splitting the
qualified name on its first dot. An instance name that itself carries a dot
therefore reads as the instance plus part of its own name, so the capability
lookup fails with `Unknown` even though the route map resolved the entry. The
tool half must come from the owner the map returned, so a dotted instance name
routes like any other and an undotted neighbour stays independent.

`async-initialisation` found this: its bundles carry a dotless name and its tests
work around the case, and the manifest and bundle file names a daemon reads are
not guaranteed dotless.

## Acceptance criteria

- [ ] An instance registered under a name carrying a dot routes every capability
      it declares, and `gate`, `validate_request`, `validate_response` and `call`
      answer it as they do for an undotted name.
- [ ] A dotted instance name and its undotted neighbour resolve independently.
- [ ] The registry tests and the workspace suite pass.

## Blocked by

- None (can start immediately).

## Implementation notes

Landed on `task/dotted-instance-names` as the merge commit named below.
Implemented by the orchestrating parent at the operator's instruction, after
`async-initialisation`'s verifier surfaced the defect and the operator chose a
direct fix over a chain dispatch, so this ticket records the work rather than
reporting a worker's. `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, `cargo test -p plugin-runtime --test registry` and the full `cargo
test` (51 test targets, all ok) pass on the landed tree.

One change in `crates/plugin-runtime/src/registry.rs`: `route` now derives the
tool half from the owner the routing map returned for the exact qualified key,
instead of splitting the qualified name on its first dot. The map is keyed on
`<instance>.<tool>`, so an instance name carrying a dot ("metis.echo") is
unambiguous there, while the first-dot split read it as the instance plus part
of its own name and the capability lookup then failed with `Unknown` for a
route the map had already resolved. The undotted case is unchanged: the owner is
still the whole prefix and the tool half is what follows its separator.

One test in `crates/plugin-runtime/tests/registry.rs`,
`an_instance_name_carrying_a_dot_routes_like_any_other`: it registers the
fixture under "metis.echo", asserts `route`, `gate`, `validate_request` and
`call` all answer `metis.echo.tool.echo`, and then registers a neighbouring
instance under "metis" and asserts the two namespaces resolve independently.

Contract text needed no change: section 11 rule 8 already defines a capability
name as instance-qualified, and this ticket makes the implementation match it
for every instance name rather than for dotless ones alone.
