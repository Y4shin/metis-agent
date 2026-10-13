# Plugin registry: session handoff

Entry point for continuing the plugin-boundary work. It carries the objective,
the condensed context, the prototyping toolbox, what is already answered, what is
still open, and how to resume the task workflow afterwards.

## Start here

1. Read this document end to end.
2. Read `spikes/plugin-registry/README.md`, then its `FINDINGS3.md` for the
   detailed record, and `spikes/plugin-registry/SEAM-SPEC.md` for the contract.
3. Run the prototype once to see it green, with the commands under "How to run
   the prototype".
4. Then either resume the task workflow as described at the bottom, or, if the
   operator asks for prototyping, run the round described under "Next round (not
   started)".
5. Report before starting anything that spends significant effort.

## Objective

Keep prototyping until every important question is answered, both the ones listed
below and any that come up on the way. Then record the findings where they belong
(see "Where findings go"), and resume the current task workflow effort,
`metis-foundations`, prioritising plugin-related work whenever it is unblocked,
because the plugin ABI is the interface other efforts are waiting on.

Concretely, the loop is: start one prototyping round (one agent advancing the
current prototype, or several agents on their own copies for independent
questions), let the parent analyse the findings, fold the agreed findings back
into the main prototype with one writer, update this document, and repeat until
the open list is empty of things that matter.

A round runs only when the operator asks for one. This loop describes how the
work proceeds once they do; it is not a standing instruction to start
prototyping. When unsure, report and wait.

## Notifications

Use the `notify_user` tool so the operator can walk away and know when to come
back. The same convention applies to every session that continues this work:

- Send one at the end of **every** turn, including turns that only report progress,
  and send one **before** any `ask_user_question` call, so a question never sits
  unseen.
- Keep it short and lead with the actionable part: what is running, what needs the
  operator, or that nothing does. A couple of sentences plus the key numbers is the
  right size.
- Name the identifiers and paths that matter, such as the run id, the commit, and
  which prototype copy or lane produced a result.
- Say explicitly when nothing is needed from the operator. That is the common case
  and it is what makes the channel trustworthy.

## Status as of the prototype commit `25bb165`

- The main prototype now lives in the repository at `spikes/plugin-registry`
  (see its `README.md`). It is throwaway: not product code, not a committed ABI,
  not wired into CI.
- It is green: **92 checks, 0 failures, exit 0** in debug and release, with the
  legacy callback path, the plugin registry, both-direction schema validation,
  version gating, and the host-polled future seam all present.
- Durable: a pre-seam snapshot is kept at
  `~/.local/share/metis/prototypes/plugin-registry`.
- Ephemeral, expect these to vanish and do not depend on them: the seam lane
  copies at `/tmp/metis-plugin-registry-prototype-c1`, `-c2`, `-c3`, the earlier
  candidate copies at `/tmp/metis-plugin-registry-prototype-cand-a` and `-cand-b`,
  and the round-2 scratch under `/tmp/metis-plugin-registry-round-2/`. Their
  results are already folded into the repository prototype and its findings.
- The written seam contract is `spikes/plugin-registry/SEAM-SPEC.md`, and the
  source summary of the async approaches that started this work is committed
  beside it at `spikes/plugin-registry/context/async-ffi-summary.md`.
- Findings: `FINDINGS.md` (registry shape, dyn compatibility, type erasure),
  `FINDINGS2.md` (async and lifecycle surface, registry proper, descriptor and
  naming fixes), `FINDINGS3.md` (the folded host-polled future seam, hazard
  handling, versioning evidence, measurements).

A first attempt at the next prototyping round was started and then stopped on
request before it produced findings. Nothing from it is folded and none of its
numbers are trusted. Its only output is partial scratch under
`/tmp/metis-plugin-registry-round-2/`, which is not committed and may vanish. One
lane there drafted a patch shape for open item O7, which is worth reading before
that item is picked up.

