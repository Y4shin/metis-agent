# FINDINGS: lane ownership (O3, O10)

Throwaway prototype lane. Copy: `/tmp/metis-plugin-registry-round-3/ownership`.
Question: is the written ownership rule for the two service entries enough for an
implementer that is not Rust, and can such an implementer drive the seam end to
end?

## Status

COMPLETE. Read this first: the deliverable is a hand-written C `.so` driven end to
end by the prototype host, plus the ownership clause below. Everything the C
implementer needed and could not find in the clause or the ABI header is in
`Friction list`, and every one of those is a contract delta for the approval
round. Anything not driven is in `Unproven`; nothing there is described as
verified.

Check counts, all re-run on this copy at the end of the lane:

| Run | Checks | Failures | Exit |
| --- | --- | --- | --- |
| baseline harness, debug (`baseline-debug.log`) | 92 | 0 | 0 |
| baseline harness, release (`baseline-release.log`) | 92 | 0 | 0 |
| ownership probe, debug, gcc-built `.so` (`c-probe-debug.log`) | 14 | 0 | 0 |
| ownership probe, release, gcc-built `.so` (`c-probe-release.log`) | 14 | 0 | 0 |
| ownership probe, release, clang-built `.so` (`c-probe-release-clang.log`) | 14 | 0 | 0 |

The baseline is unchanged at 92/0 in both profiles. The C lane adds 14 checks and
does not touch the harness: it is a second binary (`--bin c-ownership-probe`),
and the package now pins `default-run = "proto-host"` so
`cargo run -p proto-host` still runs the 92-check harness.

## Answers at the top

1. **Is the written ownership rule enough for a non-Rust implementer?** No, not
   as written. The rule as handed to me (plugin-owned request released by the
   host, host-owned reply borrowed for the call) is *correct* for the paths I
   drove, but it is not sufficient: the envelope byte, the request payload's
   lifetime, the borrow-versus-own status of the waker passed to `poll`, the
   existence of two differently sized host tables, and the reentrancy of
   `release_buffer` inside `service_request` are all unstated and all
   load-bearing. With those five written down, a C implementer can follow the
   clause mechanically. See `Friction list`, items 1 to 5.
2. **Can such an implementer drive the seam end to end?** Yes. A 1072-line C file
   compiled with `gcc -std=c11 -Wall -Wextra -Werror` (clean, and clean under
   clang 21.1.8 with the same flags) implements the entry point, the whole table,
   `invoke_future`, `poll`, `drop`, the waker-vtable calls, a ready capability and
   a `service_request`/`service_complete` timer round trip, and drives it on the
   real host path: 14/14 checks pass in debug and release, with the C side's own
   ledger at `allocs == frees == 12, live == 0, double_frees == 0` after the
   library has been unloaded.

## The ownership clause

Contract text for the two service entries, the ready buffer, and the request
payload. `METIS_BUFFER_OWNER_PLUGIN` (2) means the plugin allocated it and the
host releases it through the plugin's `release_buffer`; `METIS_BUFFER_OWNER_HOST`
(1) means the host allocated it and the plugin must never release it;
`METIS_BUFFER_OWNER_NONE` (0) with a null `ptr` is the empty buffer. The `owner`
field is load bearing on every path.

1. **`invoke_future` request payload (host to plugin).** The host allocates and
   sets `owner = HOST`. It is borrowed for the call: the host frees it when
   `invoke_future` returns. The plugin must copy anything it needs during the
   call, and must never call `release_buffer` on it. The host table pointer
   passed to `invoke_future` lives for as long as the plugin's futures do, so a
   later `poll` may call `service_request` through it.
2. **`out_value` on the ready path (plugin to host).** The plugin allocates,
   sets `owner = PLUGIN` and `len` to the enveloped body length, and writes it
   through `out_value` on the poll that returns `Ok`. The host copies `len` bytes
   and calls the plugin's `release_buffer(buffer)` before that poll returns. The
   pointer stops being valid when `release_buffer` returns. The host never frees
   it itself.
3. **`service_request` payload (plugin to host).** The plugin allocates and sets
   `owner = PLUGIN`. The host copies the body and calls the plugin's
   `release_buffer(buffer)` before `service_request` returns, so the pointer
   stops being valid when `release_buffer` returns. The plugin must not free it
   itself. The host re-enters plugin code (`release_buffer`) inside this call.
