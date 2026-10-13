# FINDINGS 3: the host-polled future seam, folded into the main prototype

Throwaway prototype. Not product code. This is the folded state: the main
prototype `/tmp/metis-plugin-registry-prototype` now holds one ABI surface, one
host, and one plugin for the host-polled future seam, plus the legacy registry
work from `FINDINGS.md` and `FINDINGS2.md`, plus the round-two executor
baselines. `FINDINGS.md` (round one) and `FINDINGS2.md` (round two) are kept
unchanged on disk as the record of those rounds.

The seam contract is `.work/artifacts/host-polled-future-seam-spec.md`. The
three lane copies were read only and never written. Nothing in the repository
or the other copies was touched.

## Answers first

- **Is the fold complete?** Yes, for the three lanes plus the executor
  baselines. Check counts: **92 checks, 0 failures, exit 0, in debug and
  release**. The single open instruction was the in-binary pooled-executor
  baseline; the supervisor chose to fold round-two candidate A's pool, and it
  is folded rather than quoted (the safety valve was not needed).
- **What are the baselines, re-measured by one harness in this binary?**
  Release, two runs:

  | Path | reference | run 1 | run 2 |
  | --- | --- | --- | --- |
  | legacy callback path, thread-per-call executor | 26 us | 21.5 us | 22.8 us |
  | legacy callback path, pooled executor | 5.4 us | 5.8 us | 5.6 us |
  | `registry.invoke`, native in process (synchronous) | 486 ns | 661 ns | 667 ns |
  | host-polled full path (invoke_future + host timer + poll + drop) | n/a | avg 5.45 us, p50 5.12 us, p95 6.54 us, p99 9.80 us, max 97 us | avg 5.09 us, p50 4.51 us, p95 7.15 us, p99 10.2 us, max 111 us |
  | host-polled ready-on-first-poll (create + poll + drop) | n/a | 864 ns | 826 ns |
  | host-polled reuse (poll only) | n/a | 440 ns | 424 ns |
  | `catch_unwind` shim share (ready-on-first-poll) | n/a | 15 ns/call (2%) | 14 ns/call (2%) |
  | `service_request` timer round trip (last) | n/a | 2.69 us | 2.27 us |

  The host-polled path is below the thread-per-call baseline, in the same band
  as the pooled baseline, and the ready-on-first-poll path is near the
  synchronous native call. The average of the full path is skewed by a handful
  of scheduler stalls (max around 100 us in 2000 calls); the median and p99 are
  the honest numbers.
- **What could not be folded?** Nothing from the three lanes. The pooled
  executor is folded from round-two candidate A, as directed.

## What was folded, lane by lane

### Lane 1 (c1), seam and cost

- The two plugin future objects: the ergonomic one, an ordinary `async fn`
  wrapped with `async-ffi` 0.5 (`FfiCtx`), and the hand-written state machine
  with no `async-ffi` and no Rust `Future` (`HandCtx`), which stands in for a
  non-Rust implementer. `async-ffi` is cached and builds; it is used only
  inside the adapter, and nothing from it crosses the boundary.
- The host executor: one persistent host timer thread that answers
  `service_request("timer", ...)` through the plugin's `service_complete`, and
  a poll loop that parks on the waker's condvar.
- The teardown fix, kept fixed: `FutureHost::drop` takes the timer
  `mpsc::Sender` out before it joins the timer thread, because joining while
  the sender is alive can never return. The order ledger proves
  `["shutdown returned", "future host dropped", "library unloaded"]` (verified
  in the run).
- The cost benches: full path, shim share, create versus reuse.

### Lane 2 (c2), hazards

- The host waker is an `Arc<WakerState>` over an `Arc<FutureState>`, so a clone
  that outlives the future still finds a live host object: a wake after drop is
  a recorded violation, not a dangling dereference.
- Windows: a `poll_bounded` that runs one poll on a watchdog thread with a
  deadline, so a blocking poll is detected and abandoned instead of hanging.
- `drop_future` refuses a double drop, a drop after unload, and a drop while a
  poll is in progress, each recorded as a violation.
- The ordering ledger checks that no drop ever overlapped a poll.
- All nine hazards, plus a presence check, print one line each through the same
  harness.

