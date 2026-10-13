# FINDINGS 4: round three, simplifying the ABI and the Rust trait contract

Throwaway prototype. Not product code, not a committed ABI, no CI.

Round three ran four lanes, one prototype copy each, against the north star the
operator set: get as close as possible to the plugin handing the host async tasks
that the host launches and awaits on its own runtime, and prefer deleting surface
over adding it.

Every headline number below was re-run by the parent against the lane copies. The
logs are committed under `round-3/verification/`. The lane copies under `/tmp`
are ephemeral; the lane findings and their evidence are copied into `round-3/`.

## Status

Four of four lanes completed. **Nothing is folded into the prototype yet**, and
that is deliberate: the operator asked for an approval round on the final FFI/ABI
and trait shape before the prototype takes it. The proposal to approve is
`ROUND3-CONTRACT-PROPOSAL.md` beside this file; it was approved as
`docs/plugin-contract.md`, and the implementation ticket is
`docs/tasks/metis-foundations/tickets/plugin-contract-v1`.

## Answers at the top

- **shape:** a futures-only v2 table plus a single capability-name-routed `call`
  method is smaller, and the routed shape is strictly easier to box than the
  landed four-arm trait. With the legacy callback path compiled out the suite
  stays green and the release cdylib drops 40 percent.
- **reclaim:** an abandoned blocking poll can be reclaimed exactly once when a
  late poll returns, and the unload rule should be quarantine (rule b), sharing
  one field with the leak accounting and deleting the fixed 900 ms wait.
- **ownership:** the seam is implementable from C. A hand-written C plugin drove
  the ready path, a host-answered timer, cancel, and a drop with the reply in
  flight, with a zero-live ledger read back after unload, and it surfaced five
  rules the contract never stated.
- **async-ffi:** drop the crate. A destructor panic inside a plugin future aborts
  the daemon with SIGABRT, including for a plugin that returned `Ok`, and no
  `catch_unwind` outside the crate can contain it.

## Verified in the parent

Re-run from `/home/patric/Projects/metis-agent` against each lane copy, logs in
`round-3/verification/`:

| Lane | Configuration | Checks | Failures | Exit |
| --- | --- | --- | --- | --- |
| shape | ON, debug | 95 | 0 | 0 |
| shape | ON, release | 95 | 0 | 0 |
| shape | OFF (`--no-default-features`), debug | 20 | 0 | 0 |
| shape | OFF, release | 20 | 0 | 0 |
| reclaim | debug | 101 | 0 | 0 |
| reclaim | release | 101 | 0 | 0 |
| ownership | baseline host, release | 92 | 0 | 0 |
| ownership | C-plugin probe, release, gcc `.so` | 14 | 0 | 0 |
| async-ffi | debug | 100 | 0 | 0 |
| async-ffi | release | 100 | 0 | 0 |

Baseline before this round was 92 checks, 0 failures, exit 0. The repository
prototype is untouched: `git status --porcelain` was empty after all four lanes.

## What each lane actually established

### shape: the deleted surface and the v2 sketch

The lane added a `legacy-callbacks` feature, compiled it out, and measured what
becomes dead. Reproduced in the parent at 20 checks in the OFF configuration
against 95 in ON.

Deleted when the futures path is the only call path:

- `invoke` (the work path), the `complete` host callback and its
  `MetisCompleteFn` type, and the landed `HostAdapter` slot, waker, `Await` and
  `Completed` machinery.
- The primitives-side `Executor`/`Task`/`SpawnRefused` seam, the SDK's
  `PoolExecutor`, `IdlePolicy` and `ThreadPerCallExecutor`, and 34 gate sites in
  the cdylib covering per-request tokens, job plans, worker bodies and executor
  management.
- 13 legacy capability descriptors and 75 legacy host checks.

Measured size effect, same target directory: release cdylib 1,554,520 to 924,328
bytes (minus 40 percent), debug cdylib 64,953,560 to 12,803,920 (minus 80
percent), release host 7,585,288 to 7,072,328 (minus 7 percent). The dependency
closure lost the `proto-registry` and `proto-sdk` edges.