4. **`service_complete` payload (host to plugin).** The host allocates and sets
   `owner = HOST`, or passes the empty buffer when the answer carries no data
   (the prototype's timer service always passes the empty buffer). It is borrowed
   for the call: valid until `service_complete` returns, never to be released by
   the plugin, and the plugin must copy it if it needs it later. `service_complete`
   may be called from any thread, including while the plugin is inside `poll`.
5. **Drop while a service reply is in flight.** The host must not call `drop`
   while a poll is in progress (it observes a poll return first); the plugin may
   assume `drop` does not overlap `poll` for the same future. On `drop` the
   plugin releases every plugin-owned buffer the future still holds and every
   waker clone it kept, using the waker vtable's `drop` (or `wake`; both
   consume). A `service_complete` that arrives after the drop for the same
   request id finds no live future: the conforming plugin wakes nobody, records
   the late completion, returns `Ok`, and touches no memory the host may have
   freed. Waking a waker whose future is gone is the non-conforming case the host
   detects as a protocol violation and must not crash on. The plugin's context
   must live until the last of (drop, service reply) has happened and not a
   moment longer; the C implementer needed a refcount plus a per-request slot
   table for exactly this, and the host must not free the future's memory before
   both.

## What was actually driven

Exact commands, from `/home/patric/Projects/metis-agent` (the devenv shell is
required for `gcc`, `clang` and `cargo`):

```sh
# 1. the hand-written C plugin, both compilers, the prototype's own flags
devenv shell -- gcc -std=c11 -Wall -Wextra -Werror -O2 -fPIC -shared -pthread \
    -I /tmp/metis-plugin-registry-round-3/ownership/crates/proto-abi/include \
    -o /tmp/metis-plugin-registry-round-3/ownership/c-ownership/libmetis_c_plugin.so \
    /tmp/metis-plugin-registry-round-3/ownership/c-ownership/c_plugin.c
devenv shell -- clang -std=c11 -Wall -Wextra -Werror -O2 -fPIC -shared -pthread \
    -I /tmp/metis-plugin-registry-round-3/ownership/crates/proto-abi/include \
    -o /tmp/metis-plugin-registry-round-3/ownership/c-ownership/libmetis_c_plugin_clang.so \
    /tmp/metis-plugin-registry-round-3/ownership/c-ownership/c_plugin.c

# 2. build the whole workspace first (cargo run does not rebuild the cdylib member)
devenv shell -- cargo build --release \
    --manifest-path /tmp/metis-plugin-registry-round-3/ownership/Cargo.toml

# 3. the 92-check harness, still the default binary
devenv shell -- cargo run -q --release \
    --manifest-path /tmp/metis-plugin-registry-round-3/ownership/Cargo.toml -p proto-host

# 4. the ownership probe, driving the C .so on the real host path
C_PLUGIN_LIB=/tmp/metis-plugin-registry-round-3/ownership/c-ownership/libmetis_c_plugin.so \
    devenv shell -- cargo run -q --release \
    --manifest-path /tmp/metis-plugin-registry-round-3/ownership/Cargo.toml \
    -p proto-host --bin c-ownership-probe
# (drop --release for the debug run; point C_PLUGIN_LIB at the clang .so for that run)
```

Roll-up: `bash /tmp/metis-plugin-registry-round-3/ownership/c-ownership/build.sh`
runs the two C compiles, the workspace build and the release probe in one go
(exit 0).

### The C `.so` on the real path

- Both compilers emitted the shared library with **no diagnostics** under
  `-std=c11 -Wall -Wextra -Werror`: gcc 16.2.0 and clang 21.1.8.
- `libmetis_c_plugin.so` is loaded by the prototype's own `libloading` loader
  (`AbiPlugin::load`), reached through the exported symbol
  `metis_plugin_entry_v1`, and driven with the prototype's real
  `FutureHost`/`HostFuture` seam, not a stand-in. No Rust glue in the plugin:
  `nm -D --defined-only libmetis_c_plugin.so` shows exactly one defined dynamic
  symbol, `metis_plugin_entry_v1`, and the undefined list is libc and pthread
  only (no libstd, no `async-ffi`, no Rust runtime).