## Status as of round 3 (four lanes, verified in the parent, nothing folded)

- Round three ran four lanes on their own copies under
  `/tmp/metis-plugin-registry-round-3/`: shape (ABI and trait simplification plus
  legacy-path decommission), reclaim (blocked-poll reclaim and the unload rule),
  ownership (payload ownership plus a real C implementer), async-ffi (keep or
  drop). All four completed and every headline number was re-run by the parent.
- The record is `spikes/plugin-registry/FINDINGS4.md`, with the lane findings,
  their evidence and the C plugin source preserved under
  `spikes/plugin-registry/round-3/`, and the parent's own re-runs under
  `round-3/verification/`.
- Nothing is folded into the prototype yet. The operator approved the revised
  contract as the v1 shape on 2026-10-13. The contract text now lives at
  `docs/plugin-contract.md`, and `spikes/plugin-registry/ROUND3-CONTRACT-PROPOSAL.md`
  is the proposal record (decisions D1 to D7) behind it. Folding the approved
  shape into the prototype still has to happen, and where it lands depends on
  which ticket owns it.
- Verified in the parent: shape 95/95 with the futures path on and 20/20 with the
  legacy path compiled out (both in debug and release), reclaim 101/101 with zero
  leaked ctx boxes and waker clones, ownership 92 host checks plus 14 C-probe
  checks with a zero-live C ledger, async-ffi 100/100. The repository prototype
  is untouched (`git status --porcelain` empty).
- What round three settles, pending approval: one call path with no legacy
  callbacks (D1); one capability-name-routed authoring method replacing the
  four-arm vocabulary that caps a plugin at four capabilities (D2); `async-ffi`
  is dropped, because a destructor panic in a plugin future aborts the daemon
  with SIGABRT even for a plugin that returned `Ok` (D3); the unload rule is
  quarantine plus one reclaim on a late poll, and the fixed 900 ms wait is
  deleted (D4); the service ownership rules become contract text after a
  hand-written C implementer drove them (D5); two unread vtable `size` fields are
  deleted or read (D6); and the `metis.*` control plane moves onto the futures
  path so `invoke` can be deleted rather than kept as an inline channel (D7).
- Open items resolved or moved by round three: **O1** and **O2** designed and
  proven, awaiting the operator's rule choice; **O3** written up, awaiting
  contract text; **O4** resolved, drop the crate; **O5** becomes mostly moot
  because D1 deletes the executor seam and the pool; **O7** becomes moot because
  D1 deletes the landed `HostAdapter`, so no separate fix ticket is needed;
  **O10** resolved for today's table by a real C implementer, and re-opened for
  the revised v1 table if D1 and D2 are approved; **O16** answered by D2, awaiting
  approval. Still untouched: O6, O8, O9, O11, O12, O13, O14, O15, O17, O18.
- The approved contract is now a ticket:
  `docs/tasks/metis-foundations/tickets/plugin-contract-v1` (size xl, ready, no
  blockers, and it now blocks `plugin-compatibility`, whose prefix-tolerance
  criterion was removed from it). It lands the whole contract, including the
  parts that would otherwise have waited on `metis-plugins`,
  `metis-plugin-sdk` and `metis-daemon`, because those efforts have maps but no
  tickets yet. The three maps that carried the superseded decisions are amended
  in the same change.

### How to run the prototype

```sh
cd <repo root>
devenv shell -- cargo build --release --manifest-path spikes/plugin-registry/Cargo.toml
devenv shell -- cargo run -q --release --manifest-path spikes/plugin-registry/Cargo.toml -p proto-host
```

Build the workspace first: `cargo run` of the host does not rebuild the cdylib,
and the harness refuses to run against a stale or missing library. `cargo` is
only on the devenv shell's path.

## Condensed context

