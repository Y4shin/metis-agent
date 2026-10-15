---
type: findings
title: What the plugin control plane's counters were for, and whether anything consumes them
status: stable
---

## Question

`docs/plugin-contract.md` names "counters" once directly, in section 14 item 7
(line 270), as a member of the `metis.*` control plane that moves onto the
futures path, and once indirectly, in section 11 rule 6 (line 201), where a
quarantined plugin's "counters stay honest". Nothing in the tree defines the
term. This resolves three things: where the word came from and what it referred
to at each place, whether anything consumes it, and whether it is orphaned. It
also checks the operator's reading that counters may have meant atomic request-id
creation to map running futures, which would make them host state rather than
plugin state.

All revision-sensitive claims below cite the commit, and the current tip is
`d403d8b`.

## Occurrences and their sources

### In the current contract (HEAD `d403d8b`)

1. `docs/plugin-contract.md:270`, section 14 item 7:
   "The `metis.*` control plane (schemas, configure, counters) moves onto the
   futures path so `invoke` can be deleted structurally rather than surviving as
   an inline side channel."
   This is the direct occurrence. `counters` is a third member beside `schemas`
   and `configure`.
2. `docs/plugin-contract.md:201`, section 11 rule 6:
   "A plugin with an abandoned, still-running poll is quarantined ... and its
   counters stay honest."
   Here `counters` means the instance's accounting remains accurate while
   quarantined. It is not a capability.
3. `docs/plugin-contract.md:264`, section 14 item 4:
   "one field serving as both the leak counter and the quarantine predicate".
   This names `abandoned_active` explicitly. It is the same counter sense as (2),
   not the control-plane sense.
4. `docs/plugin-contract.md:294`, section 15, "Control plane":
   "only the per-call control capabilities (configure, counters) move onto the
   futures path".
   Same sense as (1): `counters` is a per-call control capability.

Git history: `docs/plugin-contract.md` was created in `dc47e5f` (2026-10-13,
"docs: approve the plugin contract v1"). Lines 270 and 201 already carried the
word at creation (then at lines 270 and 202), so the term entered the contract
with the first version of the file, copied from the spike proposal. `39d72e6`
(2026-10-13) later renamed the revision from v2 to v1 and kept the wording.

### Provenance in the spike

