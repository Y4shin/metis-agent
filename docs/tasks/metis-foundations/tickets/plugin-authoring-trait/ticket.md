---
type: ticket
subtype: feature
title: Plugin authoring trait and adapters
status: deprecated
workflow_state: done
blocked_by:
- plugin-abi-core
size: m
---

## What to build

The Rust authoring experience: a plugin author implements a trait and never sees
the C boundary, with adapters translating both directions, so that plugins are
ordinary async Rust while the ABI stays stable.

## Acceptance criteria

- [ ] The authoring trait and both adapters live in the plugin contract crate
      behind a feature gate, so the runtime depends on the primitives and payload
      types alone.
- [ ] The trait exposes a cancellation token on the per-request context, plus a
      poll convenience, and a call blocked on I/O observes cancellation
      promptly.
- [ ] The adapter always returns pending and completes through the host callback,
      while the ABI keeps its inline path for plugins that implement it directly.
- [ ] A lifecycle hook exists, and plugin-originated events have a route through
      the trait, asserted by a test where an event reaches the host.
- [ ] Capability descriptors and dispatch arms are checked against each other at
      instance creation, so a missing arm fails loudly instead of appearing as a
      not-found at call time.
- [ ] The error that suppresses a completion after a successful cancel is
      documented and covered by a test.
- [ ] The fixture plugin is rewritten through the trait, passes the same harness,
      and the trait module imports no ABI type.

## Blocked by

- `plugin-abi-core`, because the adapters translate onto the primitives.

## Implementation notes

Landed on `task/plugin-authoring-trait` as the merge commit `34ee80f`
("ticket(plugin-authoring-trait): Plugin authoring trait and adapters"), parents
`beba914` (the landing branch, which was level with `main`) and `2753d32`
(working branch `ticket/plugin-authoring-trait`). The merge was clean with no
conflicts, the landing tree is byte-identical to `2753d32` (`git diff 2753d32
HEAD` is empty), and `ticket/plugin-authoring-trait` was deleted after the
merge. Because the landed tree is the tree the verify worker tested, no gate was
re-run at landing. The review artifact
`deviation-reports/plugin-authoring-trait.md` is committed with this note.

1. The trait's capability arm vocabulary is fixed at four arms and the
   capability names belong to the plugin. The arch spec asks for four things
   that cannot all hold at once: one async method per capability *on the trait*,
   plugin-chosen capability names (`tool.echo`, `tool.panic`), descriptors and
   dispatch arms as *separate declarations* compared at instance creation, and
   no macro. A per-capability trait method fixes the arm vocabulary, so the
   names are the plugin's (`const ANSWER`/`DEFERRED`/`CANCELLABLE`/`FAULT`, each
   defaulting to `""` = not implemented) and `capabilities()` stays the separate
   declaration the check compares them with. A plugin may implement fewer than
   four arms (unused arms are unnamed and answer `Unsupported`), but the
   *number* of arms is a trait change. A generic capability set would need a
   macro or a runtime dispatch method, which this ticket forbids. This is the
   one place the landed surface is narrower than the spike's "one method per
   capability" shape suggests, and it is the item to re-decide before
   `metis-plugin-sdk` pins author-facing API.
2. `PluginError`'s status mapping lives in the adapter (`plugin_adapter`), not
   on the error type, which is what keeps `src/authoring/traits.rs` free of ABI
   types (criterion 7).
3. The allocation ledger moved into the adapter (`authoring::alloc`), because
   the adapter is what allocates every buffer now. The fixture still *reports*
   it: its `stopped` hook logs `FIXTURE_ACCOUNT allocs=... frees=... live=...
   violations=... suppressed=...`, so the harness's ownership checks are
   unchanged. The fixture's own `plugin_alloc`/ledger/`release_buffer` code is
   gone.
4. `shutdown` collects the instance handle (ported from the landed fixture,
   which the harness asserts: `destroy_after_shutdown_not_found`). Recorded
   behaviour: `shutdown` releases the instance; a later `destroy_instance` on
   the same handle is `NotFound`. `create_instance` also clears a reused address
   from the released set, which the landed fixture did not do and which a second
   instance needs.