- The C side's own view of the tables, read back through the seam:
  `table_size=88` (16-byte prefix + 9 function pointers), `create_host_size=48`
  (the landed host table, no `service_request`), `service_host_size=56` (the
  `invoke_future` table, with `service_request`).
- Ready path: the first poll returns `Ok` with a plugin-allocated buffer
  (`{"impl":"c","kind":"ready","x":7,"guard":1}`), the host reads it and releases
  it through the C `release_buffer`.
- Timer round trip: first poll `Pending` after the plugin issued
  `service_request("timer", {"delay_ms":0})` and cloned the host waker; the host's
  timer thread called the C `service_complete`; the clone's `wake` parked and
  resumed the host poll loop; the second poll returned
  `{"impl":"c","kind":"timer","x":9,"polls":2}`. Host counters:
  `service_requests=3`, host-side released service payloads `=3`.
- Cancel: the C `cancel` marked the request, woke the host through the clone, and
  the next poll returned `Cancelled`; the host dropped the future after that poll
  return.
- Drop with a reply in flight: a timer future dropped right after it armed; the
  C `drop` released the waker clone (host `live_wakers` back to 0) and the reply
  arriving 150 ms later found no slot, so the plugin counted a late completion,
  woke nobody, and the host recorded **no** stale wake and no violation.
- Instance lifecycle: a second `create_instance`, `destroy_instance` returning
  `Ok`, and a repeat destroy returning `NotFound`.

### Accounting, both sides

C counters, read through the seam after all the lane work, and again from the
plugin's ledger file written at `shutdown` and read after the library unloaded:

| Counter | mid-run (through the seam) | final (ledger, after unload) |
| --- | --- | --- |
| `allocs` | 11 | 12 |
| `frees` | 11 | 12 |
| `live` | 0 | 0 |
| `double_frees` | 0 | 0 |
| `unknown_releases` | 0 | 0 |
| `foreign_releases` (deliberate guard tests) | 2 (2) | 2 (2) |
| `ready_buffers` / `frees_ready` | 7 / 7 | 8 / 8 |
| `service_payloads` / `frees_service` | 3 / 3 | 3 / 3 |
| `legacy_buffers` / `frees_legacy` | 1 / 1 | 1 / 1 |
| `waker_clones` / `waker_releases` | 3 / 3 | 3 / 3 |
| `futures_created` / `futures_freed` | 10 / 9 (one in flight: the read's own) | 10 / 10 |
| `late_completions` | 2 | 2 |
| `service_roundtrips` | 3 | 3 |
| `cross_thread_completions` | 3 | 3 |
| `double_drops`, `alloc_failures`, `bad_envelope_in`, `live_overflow`, `service_request_failures` | 0 | 0 |

Two-sided equality: the host's released service payload count (`FutureHost::released()`
= 3) equals the C side's `frees_service` = 3, and the host recorded
`violations == 0` for the whole run. The C side's own guard, driven twice, hands
the host-owned request buffer to its own `release_buffer` and gets
`InvalidArgument` back (2 guard tests, 2 refusals): the buffer the host owns never
reaches the plugin's free path.

`foreign_releases == 2` is not a leak or a mistake: it is the deliberate
owned-buffer guard test. It is the only nonzero counter that is not a match
between the two sides, and it is counted separately as `guard_tests` so the two
numbers cannot drift apart silently.

Threading, measured: `cross_thread_completions == 3`, equal to the number of
service answers delivered (`host_owned_service_payloads == 3`). Every answer
arrived on the host's timer thread, never on the poll thread, so the C implementer
had to make its state thread safe. The counter compares `pthread_self()` of the
delivery against the thread that first polled.

### Friction list

Every place the C implementer had to know something the ownership clause (as
handed to me) or the ABI header did not state. Each is a contract delta.