5. `spikes/plugin-registry/ROUND3-CONTRACT-PROPOSAL.md:155`, decision D7:
   "re-express the `metis.*` control calls (schemas, configure, counters) as
   capabilities on `invoke_future`, so `invoke` can be structurally deleted".
   This is the sentence the contract copied. It was introduced in `54bd746`
   (2026-10-13, "spikes(plugin-registry): round 3 findings, verified, fold
   pending approval"), the same commit that added the round 3 shape and reclaim
   findings.
6. `spikes/plugin-registry/round-3/shape/FINDINGS-shape.md:170`, the Deleted
   surface table, has the row
   "`schemas / configure / accounting / counters` | cdylib `metis.*` | protocol |
   **keep**, inline". The row lists four items, so `counters` is distinct from
   `accounting`.
7. `spikes/plugin-registry/round-3/shape/FINDINGS-shape.md:51` and `:148-149`:
   "the plugin's own counters show `threads_spawned` delta 0 and `poll_threads`
   1"; "the plugin's own counter read back through the seam
   (`metis.future_counters`, itself a host-polled future ...)".
8. `spikes/plugin-registry/round-3/shape/FINDINGS-shape.md:384`:
   "`metis.*` over futures is sketched, not driven."

### What the prototype's counters actually were

The prototype cdylib defines no `metis.counters` capability. Its inline `metis.*`
control plane is `metis.schemas`, `metis.descriptors_inline`, `metis.configure`,
`metis.accounting`, `metis.future_stats` and `metis.executor`
(`spikes/plugin-registry/crates/proto-plugin-cdylib/src/lib.rs:642-659`). The
host-internal set is named at `spikes/plugin-registry/FINDINGS2.md:78-81`:
`metis.schemas`, `metis.configure`, `metis.descriptors_inline`, `metis.accounting`.

The capability that returns counters is `metis.future_stats`, and in the future
lane `metis.future_counters`:

9. `spikes/plugin-registry/crates/proto-plugin-cdylib/src/future.rs:115-117`
   ("The counters the host reads back, through the host-polled
   `metis.future_counters` capability or the legacy `metis.future_stats` one").
10. `spikes/plugin-registry/crates/proto-plugin-cdylib/src/future.rs:535-538`
    and `:682-683` (the `Counters` hand mode returns `stats_json()` as the
    payload).
11. `spikes/plugin-registry/crates/proto-plugin-cdylib/src/future.rs:924-927`
    (`"metis.future_counters"` route, deliberately not advertised).
    The fields of `stats_json()` are plugin-owned: `futures_created`,
    `futures_dropped`, `poll_calls`, `poll_threads`, `threads_spawned`,
    `waker_clones`, `waker_releases`.

So the word `counters` in D7 and in the deleted-surface row refers to the
plugin-side metrics capability (`metis.future_stats` / `metis.future_counters`),
read back by the host. It does not refer to `metis.accounting` (buffer
ownership), which is listed separately.

### The request-id reading (the operator's guess)

12. `spikes/plugin-registry/crates/proto-registry/src/lib.rs:259-289`:
    `static NEXT_CALL_ID: AtomicU64 = AtomicU64::new(1);` and
    `Call::new()` returns `with_id(NEXT_CALL_ID.fetch_add(1, Ordering::SeqCst))`.
    This is the only atomic request-id counter in the prototype. It lives in the
    shared `proto-registry` crate, not in the `metis.*` control plane, and no
    document names it `counters`.
13. `docs/plugin-contract.md:236`, section 13, "Who owns what":
    "The daemon owns: the poll loop and its runtime, request id allocation
    policy, ...".
14. `crates/plugin-contract/src/authoring/call.rs:344-356`: the real
    `Call::new(id, host, token)` receives the id, and `Call::id()` returns it.
    The host supplies the id; the plugin does not allocate it.

### Measurement output (not definitions)

The `NOTE future seam counters: {...}` lines in the round 3 and host output files
are runtime measurements of the plugin counters read back through the seam, for
example `spikes/plugin-registry/round-3/verification/verify-reclaim-release.txt:118`
and `:214`, and `spikes/plugin-registry/host-output-seam-release-1.txt:118`. They
are evidence of what the counters reported, not a contract concept, and nothing
in the real tree reads them.

### Unrelated `counter` senses (checked and excluded)

- `docs/tasks/archive/metis-foundations/arch-spec.md:412` ("the harness counter
  agree"), `:1204` (canary counter), `:4995` (the hazard counter "exists but no
  fixture capability produces one on purpose"). These are the ABI harness and
  hazard counters, not the control plane.
- `docs/tasks/archive/metis-foundations/tasks/realtime-semantics/task.md:74` and
  `arch-spec.md:2910`: a daemon-wide monotonic event counter. Unrelated.
- `crates/api-contracts/src/events.rs:863`: "one counter serves every channel".
  Unrelated.

## Consumers

Explicit search at HEAD (`d403d8b`):

```
grep -rn "metis.counters\|metis_counters\|\"counters\"\|Counters" \
  --include=*.rs --include=*.md --include=*.toml .
```

This finds no definition and no consumer of a counters capability. The only
`Counters` identifier is the spike's `HandMode::Counters`
(`spikes/plugin-registry/crates/proto-plugin-cdylib/src/future.rs:538`), and the
only `metis.counters` mention is the open question at
`docs/tasks/metis-plugins/tasks/configuration-and-host-context/task.md:206`. A
scoped search of `crates/**/*.rs` for `counters` returns only comments at
`crates/plugin-fixture/src/bin/abi-harness.rs:24,1136,1173,1507` and
`crates/plugin-runtime/src/lib.rs:626`.

There is no consumer, and no definition, of a `metis.*` counters capability in
the current tree.

The candidates named in the task resolve as follows:

- **Plugin-side accounting.** `crates/plugin-contract/src/authoring/alloc.rs`
  defines `Accounting { allocs, frees, live, violations }` with the `ALLOCS`,
  `FREES` and `VIOLATIONS` atomics. It is consumed by the fixture, which logs it
  as `FIXTURE_ACCOUNT` (`crates/plugin-fixture/src/lib.rs:153`), and by
  `crates/plugin-contract/tests/authoring.rs:494,522-526,618,682-696,790-796`.
  This is a buffer-ownership ledger, present and used, but it is not the
  control-plane `counters` and is not reported through a capability.
- **Host waker counts.** `crates/plugin-runtime/src/lib.rs:351-356` defines
  `Host::waker_counts()`, returning `(cloned_wakers, released_wakers,
  wakes_after_drop)`. The comment at `:626` states "The counters live on the
  host, not here". Consumed by `crates/plugin-runtime/tests/concurrency.rs:72`
  and recorded in `docs/tasks/metis-plugins/map.md:136`. These are host-owned,
  always present, and not a capability.
- **Ordering ledger.** `plugin-runtime`'s `order(...)` ledger. It is a ledger,
  not counters.
- **Fixture ledger.** The same `Accounting` line as the first item.
- **ABI harness counter checks.** `crates/plugin-fixture/src/bin/abi-harness.rs:24,1136,1173,1507`
  keep host-side waker and service counters and compare them with the fixture's
  ledger. Harness-internal, not a contract capability.

Net: nothing consumes a counters capability today, and no code needs one. The
two real mechanisms behind the two readings (host waker and request-id
bookkeeping, and the plugin buffer ledger) already exist outside the contract and
are not exposed as a capability.

## Recommendation

**Remove the term and the feature from the contract text. Leave v1 without a
counters capability.**

Rationale:

1. Nothing in the tree defines or consumes `metis.counters`, so no dependant
   breaks when the word leaves.
2. The two needs the word might have named are already met, each without a new
   capability:
   - Host-owned request-id and future and waker bookkeeping is host state.
     Section 13 already assigns request-id allocation policy to the daemon
     (`docs/plugin-contract.md:236`), and `Host::waker_counts()` already reports
     the waker counts. The operator's reading is a real mechanism in the spike
     (the `NEXT_CALL_ID` atomic, item 12), but the sources never call it
     `counters` and never make it a `metis.*` capability. It needs no plugin
     facing shape.
   - Plugin-owned accounting reported to the host is the actual origin of the
     word (the prototype's `metis.future_stats` and `metis.accounting`
     counters), and it is diagnostics. The real `Accounting` ledger already
     serves that role for the fixture and tests. v1 does not need it as a
     capability.
3. The choice the dependent task holds open (dedicated table entry, reserved
   namespace capability, or nothing) therefore resolves to nothing. A capability
   whose only consumer would be a diagnostic read is not worth an ABI entry or a
   reserved name in v1.

If the effort instead chooses to keep it, the only shape the sources support is
**plugin-owned accounting reported to the host**: an instance-qualified
capability on the futures path that returns the plugin's own counters (the
`metis.future_stats` / `metis.future_counters` analogue). The **host-owned
request-id and future bookkeeping** reading must not be written as the counters
shape. That reading describes host state that section 13 already owns and that
needs no plugin-facing capability, so recording it as one would contradict
section 13.

Which reading holds: the sources support the plugin-owned accounting reading as
the origin of the word. The operator's host-owned request-id reading does not
hold as the meaning of `counters`, although it correctly identifies that
request-id allocation is host state and is already assigned to the daemon. Under
either reading, keeping the word in the contract adds nothing.

## Impact on dependents

- `configuration-and-host-context`: its D12 handoff
  (`docs/tasks/metis-plugins/tasks/configuration-and-host-context/task.md:165-167`)
  is unblocked. The counters sub-decision resolves to leave v1. Its remaining fog
  at `task.md:206` ("Whether the counters stay a single `metis.counters`
  capability or become part of the context") closes with "neither".
- `docs/plugin-contract.md`: section 14 item 7 (line 270) and section 15
  (line 294) need the amendment that removes `counters` from the control-plane
  lists. The contract already owes an amendment from the D12 handoff.
- The effort's ticket list loses the counters entry.
- No code change is implied. There is nothing to delete in `crates/`.

## Unresolved questions

- Whether the daemon wants a plugin-reported diagnostics or metrics capability
  that is distinct from `counters`. If it does, that is daemon and SDK work
  (post-run reporting), not a v1 contract entry, and it should be named
  explicitly as metrics rather than reviving the undefined word.
- The exact origin is reconstructed, not literal: no `metis.counters` string
  exists in the spike. The identification of the plugin counters with
  `metis.future_stats` rests on the deleted-surface row (item 6), the D7 list
  (item 5) and the code (items 9 to 11). Confidence is high for the occurrences
  and consumers, and medium-high for the origin reconstruction.