A plugin is a native shared library loaded into the daemon process. The boundary
is a C function table the plugin exports (`list_capabilities`, `invoke`,
`invoke_future`, `cancel`, `release_buffer`, `shutdown`, `service_complete`) plus
a host table the daemon passes in (`complete`, `progress`, `emit_event`, `log`,
`service_request`). Only opaque pointers, integers, function pointers, and byte
payloads cross; both tables are size-prefixed and append-only, so old readers use
the prefix they know.

There are two ways a plugin can answer a call:

1. **Legacy callback path.** `invoke` returns `Pending`, the plugin finishes the
   work on its own executor, and calls the host's `complete`. The executor is the
   cost problem: one OS thread per call costs 22 to 26 us, a bounded pool costs
   about 5.8 us.
2. **Host-polled future seam.** `invoke_future` returns an opaque context plus
   `poll`/`drop` and waker vtables. The daemon polls the plugin's future on its
   own runtime, and answers the plugin's timer and IO needs through
   `service_request` and `service_complete`. The plugin owns no executor and
   spawns no thread.

Around both sits the host-side registry: routing by capability name, per
capability version gating, duplicate-name refusal, enable and disable,
unregister, and both-direction payload validation against JSON Schema the plugin
publishes. It holds `Box<dyn Plugin>` over two adapters: a native in-process
plugin and an ABI-loaded cdylib.

Measured in the one binary, release, my own runs:

| Path | Cost |
| --- | --- |
| Host-polled, ready on first poll (create + poll + drop) | 864 ns, 440 ns reused |
| Host-polled, one host service round trip | p50 4.5 to 5.1 us, p95 7.2, p99 10.2, max about 100 us |
| Legacy callback path, pooled executor | 5.8 us |
| Legacy callback path, thread per call | 21.5 to 22.8 us |
| Native in-process synchronous call | 661 to 667 ns |
| Edge validation, both directions | about 320 to 390 ns |

Rules the seams depend on: each side frees only what it allocated; every poll and
drop shim and every host callback is panic-contained; one poller at a time with a
bounded poll deadline and a watchdog; cancel marks the token and the future
resolves as `Cancelled` before it is dropped, never dropped mid-flight; and
teardown is stop producers, then join, then drop every future and waker clone,
then unload (`["shutdown returned", "future host dropped", "library unloaded"]`).

## Prototyping toolbox

Techniques that worked, to reuse in later rounds:

1. **Always subagents, never forked parent context.** Use fresh-context children.
   The parent conversation is enormous and forking it is expensive and noisy.
2. **Two shapes of round.** Either one agent advances the current prototype, or
   fan out with one *copy* of the prototype per independent question. Fan-out is
   for questions that do not share code.
3. **Where copies live.** The main prototype is the repository copy at
   `spikes/plugin-registry`. Lane copies live under a per-round directory in
   `/tmp`, for example `/tmp/metis-plugin-registry-round-2/<lane>`. Each lane owns
   exactly one copy, writes nowhere else, and deletes nothing.
4. **One writer.** Lanes never edit the main prototype. After the parent reads the
   lane findings and decides what is agreed, a single agent folds those findings
   into the main prototype.
5. **Write the interface contract once, then implement it independently.** Fan-out
   only works against a written spec (`SEAM-SPEC.md`); independent implementations
   of one spec also validate the spec itself.
6. **Make a killed run diagnosable.** Flush stdout per line, bound every wait with
   a short deadline that reports failure rather than blocking, and print one line
   per check with a non-zero exit on failure. A run killed at a timeout must still
   leave its last marker on disk. Buffered stdout hid a hang for a whole cycle
   once.
7. **Build the cdylib before running the host.** `cargo run` of the host does not
   rebuild a cdylib member, and a stale library produces meaningless results.
8. **Depend on the real crate.** The prototype path-depends on the landed
   `crates/plugin-contract`, so it exercises the real ABI types rather than a
   copy.
9. **Demand honest "unproven" lists, then verify the claims yourself.** Re-run the
   host in the parent, check the counts and exit codes, and compile what the lane
   claims compiles. Do not relay numbers you have not reproduced.