Kept: `service_complete`, `service_request`, `progress`, `emit_event`, `log`,
`create_instance`, `destroy_instance`, `list_capabilities`, `cancel`,
`release_buffer`, `shutdown`, and the whole `MetisFutureHandle`,
`MetisFutureVtable`, `MetisWaker` and `MetisWakerVtable` set.

The object-safety probe is a compiler verdict, not a claim: as a return-position
`impl Trait` the routed trait is not `dyn` compatible (`E0038`), exactly like the
four-arm trait (`E0191` on the four associated types); boxed, the routed shape
compiles with no diagnostics while the four-arm shape does not get that for free.
The full sketch is in `round-3/shape/FINDINGS-shape.md`.

Launch and await, release, ON: ready-on-first-poll 947 to 1288 ns per call
against the FINDINGS3 864/826 ns reference, full path p50 6893 to 8165 ns against
the same-run direct path p50 5200 to 5891 ns, so the naive launch-and-await
bridge costs roughly 1.7 to 2.2 us per call. Eight concurrent plugin futures
completed, plugin counters `threads_spawned` delta 0 and `poll_threads` 1.

### reclaim: reclaim once, quarantine the rest

Watchdog deadline 150 ms; the abandoned poll returned after a 1200 ms plugin-side
hold. Before reclaim: `live_futures = 1`, `live_waker_clones = 1`. After: both
zero, a 24-byte late ready buffer released, `blocking_polls_started/returned =
1/1`, double reclaim refused as `AlreadyReclaimed`, poll after reclaim
`internal`, drop after reclaim refused. Whole-run teardown: `futures_created ==
futures_dropped == 62021`, `waker_clones == waker_releases == 46031`, zero leaks.
ABI delta: zero, `diff -rq` against the repository `crates/proto-abi` is empty.

Unload ledger, reproduced in the parent:

```
["unload refused: 1 abandoned poll(s) not yet reclaimed, a thread may be inside plugin code",
 "unload allowed: no thread inside plugin code",
 "shutdown returned", "future host dropped", "library unloaded"]
```

The lane recommends rule (b), quarantine, with `abandoned_active` serving as both
the leak counter and the quarantine predicate: new futures are refused while it
is nonzero, the mapping stays resident, and unload is allowed only when it
clears. Rule (a) keeps routing into the stuck plugin; rule (c) contradicts the
decided "host stays usable" answer and abandons the reclaim.

Added host state: one `AtomicUsize` on the driver, two driver fields, three
per-`HostFuture` fields, and one `ReclaimSlot` heap signal per watchdogged poll.
Nothing on the wire.

The parent intervention is part of the record: the lane's first teardown check
read plugin counters after `shutdown` had `Box::from_raw`'d the instance, so the
run wedged after `== teardown ==` with all threads futex-blocked until the parent
killed it. Root cause and fix are in the lane findings, and a new check now
covers it: a routed call after shutdown is refused with a typed error instead of
wedging on a destroyed handle.

### ownership: a real C implementer

`c-ownership/c_plugin.c` is 1072 lines (888 code lines) and compiles under
`-std=c11 -Wall -Wextra -Werror` with gcc 16.2.0 and clang 21.1.8, zero
diagnostics, exactly one defined dynamic symbol (`metis_plugin_entry_v1`) and
only libc plus pthread undefined. It implements the plugin table, `invoke_future`,
`poll`, `drop` and the waker vtable by hand, answers a capability end to end, and
answers a host timer through `service_complete`.

The ledger read back after the library unloaded, reproduced in the parent:

```
allocs=12 frees=12 live=0 double_frees=0 unknown_releases=0
waker_clones=3 waker_releases=3 futures_created=10 futures_freed=10
```

Tables seen from C: `table_size=88`, `create_host_size=48` (landed, no
`service_request`), `service_host_size=56`. The implementer must have 690 code
lines, of which 404 are the future object.