### Lane 3 (c3), versioning and expressibility

- `crates/proto-abi` is the current ABI: the landed surface byte for byte, with
  the seam appended (appended entries, appended types, `size` first). presence
  is decided by `size` alone.
- `cbindgen` regenerates `crates/proto-abi/include/metis_plugin_abi.h`
  byte-identically (verified). The generated C compiles as C11 with
  `-std=c11 -Wall -Wextra -Werror` and no diagnostics (gcc and clang), and
  `build.rs` compiles `c/layout_static_asserts.c` on every build with the same
  flags.
- Rust `const` layout asserts (`src/layout.rs`) and the C asserts agree; the
  `proto-abi` tests compare the C compiler's numbers with Rust's `size_of`,
  `align_of`, `offset_of` (3 tests pass).
- Seven compatibility checks: old host drives new plugin, new host sees the
  appended entries, old plugin with a short table is still driven, short
  prefixes are refused, a landed host table reports `service_request` absent,
  and a new host drives one appended future end to end.

### Round-two candidate A, executor baselines

- The primitives-side `Executor` seam (`spawn`, `drain`, `is_draining`) is in
  `proto-registry` and stays runtime-free. `proto-sdk` holds the bounded
  `PoolExecutor` and the `ThreadPerCallExecutor` fallback.
- The plugin's legacy callback path now runs through `state.executor().spawn`
  with the pool as the default; `metis.executor` switches to `thread` or
  `pool`. The switch runs inline in `invoke` (draining the old executor from
  one of its own workers would deadlock). Shutdown refuses new calls by
  draining the executor with a deadline and escalating to an unbounded join.
- The host bench switches the plugin executor via `metis.executor` and
  re-measures the same cdylib `tool.echo` call under both, so the two baselines
  come from one harness in one binary.

The host-polled seam does not use the executor and spawns no plugin thread on
the ordinary path; the check `the host-polled path spawned no plugin thread and
no plugin runtime` still proves it (delta 0, `poll_threads == 1`).

## What the folded prototype is

- **One ABI surface.** `proto-abi` is the single current table: the landed
  prefix, then `invoke_future` and `service_complete` on the plugin table and
  `service_request` on the host table, plus `MetisFutureHandle`,
  `MetisFutureVtable`, `MetisWaker`, `MetisWakerVtable` and their
  function-pointer aliases. Every field is a pointer, an integer, or a function
  pointer.
- **One host.** `proto-host/src/future_driver.rs` holds the executor, the
  waker with liveness, the watchdog, the ordering ledger, the refusals, the
  timer thread and the teardown fix, and the hazard harness. There is no second
  host for the hazards.
- **One plugin future module.** `proto-plugin-cdylib/src/future.rs` holds both
  vtable implementations, the hazard modes, and the lane 3 self-woken probe.
  All capabilities route through the two objects; there is no third object.
- The legacy `Pending` plus `complete` path, the registry, capability routing,
  both-direction schema validation, version gating, duplicate-name refusal,
  enable and disable, and unregister all still work; the whole legacy suite is
  green alongside the new checks.

## Check counts

| Group | Checks |
| --- | --- |
| legacy registry, async, typed, shutdown and unload suite (carried over) | 65 |
| lane 1: seam and teardown (8 seam plus 2 teardown) | 10 |
| lane 2: hazards (presence plus nine hazards) | 10 |
| lane 3: versioning | 7 |
| **total** | **92** |

Both debug and release print `92 checks, 0 failures` and exit 0. Every check
prints one `PASS`/`FAIL` line; the harness flushes stdout after every line, so a
killed run still leaves its markers on disk. Every host-polled wait has a short
deadline that reports `FAIL` instead of blocking.

Unit tests: 3 `proto-abi` layout tests plus 2 host ordering-ledger tests, all
pass.

## Commands

```
cd /home/patric/Projects/metis-agent
devenv shell -- cargo build           --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml
devenv shell -- cargo test            --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml
devenv shell -- cargo build --release --manifest-path /tmp/metis-plugin-registry-prototype/Cargo.toml
devenv shell -- /tmp/metis-plugin-registry-prototype/target/debug/proto-host
devenv shell -- /tmp/metis-plugin-registry-prototype/target/release/proto-host
```

