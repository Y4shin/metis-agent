# FINDINGS reclaim: reclaiming an abandoned blocking poll, and the unload rule

Throwaway prototype. Not product code. Lane `reclaim` of round 3, answering open
items O1 and O2 of `docs/plugin-registry-handoff.md`. Work happened only inside
`/tmp/metis-plugin-registry-round-3/reclaim`. Nothing in the repository, another
lane copy, or any other `/tmp/metis-plugin-registry-*` path was written. Nothing
was deleted. Nothing was committed.

## 1. Status: COMPLETE

Read this first: the reclaim is proven **in this prototype only**, on one host
thread that calls `reclaim_abandoned(&mut self, deadline)` after a watchdog
abandon. It is not proven safe under a real daemon where a reaper thread, not the
caller, owns the handle. The complete list is in "Unproven".

Check counts and exit codes, both measured in this copy after every change:

| Build | Checks | Failures | Exit |
| --- | --- | --- | --- |
| debug | **101** | **0** | 0 |
| release | **101** | **0** | 0 |

Baseline before this lane: 92 checks, 0 failures, exit 0, in debug and release.

The earlier run that wedged (see section 4) is recorded from its markers, not
from memory. It is not described as verified.

## 2. Answers at the top

- **How does the host reclaim an abandoned blocking poll?** With a per-poll
  completion signal. The watchdog path now hands the poll thread an
  `Arc<ReclaimSlot>`; when the late poll returns, the thread publishes the status
  and buffer into the slot. The host's `reclaim_abandoned(&mut self, deadline)`
  waits on that slot with a bounded deadline, releases the late ready buffer
  through the plugin's `release_buffer`, clears the in-progress flag, and calls
  the plugin's `drop` exactly once. Proven once-only, with leak counters going
  1 -> 0 for both the ctx box and the stored waker clone in one run.
- **What is the unload rule?** Rule (b), quarantine. `abandoned_active` (the
  count of abandoned polls not yet reclaimed) is the quarantine predicate. While
  it is nonzero: the host refuses to unload and keeps the mapping, quarantines
  the plugin, and `invoke_future` refuses new work with `FutureError::Quarantined`.
  When it falls back to zero (reclaim succeeded), the quarantine clears
  automatically and only then is the unload allowed. The old fixed 900 ms sleep
  is gone.

## 3. What was actually driven

### Exact commands

```sh
cd /home/patric/Projects/metis-agent
devenv shell -- cargo build         --manifest-path /tmp/metis-plugin-registry-round-3/reclaim/Cargo.toml
devenv shell -- cargo build --release --manifest-path /tmp/metis-plugin-registry-round-3/reclaim/Cargo.toml
timeout 180 devenv shell -- cargo run -q           --manifest-path /tmp/metis-plugin-registry-round-3/reclaim/Cargo.toml -p proto-host
timeout 180 devenv shell -- cargo run -q --release --manifest-path /tmp/metis-plugin-registry-round-3/reclaim/Cargo.toml -p proto-host
devenv shell -- cargo test             --manifest-path /tmp/metis-plugin-registry-round-3/reclaim/Cargo.toml
```

The cdylib is built before the host in every run. The `timeout 180` wrapper is
deliberate: after the wedge in section 4 the lane may never wedge again without
leaving a bounded, diagnosable exit.

- `cargo run` release: `101 checks, 0 failures`, exit 0.
- `cargo run` debug: `101 checks, 0 failures`, exit 0.
- `cargo test`: 3 `proto-abi` layout tests plus 2 host ordering-ledger tests, all
  pass.

Output kept: `/tmp/reclaim-debug-{1..6}.txt`, `/tmp/reclaim-debug-cargorun.txt`,
`/tmp/reclaim-release-{1,2,3}.txt`, `/tmp/reclaim-release-cargorun.txt`.

### The reclaim, one line per driven check (names quoted from the run)

Baseline hazard 8 changed from "detected and reported" to "detected and
reclaimed once": it now reclaims the blocker instead of sleeping 900 ms.

New lane checks, all `PASS` in debug and release:

1. `reclaim 1`: the watchdog abandons the blocking poll while the plugin is still
   inside its own code. Watchdog deadline 150 ms; abandoned after 150 ms; the
   hold sleeps 1200 ms, and the plugin's own `blocking_in_code` flag read `true`
   through its counter capability at that moment.
2. `reclaim 2`: before reclaim one ctx box and one stored waker clone are leaked
   by design. Counters at that instant: `live_futures = 1`,
   `live_waker_clones = 1`.