1. **The payload envelope is invisible.** The header has no constant, type or
   comment for it. `out_value` bodies and `service_request` bodies must start with
   the byte `0x01` (version 1 in the low nibble, JSON codec bit clear). The
   implementer learned it from the host's reader (`metis_payload_body`). Failing
   it is silent: the host's ready path does
   `metis_payload_body(bytes).map(to_vec).unwrap_or_default()`, so a missing
   envelope yields an empty body with no error, and `service_request` with a
   missing envelope yields an empty body, which the host parses as `delay_ms = 0`.
2. **The request payload's lifetime is one call.** The host's `invoke_future`
   drops the backing `Vec` as soon as the call returns, so the request pointer is
   dangling for any later poll. The clause and the header say nothing; a plugin
   that stores the pointer reads freed memory on the next poll. The C implementer
   copies `x` and `delay_ms` during the call.
3. **The waker passed to `poll` is borrowed although it is passed by value.** In
   C, by value reads as ownership, and the header says `wake` and `drop` "consume
   the waker they are given". The host releases its own reference after `poll`
   returns (`Self::drop_waker(waker)` in `poll_once_inner`), so a plugin that
   consumed the passed handle would release the host's `Arc` a second time. The
   implementer had to learn from the Rust prototype that a clone is mandatory.
4. **There are two host tables with different sizes, and only one has
   `service_request`.** `create_instance` receives the landed 48-byte table (four
   callbacks); `invoke_future` receives the 56-byte table with
   `service_request`. The header declares only the larger struct, so reading
   `host->service_request` from the `create_instance` table is an out-of-bounds
   read. Nothing in the header or the clause says which table a callback may use.
   The presence rule for the C side is also hand-written: the header has no
   predicate, and the Rust helper `metis_host_api_has_service_request` is not in
   the C header, so the implementer wrote
   `size >= offsetof(..., service_request) + sizeof(fnptr)` by hand. (The
   versioning lane found the same asymmetry from the other side:
   `versioning.rs` checks that a landed `HostAdapter` table reports
   `service_request` absent, `host_size == 48`.)
5. **`release_buffer` is reentered inside `service_request`.** The host frees the
   plugin's request payload synchronously, from inside the plugin's own
   `service_request` call. A plugin that holds a mutex across `service_request`
   and takes it in `release_buffer` deadlocks. Nothing states this. (Measured: the
   host's released count and the plugin's `frees_service` agree exactly, so the
   release really does happen inside the call.)
6. **The entry symbol name is not in the header.** The header has the typedef
   `MetisPluginEntryV1Fn` but no comment naming `metis_plugin_entry_v1`. The
   implementer took the literal from the Rust constant.
7. **`service_complete` runs on a thread that is not the poll thread.** Measured
   3/3. Only the waker's `wake` is documented as callable from any thread; the
   clause says nothing about `service_complete`'s thread, nor about its
   concurrency with `poll`.
8. **Nothing states whether `drop` is idempotent.** The spec says the host calls
   it exactly once; the header has no rule for a second call. The implementer
   returns `Ok` and counts it (`double_drops`, 0 in this run).
9. **Nothing states the plugin-side cancellation duties.** That `cancel` must
   resolve the future as `Cancelled` and wake the host task, and whether it
   retires the plugin's own record, is only in the Rust prototype, not in the
   clause or the header.
10. **No buffer size, alignment or slot-count rule.** The implementer chose
    `cap == len`, byte payloads on an 8-byte-aligned allocation, `ptr == NULL` for
    "no payload", a fixed 8-entry slot table, and a refusal (`Internal`) when the
    slot table is full. None of those is ABI text; a second implementer would
    choose differently and still be conformant.
11. **`clone` may not fail, and there is no way to say so.** The header's
    signature has no error path, so the implementer treats a null `vtable` as "no
    waker" and never calls `clone` on a null handle. Unstated.
12. **The future and waker vtables carry a `size` field that the host never
    reads.** Presence-by-size is the append-only rule for the plugin and host
    tables, and the host's `versioning.rs` uses those. For `MetisFutureVtable` and
    `MetisWakerVtable` the `size` field is written by both sides and read by
    neither, so a short or wrong-sized future vtable is undetectable. Either the
    host should check it, or the field should be deleted from both new vtables.