The cdylib is built before the host in every run, because `cargo run` of the
host does not rebuild a cdylib member. Full output is in
`host-output-seam-debug.txt`, `host-output-seam-release-1.txt` and
`host-output-seam-release-2.txt`.

cbindgen and the C compile:

```
devenv shell -- cbindgen --config crates/proto-abi/cbindgen.toml \
    --output /tmp/regen.h --crate proto-abi    # byte-identical to the committed header
devenv shell -- cc    -std=c11 -Wall -Wextra -Werror -c crates/proto-abi/c/layout_static_asserts.c \
    -I crates/proto-abi/include -o /tmp/layout.o   # exit 0, no diagnostics
devenv shell -- clang -std=c11 -Wall -Wextra -Werror -c crates/proto-abi/c/layout_static_asserts.c \
    -I crates/proto-abi/include -o /tmp/layout.o   # exit 0, no diagnostics
```

## Still open, unproven, never described as verified

- **Reclaiming an abandoned blocking poll.** When the watchdog times out, the
  host abandons the future and leaves the blocked poll thread where it is,
  because the host must never drop a future while a poll is in progress. The
  abandoned `ctx` box and one stored waker clone are leaked by design. A real
  host needs a per-poll completion signal or a quarantine queue to reclaim
  after a late poll return.
- **Unloading while a blocked poll is still inside plugin code.** The prototype
  waits 900 ms for the blocking thread to leave plugin code before the library
  can unload. A real host cannot wait forever; in process it would have to
  quarantine and delay unload. Not proven.
- **Payload ownership for the two service entries.** `service_request`'s
  payload is plugin-owned and released by the host through `release_buffer`;
  `service_complete`'s payload is host-owned and borrowed for the call. This is
  an interpretation consistent with the landed buffer rules, not a clause the
  spec settles, and it was not exercised against a non-Rust implementer.
- **Only lane 1 exercised `async-ffi`.** The folded prototype does use it, and
  the ergonomic path is driven end to end. But `async-ffi` aborts the process
  if a future's `Drop` panics, which our shim cannot contain, and that is not
  exercised. Lane 2's hazards run through the hand-written object, and lane 3's
  probe does too.
- **The host timer is one thread per driver, not a pool.** A slow timer delays
  every later timer on that driver. Unmeasured under concurrency.
- **The blocking-poll detection is per call where used, not on every poll.**
  The ordinary path polls directly and bounds the whole call with a deadline;
  the watchdog is used for the blocking hazard (and is available to any caller
  that wants it). A poll that blocks on the ordinary path is caught by the
  call-level deadline, not the watchdog. This is the same shape lane 2 shipped.
- **Pool sizing and the idle-timeout re-init race.** The folded pool uses the
  candidate A implementation unchanged; the idle policy is `Stay` by default
  and the re-init race was not stress-tested.
- **No C implementer was compiled.** The hand-written vtable is Rust standing
  in for one.

## Files changed in the main prototype

- `Cargo.toml`, `Cargo.lock`.
- `crates/proto-abi/src/lib.rs`, `src/layout.rs`, `Cargo.toml`, `build.rs`,
  `cbindgen.toml`, `c/layout_static_asserts.c`,
  `include/metis_plugin_abi.h`, `tests/layout.rs` (lane 3).
- `crates/proto-plugin-cdylib/src/lib.rs`, `src/future.rs`, `Cargo.toml`
  (lanes 1, 2, 3, plus the executor wiring).
- `crates/proto-registry/src/lib.rs` (the primitives-side executor seam).
- `crates/proto-sdk/` (new: the bounded pool and the thread-per-call fallback).
- `crates/proto-host/src/future_driver.rs` (the one host, lanes 1 and 2),
  `src/future_bench.rs`, `src/versioning.rs` (lane 3), `src/bench.rs`,
  `src/abi_adapter.rs`, `src/lib.rs`, `src/main.rs`, `Cargo.toml`.
- `FINDINGS3.md` (this file), `host-output-seam-debug.txt`,
  `host-output-seam-release-1.txt`, `host-output-seam-release-2.txt`.

Nothing was deleted or pruned.