3. `reclaim 3`: the host refuses to unload, quarantines the plugin, and routes no
   new call to it. `try_unload -> Refused`, `is_quarantined() == true`,
   `mapping_alive() == true`, and a fresh `invoke_future` returned
   `FutureError::Quarantined`.
4. `reclaim 4`: a reclaim attempted while the poll is still in progress is
   recorded and deferred, never a use-after-free. The early call with a 50 ms
   deadline returned `StillInProgress`, recorded the violation, and left the
   future undropped.
5. `reclaim 5`: the late poll returns, the host reclaims once, and a double
   reclaim, a poll after reclaim, and a drop after reclaim are refused and
   recorded. Reclaim released a 24-byte late ready buffer, the second reclaim
   returned `AlreadyReclaimed`, `poll()` after reclaim returned `internal`, and
   `drop_future()` after reclaim returned
   `Err(Protocol("double drop of future ... refused"))`. Plugin counters
   `blocking_polls_started = 1`, `blocking_polls_returned = 1`.
6. `reclaim 6`: after reclaim zero ctx boxes and zero waker clones are leaked.
   In the same run as check 2: `live_futures 1 -> 0`, `live_waker_clones 1 -> 0`,
   host `live_wakers = 0`.
7. `reclaim 7`: the quarantine clears, the same loaded mapping drives a fresh
   call, and only then is the unload allowed. `is_quarantined() == false`,
   fresh future returned `ok`, `try_unload -> Allowed`, `mapping_alive() == false`.
8. Whole-run check: `zero leaked ctx boxes and zero leaked waker clones across
   the whole run`. At that point, after the seam benchmarks, the plugin reported
   `futures_created == futures_dropped == 62021` and
   `waker_clones == waker_releases == 46029`.
9. Post-shutdown guard: `a routed call after shutdown is refused with a typed
   error, not a wedge on a destroyed handle`.

### The unload rule, as an ordering ledger

The teardown ledger from the release run:

```
["unload refused: 1 abandoned poll(s) not yet reclaimed, a thread may be inside plugin code",
 "unload allowed: no thread inside plugin code",
 "shutdown returned", "future host dropped", "library unloaded"]
```

The refusal is recorded while the blocking thread is inside plugin code; the
allow follows only after the reclaim lowered `abandoned_active` to zero.

### What reclaim costs in added host-side state

This matters against the north star (smaller surface), so it is counted exactly.

- **C ABI: zero.** `crates/proto-abi` is byte-identical to the repository copy
  (`diff -rq` empty). No new ABI type, table entry, function-pointer alias, or
  `size` change. Reclaim is entirely host-side.
- **One shared driver field:** `FutureInner.abandoned_active: AtomicUsize`. This
  one field is both the leak counter and the quarantine predicate.
- **Two driver fields:** `FutureHost.mapping_alive: AtomicBool` and
  `FutureHost.unload_refusals: AtomicUsize`.
- **Three fields per `HostFuture`:** `inner: Arc<FutureInner>` (a shared ref, not
  a copy), `reclaimed: bool`, and `reclaim_slot: Option<Arc<ReclaimSlot>>` (8
  bytes, `None` until the watchdog fires).
- **One heap `ReclaimSlot` per watchdogged poll only.** `ReclaimSlot` is an
  `Arc` over `Mutex<Option<LateCompletion>>` plus a `Condvar`; `LateCompletion`
  is a status plus one `MetisBuffer`. It is created inside `poll_bounded`, not on
  the ordinary poll path (`poll_once_inner` / `poll_to_ready`), so the hot path
  allocates nothing new. `PollJob` swapped its `mpsc::Sender` for an
  `Arc<ReclaimSlot>`, a net zero field count.
- **Plugin side (prototype scaffolding, not contract):** three static counters,
  one static `AtomicBool`, `HandState.blocking_started: bool`, a `BlockingHold`
  mode, and the `future.blocking_hold` capability. None of this is needed by a
  real plugin; it exists so the host can observe "a thread is inside plugin
  code" and count the leak. A real plugin needs nothing from this lane.

So: yes, reclaim needs state per future (`reclaimed`, an `Option<Arc>` to the
per-poll signal, and a shared `Arc`), plus one heap signal per watchdogged poll.
It adds nothing to the wire protocol.

## 4. The teardown wedge (recorded, then fixed): a use-after-free, not a missing deadline

The first version of this lane's teardown leak check read the plugin counters
through the legacy `AbiPlugin::call_pending` path placed *after*
`abi.shutdown()` had already run. It wedged.

Marker evidence (`/tmp/reclaim-debug-2.txt`, kept):