13. **The landed Rust plugin's `service_complete` frees a plugin-owned answer.**
    `crates/proto-plugin-cdylib/src/future.rs` has
    `if payload.owner == METIS_BUFFER_OWNER_PLUGIN { release_buffer(payload) }`.
    Under clause item 4 that is freeing the host's buffer; the C implementer only
    counts it (`plugin_owned_service_payloads == 0` in this run). The clause
    should be made explicit and that branch deleted, or the ABI should say a
    plugin-owned answer is legal and then define who frees it.

### How many lines of C the minimal implementer needed

Measured on `c-ownership/c_plugin.c` (1072 lines total):

| Part | Code lines (no blanks, no comments) |
| --- | --- |
| whole file | 888 |
| lane instrumentation (counters, accounting JSON, ledger writer) | 198 |
| **everything an implementer must have** | **690** |
| … the future object (structs, slots, refcount, `poll`, `drop`, both vtables, `invoke_future`, `service_complete`) | 404 |
| … the landed entries and the table (create/destroy/list/invoke/cancel/release/shutdown/entry) | 134 |
| … the plugin allocator plus `release_buffer` | 85 |
| … the request-body field reader | 34 |

So a conforming C plugin that answers two capabilities over the future seam cost
about 690 code lines here, and the single largest block is the future object
itself (404), not the ownership bookkeeping. A genuinely minimal single-ready-
capability C plugin, with no timer, no cancel, no counter tags and no landed-entry
bodies beyond stubs, is a hand estimate rather than a measurement: roughly 250 to
300 lines. The 690 is the measured number.

## Unproven

- **Not driven:** the legacy `invoke`/`Pending`/`complete` path *from C*. The C
  `invoke` returns `Unsupported` (and `legacy_invoke_calls == 0`), because this
  lane is about the future seam. A C plugin that answers through `complete`
  instead is untested.
- **Not driven:** the host callbacks `complete`, `progress`, `emit_event`, `log`
  from the C side. The C instance stores the `create_instance` host table but
  never calls it.
- **Not driven:** the IO half of `service_request`. Only `"timer"` is exercised;
  nothing C-side has asked for an IO service or received a non-empty
  `service_complete` payload, so item 4 of the clause is proven only for the empty
  (host-owned, `owner = NONE`) answer.
- **Not driven:** a non-`Ok` `service_complete` status from C.
- **Not driven:** multiple concurrent C futures on one instance. The lane drives
  one future at a time; the slot table, the refcounts and the mutex are therefore
  exercised for single-threaded polling plus one cross-thread answer, not for two
  polls in flight.