Five rules the contract never stated, each a contract delta: the payload envelope
byte, the one-call request lifetime, the borrowed-but-by-value waker, two
differently sized host tables, and `release_buffer` being reentrant inside
`service_request`. The lane also found that the landed Rust `service_complete`
frees a plugin-owned answer, which contradicts the ownership clause.

### async-ffi: the abort, verified

Reproduced in the parent from `verification/verify-ffi-release.txt`:

| Case | poll | Result |
| --- | --- | --- |
| `async-ffi`, pending future | `Pending` | killed by signal 6 (SIGABRT) |
| `async-ffi`, future that answered `Ok` | `Ok` | killed by signal 6 (SIGABRT) |
| `async-ffi`, same drop on a worker inside `catch_unwind` | `Ok` | killed by signal 6 (SIGABRT) |
| hand-written vtable, same panic | `Pending` | exit 0, contained |
| hand-rolled async object, pending | `Pending` | exit 0, contained |
| hand-rolled async object, after `Ok` | `Ok` | exit 0, contained |

stderr: `async-ffi: abort due to panic across the FFI boundary in Future::drop`.
The plugin shim's own `catch_unwind` is already in that path, so the abort is not
containable from outside the crate. A panic in `poll` through the crate is
contained, so the asymmetry is only in `drop`.

Comparison: 271 lines (213 code, 13 `unsafe`) for `async-ffi` against 172 (133, 7)
for the hand-rolled helper and 309 (244, 17) for the hand-written vtable. cdylib
1,139,912 against 1,117,184 bytes (2.0 percent, crate-attributed 25 symbols and
2,725 bytes of text, no new `NEEDED`). Ready path 899/902/738 against 880/885/584
against 825/818/593 ns over three runs, so the crate does not win per call.

## New rules the round produced

1. A destructor panic inside a plugin future must be contained by the SDK's own
   future object, so `async-ffi` goes and the SDK ships the boxed-future helper.
2. `abandoned_active` is one field with two jobs: leak accounting and the
   quarantine predicate that gates both new futures and unload.
3. The payload envelope, the request lifetime, the waker value semantics, the two
   host table sizes and the `release_buffer` reentrancy rule are contract text,
   not folklore. They belong in the C header and in the ownership clause.
4. `MetisFutureVtable.size` and `MetisWakerVtable.size` are written and never
   read, so a wrong-size vtable is undetectable: either read them or delete them.

## Not folded, not proven

- Nothing is folded yet; the prototype still carries the landed append-only
  tables and the four-arm trait. Folding waits on the approval round.
- Reclaim runs on the caller through `&mut self`; a real daemon needs a reaper
  thread, which is designed but not built.
- A poll that never returns leaves `abandoned_active` nonzero forever: plugin
  quarantined, mapping resident, one thread and one `ReclaimSlot` leaked. Safe,
  not fixed.
- No panic-in-drop-during-reclaim, terminal-error late poll, multi-reaper, Miri
  or ASan run, and no real mid-run `dlclose`.
- The plugin-side `invoke` still dereferences a freed instance; the adapter guard
  covers the routed path only.
- The C plugin deliberately never frees the loaded instance struct, so one
  instance leaks at unload outside the buffer ledger.
- No C implementer was built for the simplified v2 table in this round. The C
  evidence covers today's table, not the proposed one.
- The host timer is still one thread per driver and serializes `service_request`.
- The five unstated ownership rules are written up in the lane findings but the
  clause itself is not yet contract text.

## Files

- `round-3/ROUND.md` and `round-3/LANE-*.md`: the round and lane briefs.
- `round-3/shape/`, `round-3/reclaim/`, `round-3/ownership/`,
  `round-3/asyncffi/`: lane findings, evidence, and the C plugin source.
- `round-3/verification/`: the parent's own re-runs, one log per configuration.
- `ROUND3-CONTRACT-PROPOSAL.md`: the proposal record behind the approved
  contract, kept for provenance.