- Output stops at the section header `== teardown` (file ends at line 216); there
  is no `N checks` summary line, no teardown `NOTE`, and no `library unloaded`.
- The parent observed the process (PID 573525) still alive after about 4.5
  minutes, all 20 threads futex-blocked, including the main thread, and killed it
  externally (SIGTERM, exit 143).

Root cause, from reading the code (not from a stack trace):

- `shutdown` ends with `destroy_instance(instance)`, and `destroy_instance` does
  `Box::from_raw(instance as *mut Instance)` and drops the `Instance`.
- `invoke` reads `&*(instance as *const Instance)` with no check against the
  `released_instances` set, so any later routed call through the same pointer
  dereferences freed memory. The freed `InstanceState` contains several
  `Mutex`es, and blocking on a futex in freed or reused memory is exactly the
  observed futex stall.

Fix (driven):

- The teardown leak check moved to *before* `shutdown`, where the same stats call
  is legal and already used by the round-trip bench.
- `AbiPlugin` already had a `shut_down: AtomicBool`; it is now checked by a new
  private `ensure_live`, called at the top of `call_pending`, `invoke_bounded`,
  `begin`, `fire`, and `invoke_on`. A post-shutdown routed call now returns a
  typed `PluginError::Internal` immediately instead of touching the freed handle.
  The new check `a routed call after shutdown is refused with a typed error, not
  a wedge on a destroyed handle` is what proves it.

This is a legacy-adapter hazard (call O7 shaped), not a reclaim-design bug. The
reclaim waits (`ReclaimSlot::wait`, `HostTask::wait`) all carry caller-supplied
deadlines and print a violation instead of blocking.

## 5. Unproven

Everything here was **not** driven or **not** measured, and is not described as
verified.

- **Reclaim under a real daemon.** The prototype reclaims from the same thread
  that called `poll_bounded`, via `reclaim_abandoned(&mut self, ...)`. A real host
  cannot block a caller on a late poll; it needs a reaper that owns abandoned
  handles and polls their completion signals. That shape is designed here but not
  built or measured.
- **A poll that never returns.** `abandoned_active` stays nonzero forever, so the
  plugin stays quarantined and the mapping stays resident for the process
  lifetime. Safe, but a thread and a `ReclaimSlot` leak. Not fixed.
- **A panic in the plugin's `drop` during reclaim.** `reclaim_abandoned` handles
  the returned error path but that hazard was not driven.
- **A terminal late poll (an error status, not `Ok`/`Pending`).** The slot stores
  whatever status the late poll returns; only a late `Ok` was driven.
- **Exactly-once under concurrency.** The guard is a `bool` plus the slot's
  `take`, both reached through `&mut self`; two reclaimers are excluded by the
  borrow, not by an atomic. No multi-reaper race was driven.
- **Miri, ASAN, or loom.** The `unsafe impl Send/Sync for ReclaimSlot` and the
  pointer transfer from the poll thread to the host thread are argued in
  comments, not machine-checked.
- **A real `dlclose`.** "Unload allowed" is a host gate plus a ledger marker; the
  prototype still unloads by dropping the library at teardown. Mid-run unload was
  not performed.
- **The exact wedge frame.** The use-after-free root cause is inferred from code
  reading. The blocking frame was not captured with a debugger or a backtrace.
- **The plugin-side `invoke` UAF itself.** `invoke` still dereferences `instance`
  without a `released_instances` check. The adapter guard closes the routed-call
  path the harness uses, not every raw-table caller.
- **The `stale_wake` counter fix.** `SendWaker::wake` now counts its release
  through `release_waker`, which balances the plugin's clone/release counters. It
  is a diagnostic fix; it does not change the reclaim result.

## 6. Recommendation

Adopt the reclaim design for the watchdog path and adopt rule (b), quarantine,
as the host's unload rule. The per-poll completion signal is small (no ABI
change, one `AtomicUsize` on the driver, three fields per future, one heap slot
per abandoned poll) and it turns the "leak the ctx box and one waker clone by
design" answer into an observable, once-only reclaim with a zero-leak teardown.
The quarantine predicate is the same counter that proves the reclaim, so the
unload rule and the leak accounting share one field instead of adding two
mechanisms. The old fixed 900 ms wait is deleted: the host now decides from the
completion signal, and a poll that never returns simply keeps the plugin
quarantined and the mapping resident, which is safe and visible.