- **Not driven:** a C plugin that violates the clause (frees the request payload,
  frees the host's answer, consumes the borrowed waker, wakes after drop). The
  conforming path is proven; the host's detection of the non-conforming ones is
  lane 2's evidence with a Rust plugin, not mine.
- **Not measured:** the C plugin's per-call latency. The lane's outcome is
  correctness plus accounting, and no C-side timing was taken. The probe does not
  run the bench sections.
- **Not measured:** `destroy_instance` on the loaded instance (only on the second
  one), and what the host does when a plugin still holds a future at
  `destroy_instance`. The loaded instance's struct is deliberately never freed, so
  the `AbiPlugin` shutdown that follows does not touch freed memory; that is a
  small deliberate leak of one instance struct at unload (see residual risks).
- **Not measured:** a second implementer in another language (the header is C, so
  C is the cheapest cross-language proof; C++ or Zig are untested).
- `guard_tests == 2` covers only the two ready calls the probe makes; the guard is
  not exercised on the `service_request` or `service_complete` paths.

## Recommendation

Keep both service entries and the ownership rule, and write the clause as contract
text before any non-Rust implementer is expected to work from it. The seam is
genuinely implementable from C: a 1072-line hand-written plugin drives a ready
capability, a host-answered timer, cancellation, a drop with a reply in flight and
a clean unload, with a two-sided zero-leak, zero-double-free ledger
(`allocs == frees == 12, live == 0` after unload). The cost is not the ABI's size
but its unstated rules: five of them (the envelope byte, the one-call lifetime of
the request payload, the borrowed-but-by-value waker, the two differently sized
host tables, and the reentrant `release_buffer`) would each be found by a C
implementer only by reading the host's Rust source or by crashing. Under the north
star I would delete three things from the surface (`MetisFutureVtable.size`,
`MetisWakerVtable.size`, and the landed Rust plugin's `release_buffer` branch for
a plugin-owned `service_complete` payload), add exactly two things (the envelope
constant plus a presence predicate to the generated C header, and the ownership
clause as contract text), and keep every entry in both tables: nothing in the C
implementation showed a table entry that could be dropped, and the ready path, the
service request, the piecewise drop and the cancel token each earned their slot.
The one place the shape could genuinely shrink is the pair of `size` fields on the
two new vtables: they are written by both sides and read by neither, so they are
pure ceremony today.

### Contract deltas

ABI types and fields:

- **`MetisFutureVtable.size` (remove):** the host never reads it; presence is not
  decided by it anywhere. Keeping it invites a plugin to believe a shorter vtable
  is legal.
- **`MetisWakerVtable.size` (remove):** same, and it is host-side, so it is not
  even a compatibility lever for a plugin.
- **`MetisBuffer.owner` (change: make it checked on the ready path).** The ready
  path currently copies the body and skips the release when `owner != PLUGIN`,
  silently leaking the plugin's buffer; `FutureInner::release` counts a violation
  on the same mistake for `service_request`. Make both paths record a violation,
  or make `owner` redundant by defining the direction from the call's role.
- **New constant in the generated header (add):** the payload envelope byte and
  the `METIS_PAYLOAD_*` version/codec/reserved masks, plus a short comment. There
  is no way to write a conforming C plugin without them today.
- **New predicate in the generated header (add):** `metis_host_api_has_service_request(size)`
  and its plugin-table siblings, so the C side does not hand-write
  `offsetof(...) + sizeof(fnptr)`.
- **New comment-level rule on `MetisInvokeFutureFn` (add):** the request buffer is
  valid only for the call and the host frees it when the call returns.
- **New comment-level rule on `MetisServiceCompleteFn` (add):** the payload is
  host-owned and borrowed for the call, and the call may run on any thread,
  including concurrently with `poll`.
- **New comment-level rule on `MetisFuturePollFn` (add):** the `waker` argument is
  **borrowed**; the host releases its own reference after `poll` returns, so a
  plugin must `clone` to keep one.
- **Capability name for the entry point (add to the header):** state
  `metis_plugin_entry_v1` as a comment or a macro beside `MetisPluginEntryV1Fn`.

Table entries: no removals. `invoke_future` and `service_complete` on the plugin
side and `service_request` on the host side are all load bearing in the C
implementation; `cancel` and `release_buffer` are load bearing for the C
implementer too. No entry was left unused.

Trait items: none. This lane never touched the Rust trait side.

Open operator decision (unsettled by the written spec, recorded here rather than
blocking):

1. **Two host tables, or one merged table?** `create_instance` gets the landed
   48-byte table and `invoke_future` gets the 56-byte one. A C implementer must
   root the host table in one place; today the same logical host appears twice with
   different sizes, and only one carries `service_request`. Either the host should
   pass the widened table to `create_instance` as well, or the ABI should say
   explicitly that the `invoke_future` table is the one a future may use for
   services.
2. **Is a plugin-owned `service_complete` payload legal?** The landed Rust plugin
   accepts and frees one; the clause says the host owns the answer. Pick one and
   delete the other branch.

## Files written in this copy

- `FINDINGS-ownership.md` (this file)
- `ownership clause` is part of this file, not a separate document
- `c-ownership/c_plugin.c` (the C implementer, 1072 lines)
- `c-ownership/build.sh` (build and run in one command)
- `c-ownership/libmetis_c_plugin.so` (gcc 16.2.0)
- `c-ownership/libmetis_c_plugin_clang.so` (clang 21.1.8)
- `c-ownership/c-plugin-ledger.txt` (the plugin's own ledger, appended per run,
  nonce-tagged)
- `c-ownership/c-probe-marker.txt` (last check name on disk)
- `crates/proto-host/src/bin/c-ownership-probe.rs` (the 14-check probe)
- `crates/proto-host/Cargo.toml` (added `default-run = "proto-host"`)
- `baseline-debug.log`, `baseline-release.log`, `c-probe-debug.log`,
  `c-probe-release.log`, `c-probe-release-clang.log` (raw run output)