5. Harness expectations moved and checks added. `invoke.inline_result` is now
   `invoke.always_pending_completion`, `panic.exported_function_is_internal` is
   now `panic.capability_panics_into_an_internal_completion` (the panic is
   caught in the worker, so it arrives as a completion), the suppression needle
   names the adapter's log line, and `event.started_emits`,
   `event.capability_emits`, `cancel.parked_call_is_woken_promptly` and
   `cancel.cancelled_without_a_cancel_still_completes` are new. 41 checks before,
   45 after; workspace tests 16 before, 30 after (14 added, none removed).
6. Fixture additions beyond "rewritten through the trait": a fixture-local
   bounded wait (`WaitFor` + `Timer`) with a 1.5s budget, so the second half of
   the cancelled-completion rule is observable through the harness, and
   `pub struct Echo`, the payload shape the plugin's associated types name.
7. Public items the spec did not list: `Shape`, `CapabilityDescriptor`,
   `CapabilityKind`, `CapabilityList` (the adapter must serialise the D9
   declaration), `CancellationToken`/`Cancelled`, `Host` + `LogLevel` (the
   lifecycle hooks need routes with no request to attach them to), `Accounting`
   + `accounting()`, and `executor::catch_panic`/`CatchPanic` (D7 needs the
   worker to catch a plugin panic rather than lose the completion). No new
   public constant, so no cbindgen `exclude` entry was needed and the header
   drift check still exits 0. Measured: cbindgen does read the gated module (it
   warns that it skips the adapter's non-exported `cancel`/`shutdown`
   functions), so the script's "parses the crate with default features" is doing
   less work than it sounds; what keeps the header stable is the `[export]
   include` list.
8. The descriptor types now live in `plugin-contract::authoring`, and
   `crates/plugin-fixture/src/payload.rs` re-exports them.
   `plugin-compatibility` (pending sibling) expects the fixture's payload
   module to be "the one place the payload shapes live": the
   request/response/progress shapes still are, the descriptor is now the
   adapter's (it has to serialise it), and the harness keeps parsing through the
   fixture's re-export.
9. `docs/testing.md` gained a paragraph on the feature's run commands (the
   effort's rule that testing.md owns them). No CI change was needed: the
   existing steps cover the authoring code and its tests.
10. Residual risks. (a) The feature-*off* compile of the crate is not compiled
    by the workspace test build, because feature unification turns `authoring`
    on through the fixture; the declared dependency list is asserted by
    `feature_gate.rs` and `cargo test -p plugin-contract` alone exercises the
    feature-off path. A `cargo check -p plugin-contract --no-default-features`
    CI step would make it explicit; the ticket asked for no new step, so none
    was added. (b) The four-arm vocabulary (note 1) is the item most worth
    revisiting before `metis-plugin-sdk` re-exports this surface. (c) Nothing in
    the tree exercises the ABI's inline `invoke` result any more; a
    direct-implementer regression there is uncovered until a fixture or host
    adds one. (d) Two timing assumptions: the fixture's 1.5s cancellation budget
    and the harness's 250ms promptness bound, both measured at 0ms here. (e)
    `HostAdapter::callback_panics()` is the host's D7 obligation made observable;
    nothing in-tree triggers a panic inside the contract's own callback bodies,
    so that counter stays 0 and those catch paths are defensive.
11. Notable events. The three load-bearing tests were mutation-checked rather
    than trusted green (token wake removed, `check_declaration` disabled,
    `is_cancelled()` guard removed: each turned its test red, all restored). The
    first parallel test run SIGSEGV'd because a unit test handed `HostAdapter` a
    pointer to a stack-local `table::<P>()` while a worker outlived the frame;
    the test now leaks one. The adapter's pending registry, worker list and
    ledger are process-wide, so the unit-test binary serialises its cases behind
    one mutex and namespaces request ids.
12. Verification (PASS) recorded on `2753d32`: `cargo fmt --check` clean, `cargo
    clippy --all-targets -- -D warnings` clean, the authoring suites green (11
    `authoring.rs` + 3 `feature_gate.rs`), the harness `[result] PASS (45
    checks)`, the full workspace `cargo test` at 30 tests / 0 failures, and
    `bash scripts/regenerate-abi-header.sh --check` reporting no drift. No staged
    files at landing; the workflow's own `docs/tasks/state.yaml` pointer remains
    finalize-task's to move.