Against the other two rules. Rule (a), refuse to unload and keep the mapping, is
necessary but not sufficient: it never stops the registry from routing new work
into the stuck plugin, and it leaves the resident-forever state undiagnosed.
Rule (c), treat a blocking poll as fatal, contradicts the north star and the
already-decided A5 ("a blocking poll is detected, the host stays usable"): it
turns one plugin's protocol violation into a whole-daemon event and gives up the
reclaim entirely. Rule (b) is (a) plus a routing gate that clears
deterministically when the last abandoned poll is reclaimed.

Surface under the north star:

- **Delete:** the fixed `BLOCKING_POLL_WAIT_MS` sleep (done), the plain
  `mpsc` completion channel on the bounded-poll path (replaced by the one-shot
  `ReclaimSlot`), and `diagnostic_counters` (never written; the legacy stats path
  reads the counters).
- **Keep:** the single `abandoned_active` counter as the quarantine predicate,
  `reclaim_abandoned` as the one reclaim entry, and the `ReclaimSlot` on the
  watchdog path only.
- **Do not add:** any ABI type, table entry, or host-table slot for reclaim. The
  plugin learns nothing new. The only plugin change a real implementer needs is
  the behaviour it already owes the spec: `poll` must not block.
- **Open operator decision:** where the unload rule text lives. `SEAM-SPEC.md`
  rule 7 says the library is unloaded only after every future is dropped and no
  waker clone remains, but it does not say what the host does when a poll never
  returns. The proposed clause is: "While any abandoned poll has not returned,
  the host quarantines the plugin and keeps the mapping; new futures are refused;
  the unload is allowed only after every abandoned poll's completion signal has
  fired and the future has been dropped exactly once." The second open decision
  is whether that reclaim runs on the caller or on a dedicated reaper thread.

## 7. Contract deltas

Named exactly, per item.

- ABI types, added: none. (`MetisFutureHandle`, `MetisFutureVtable`,
  `MetisWaker`, `MetisWakerVtable`, `MetisBuffer`, `MetisStatus` unchanged.)
- Plugin table entries, added or changed: none. (`invoke_future`,
  `service_complete` unchanged.)
- Host table entries, added or changed: none. (`service_request` unchanged.)
- Trait items, added or changed: none. No Rust trait changed. The host-side
  `CheckSink` trait is unchanged; its impls just print more checks.
- Host-side items added (not contract, host implementation):
  `FutureInner.abandoned_active: AtomicUsize`;
  `FutureHost.mapping_alive: AtomicBool`; `FutureHost.unload_refusals: AtomicUsize`;
  `HostFuture.inner: Arc<FutureInner>`; `HostFuture.reclaimed: bool`;
  `HostFuture.reclaim_slot: Option<Arc<ReclaimSlot>>`; types `ReclaimSlot`,
  `LateCompletion`, enums `ReclaimOutcome`, `UnloadDecision`; methods
  `FutureHost::abandoned_active`, `is_quarantined`, `mapping_alive`,
  `unload_refusals`, `try_unload`; `HostFuture::reclaim_abandoned`;
  `FutureError::Quarantined`.
- Host-side items removed: the watchdog path's `mpsc::Sender`/`Receiver`
  completion channel and the `BLOCKING_POLL_WAIT_MS` fixed wait. `PollJob`'s
  `tx` field is replaced by `slot`.
- Plugin-side items added (prototype diagnostics, not contract): the
  `future.blocking_hold` capability and `HandMode::BlockingHold`; counters
  `blocking_in_code`, `blocking_polls_started`, `blocking_polls_returned`,
  `live_futures`, `live_waker_clones` in `stats_json`; `HandState.blocking_started`;
  the `BLOCKING_HOLD_MS` constant. `SendWaker::wake` now goes through
  `release_waker`.
- Adapter-side items added (host hazard fix, not contract):
  `AbiPlugin::ensure_live`, called from `call_pending`, `invoke_bounded`,
  `begin`, `fire`, `invoke_on`.
- Open operator decisions (recorded, continued): the unload-rule clause for
  `SEAM-SPEC.md` rule 7 quoted in section 6; and caller-driven reclaim versus a
  reaper thread.

## 8. Files written inside this copy

- `FINDINGS-reclaim.md` (this file)
- `crates/proto-host/src/future_driver.rs`
- `crates/proto-host/src/abi_adapter.rs`
- `crates/proto-host/src/main.rs`
- `crates/proto-plugin-cdylib/src/future.rs`

No file in the repository, another lane copy, or any other
`/tmp/metis-plugin-registry-*` path was written. Nothing was deleted. Nothing was
committed. `crates/proto-abi` is untouched, which is the evidence for the zero
ABI delta.