10. **Budget generously.** A chain killed mid-run costs the whole cycle; the lanes
    here used roughly one to three hours each.
11. **Keep the artifact self-contained.** Commit the spec, the findings, and a
    README with the run commands beside the prototype, so a later session needs
    only the repository.
12. **One question per lane.** A lane briefed with four unrelated items ran out of
    its tool budget without measuring any of them. Prefer fewer, sharper items per
    lane, each with a single measurable outcome.
13. **Guard the round authoring itself.** Build each lane's task text as an array
    spread followed by `.join("\n")`, and assert it is a string before dispatch.
    Writing `shared.concat(lines.join("\n"))` silently yields an array, and the
    child then fails with an opaque `input.task?.trim is not a function` that names
    neither the lane nor the line. That mistake cost two dispatch cycles here.

## Answered

**A1. The per-call executor on the plugin side.** Not needed. The legacy callback
path should use an SDK-owned bounded pool (5.8 us against 21.5 to 22.8 us for
thread-per-call), and a plugin using the host-polled seam needs no executor at all
(864 ns ready, 440 ns reused, p50 about 5 us with one host service round trip).

**A2. Can a plugin be genuinely runtime-free?** Yes, proven by a check that reads
the plugin's own counters through the seam, so the measurement cannot itself spawn
a thread: no plugin thread, no plugin runtime. Caveat: only while the plugin gets
its timers and IO from host services; a third-party crate that does its own IO
would still need a reactor.

**A3. Can async tool calls cross the C ABI?** Yes. The plugin hands back a
pollable future object behind opaque pointers and C vtables; `async-ffi` 0.5 works
inside the plugin adapter, and a hand-written state machine with no `async-ffi`
passes the same checks. The future must be `Send`, because the daemon may poll it
from different threads.

**A4. Can the landed ABI absorb it?** Yes, by appending. `cbindgen` emits the new
types into a byte-identical regenerated header, the generated C compiles under
`-std=c11 -Wall -Wextra -Werror` with no diagnostics, Rust and C layout asserts
agree, and seven compatibility checks pass both ways (old host driving a new
plugin through `Pending`/`complete`, new host seeing appended entries, old
short-table plugin still driven, short table refused rather than over-read). The
legacy suite stayed green throughout.

**A5. Which hazards are handled?** Contained: panics in poll, in drop, and in a
host waker; cancel resolving as `Cancelled` then drop; double drop; drop after
unload; a wake after drop; drop during a poll. Detected rather than prevented: a
plugin that ignores the cancel deadline, and a blocking poll (watchdog plus
deadline, recorded as a protocol violation, host stays usable).

**A6. Schema form and transport.** JSON Schema 2020-12 generated from the author's
types, shipped inline in the descriptor, validated offline by the host. The by-id
alternative was measured (2319 byte listing plus 5596 bytes of schemas in one
extra call, against 6014 bytes inline) and is listed as open below.

**A7. Validation policy.** Both directions at the registry. A request that
violates its schema is `invalid_argument`; a response that violates the plugin's
own published schema is an internal error attributed to that plugin, with no
quarantine.

**A8. Registry membership.** `Box<dyn Plugin>` over a native in-process adapter
and an ABI-loaded cdylib adapter.

**A9. Capability naming.** Instance-name qualified, `<instance-name>.<tool-name>`,
with multiple instances of one plugin allowed, and a config-derived instance name.
This supersedes the earlier "refuse duplicate capability names at load" rule.

**A10. Where the registry lives.** Host side (`plugin-runtime`), not in the
primitives crate, because it is serde and JSON shaped; the primitives crate stays
dependency-small and runtime-free.

**A11. Append or a clean replacement.** Append now. A clean replacement is worth
its recompile cost only when the future seam replaces `Pending`/`complete`, not
while both coexist. Resolved: that trigger arrived, and `plugin-contract-v1`
takes the clean replacement. It keeps the name v1, because nothing has been
released yet.

**A12. Earlier decisions still standing.** The trait's four role-named arms and
their capability-count cap are a known defect, and the fix is folded into the open
list below rather than treated as settled.

## Open

Each item says what it is, why it matters, and what settling it looks like.

**O1. Reclaiming an abandoned blocking poll.** Detected, not reclaimable: a plugin
that blocks inside `poll` leaves the daemon unable to free its context box and one
waker clone, because dropping while a poll may still be running inside plugin code
would free memory in use. Settle by designing a reclaim path, for example a
per-poll completion flag or a quarantine list, and proving it.

**O2. Unloading while a blocked poll is still inside plugin code.** Unloading a
library whose code is executing on another thread crashes the process. The
prototype waits about 900 ms. Settle by choosing a host rule: refuse to unload and
keep the mapping, quarantine the plugin, or treat a blocking poll as fatal.

**O3. Payload ownership for the two service entries.** The prototype's reading is
that the service request payload is plugin-owned and released by the host, and the
reply payload is host-owned and borrowed for the call. That is an interpretation,
not contract text. Settle by writing it down and proving it against a hand-written
implementer.

**O4. `async-ffi` breadth.** Only one implementation used the crate, and its
drop-panic abort path is untested. Decide between a second independent
implementation and hand-rolling the future object, which already works.

**O5. Real host integration.** The prototype's timer service is a thread per
request, the watchdog is per call rather than per poll, and pool sizing with idle
re-init has a race. Decide the real daemon's executor integration, which no
prototype has exercised.

**O6. Compile-time agreement of host and plugin payload types.** Only bytes cross
the boundary, so a mismatched struct compiles and is caught at runtime by schema
validation. Decide between generated types on both sides, a shared types crate, or
accepting runtime validation as the only tie.

**O7. The landed `HostAdapter` pending-slot leak.** When `invoke` returns a
synchronous non-`Pending` error, the slot registered before the call is never
retired, and there is no `unregister`, no `Drop` on `Await`, and no cleanup on
timeout. This is a code fix in a real crate rather than prototype material, so
make it a small ticket in `plugin-contract` with a test. A lane drafted the shape
(`unregister`, retire the slot on a non-`Pending` return in `invoke`, and an
adapter-level `wait_timeout` that retires on expiry) and identified two residual
leaks it did not fix, a drop without a completion and the plugin-side request
record, in `/tmp/metis-plugin-registry-round-2/legacy/FINDINGS-round2-legacy.md`
(scratch).

**O8. Error taxonomy against the frozen status space.** The registry must
distinguish a bad request, a version refusal, an unregistered capability, a
plugin-response violation, and a plugin failure, which the frozen statuses cannot
express distinctly. ADR-shaped.

**O9. Effort ownership.** Confirm which effort owns the registry, edge validation,
the SDK's typed facades and schema derivation, and the async executor.

**O10. Cross-language implementers.** The ABI's premise is that other languages
can implement it; no C plugin has ever been compiled, for either async path. Only
a hand-written Rust stand-in exists.

**O11. Descriptor transport.** Inline schemas versus by-id plus a schema fetch.
Both measured; pick one.

**O12. Schema evolution, and large or nested payloads.** Nothing has exercised a
capability version bump or `$ref`/`$defs`, and there is no payload size or nesting
bound anywhere.

**O13. Disable versus unload, and hot unload.** Only disabled-plus-unloaded has
been sketched; the quiescence rules interact with O1 and O2.

**O14. Packaging.** Bundle extraction, signature checks, and platform selection
have never been in scope and are untouched.

**O15. The ABI's inline completion path has no test.** `plugin-authoring-trait`
rewrote the fixture through the authoring trait, whose adapter always returns
`Pending`, so nothing exercises the inline path any more.

**O16. The product shape of the async authoring surface.** Which async shape is the
default for an author, and how the authoring trait exposes it, including fixing
the four-arm vocabulary that caps a plugin at four capabilities. This is the item
that blocks `metis-plugin-sdk` pinning author-facing API.

**O17. Instance-naming sub-details.** Whether the instance name must be globally
unique, who qualifies the capability names, what happens to in-flight calls when
an instance is unregistered, and whether the plugin ever sees the instance name.

**O18. Architecture-spec debt.** The effort's `arch-spec.md` still labels
`plugin-authoring-trait` a pending ticket, and none of the answers above are
reflected in it. Reconcile with the spec-reconciler when the plugin decisions
settle.

## Next round (not started)

Superseded in part: round three has now run (see the round 3 status above). What
remains is the fold of the approved contract into the prototype and then the
implementation ticket `plugin-contract-v1`, not
another round of the shape below. This is the historical plan the next step was
drawn from, not something already run. An earlier attempt was stopped
before producing findings, so start fresh from the repository copy. Reuse or
ignore the abandoned scratch under `/tmp/metis-plugin-registry-round-2/` as you
like.

Fan out one copy per lane under a fresh round directory, one writer per copy:

1. **Blocked-poll reclaim and unload safety** (O1, O2). Design and prove a reclaim
   or quarantine path, and propose the host rule for unload.
2. **Service payload ownership, and a hand-written non-Rust-style implementer**
   (O3, O10). Settle the ownership text and prove it with an implementer that does
   not use any Rust async machinery.
3. **`async-ffi` breadth** (O4). A second independent implementation using the
   crate, exercising cancellation and the drop-panic path.
4. **Legacy path hardening** (O5 only). A timer wheel or deadline queue instead of
   a thread per request, a per-poll watchdog, and pool sizing plus the idle
   re-init race, each with one measured outcome. Keep O7 out of this lane: it is a
   small ticket in `plugin-contract`, not a prototype.

Then fold the agreed findings into `spikes/plugin-registry` with one writer, add
`FINDINGS4.md`, update this document's open list, report the round to the
operator, and wait for them before starting another round.

## Where findings go

- Prototype findings: a new `FINDINGS<n>.md` in `spikes/plugin-registry` per round.
- Answers that are decisions: this document's "Answered" section, and an ADR under
  `docs/adr/` for anything contract-level, such as ABI additions, ownership rules,
  and the error taxonomy.
- The effort's own records: `docs/tasks/metis-foundations/map.md` and its
  `arch-spec.md`, once the plugin decisions are settled and tickets follow.
- Anything that becomes implementation work: tickets in the effort that owns it,
  never ad-hoc files.

## Resuming the task workflow

This prototyping work is a detour, not a replacement. Once the open list is
settled and recorded:

- Resume `/skill:implement-task metis-foundations`. Five of sixteen tickets are
  done: `contract-pipeline`, `plugin-abi-core`, `contract-gate-hardening`,
  `local-transport-rpc-skeleton`, `plugin-authoring-trait`. Eleven remain:
  `plugin-compatibility` first (it was the last level-one ticket and was stopped
  before it ran), then `event-stream`, `local-identity-and-acl`, `tcp-listener`,
  `windows-named-pipe`, then `cancellation-and-slow-consumers`,
  `pairing-and-tokens`, `stream-resume-and-expiry`, `tls-and-client-trust`, and
  finally `remote-pairing` and `reverse-proxy-mode`.
- Prioritise plugin-related work when it is unblocked: `plugin-compatibility`
  (the presence predicate and per-capability version gating) and, once O16 is
  decided, the authoring-surface rework it implies. The plugin ABI is the
  interface the plugin host, the SDK, and the daemon efforts are waiting on.
- Carry the known non-prototype debt: the missing `BUF_TOKEN` repository secret,
  the note that every host callback must be wrapped in `catch_unwind` for the
  `metis-plugins` effort, and the architecture-spec reconciliation in O18.
