# FINDINGS-shape: how small can the ABI plus the Rust trait surface get

Throwaway prototype round 3, lane `shape`. Copy: `/tmp/metis-plugin-registry-round-3/shape`.
Not product code, not a committed ABI. This file is written incrementally.

## Status

**COMPLETE for items 2, 3, 4 and 5. PARTIAL for item 1 (the decommission
measurement).** Read this first: the `legacy-callbacks` feature gate landed and
all four configurations build and run green, but the OFF configuration runs a
**reduced 20-check futures-only harness**, not the full 95-check suite: the OFF
harness runs 75 fewer checks, and the missing ones are all legacy-capability
checks that are compiled out. The counts below are exact and nothing is
described as more than it is.

| Configuration | Build command | Checks | Failures | Build exit | Run exit |
| --- | --- | --- | --- | --- | --- |
| `legacy-callbacks` ON, debug | `cargo build -p proto-plugin-cdylib -p proto-host` | 95 | 0 | 0 | 0 |
| `legacy-callbacks` ON, release | `cargo build --release -p proto-plugin-cdylib -p proto-host` | 95 | 0 | 0 | 0 |
| `legacy-callbacks` OFF, debug | `cargo build -p proto-plugin-cdylib -p proto-host --no-default-features` | 20 | 0 | 0 | 0 |
| `legacy-callbacks` OFF, release | `cargo build --release -p proto-plugin-cdylib -p proto-host --no-default-features` | 20 | 0 | 0 | 0 |

Baseline before any edit was **92 checks, 0 failures, exit 0** in debug and
release. The ON configuration is the baseline plus lane shape's 3 new
launch/await checks; the OFF configuration is the 20-check futures-only subset
(registry load, launch/await, waker balance, all ten hazard checks, teardown).

## Answers at the top

1. **How small can the ABI plus the Rust trait surface get if plugin futures are
   the only way the host calls a plugin?** The v2 sketch below is the exact
   answer, copy-pasteable. The plugin table keeps eleven entries (size, the two
   version words, and eight function pointers; three of the eight change or are
   promoted from the appended slots), the host table keeps six (one deleted:
   `complete`), and the authoring trait collapses from four arms to one `call`.
2. **What exactly disappears?** The `invoke` entry as a work path, the `complete`
   host callback, the executor seam with both implementations, the landed
   `HostAdapter`'s slot/waker/`Await`, and the plugin's per-request cancellation
   records. The honest caveat: `invoke` cannot be deleted from the *landed*
   table in the prototype, only drained to an inline `metis.*` control plane,
   because the table type lives in the read-only `plugin-contract` crate. The
   exact item-by-item table is under `Deleted surface`.
3. **Do the four arms survive for dyn-compatibility or object-safety reasons?**
   No, and the probe is a compiler verdict, not a claim. The four-arm shape is
   refused with `E0191` (its four per-arm associated types must be specified even
   to name `dyn Plugin`) and the routed shape is refused with `E0038` (`call`
   returns `impl Future`). The four arms add a dyn obstacle; they do not buy
   compatibility. Boxing the routed future buys `dyn Plugin` back for one
   allocation per call.
4. **8 concurrent plugin futures, plugin thread/runtime delta?** Driven, yes.
   Across the launch-and-await run the plugin's own counters show
   `threads_spawned` delta **0** and `poll_threads` **1**. Numbers below.
5. **Cost of launch-and-await against FINDINGS3?** Launch+await ready-on-first-poll
   is **947 to 1288 ns/call** against FINDINGS3's 864/826 ns. Full path p50 is
   **6893 to 8165 ns** against FINDINGS3's 5120/4510 ns, and the same-run
   direct-path p50 is 5200 to 5891 ns, so the extra **1.7 to 2.2 us is the
   `drive_all` std-waker bridge**, not the plugin.

## What was actually driven

### Baseline reproduction (before any edit)

```sh
cd /home/patric/Projects/metis-agent
devenv shell -- cargo build --manifest-path /tmp/metis-plugin-registry-round-3/shape/Cargo.toml
/tmp/metis-plugin-registry-round-3/shape/target/debug/proto-host            # 92 checks, 0 failures, exit 0
devenv shell -- cargo build --release --manifest-path /tmp/metis-plugin-registry-round-3/shape/Cargo.toml
/tmp/metis-plugin-registry-round-3/shape/target/release/proto-host          # 92 checks, 0 failures, exit 0
```

### The name-routed trait prototype (`crates/proto-routed`)

A real workspace member that replaces the four-arm vocabulary with one
capability-name-routed async surface. It compiles and its 4 unit tests pass:

```sh
devenv shell -- cargo test -p proto-routed --manifest-path /tmp/metis-plugin-registry-round-3/shape/Cargo.toml
# test result: ok. 4 passed; 0 failed
#   eight_capabilities_route_by_name ... ok
#   the_capability_cap_of_four_is_gone ... ok
#   the_return_future_is_small_and_erased ... ok
#   a_plugin_is_usable_behind_dyn_through_the_boxed_surface ... ok
```

The demo plugin routes **eight** capabilities through the one `call` method,
which is the proof that the cap of four is gone. The exact v2 trait is in the
`v2 sketch` section.

### The object-safety probe (compiler verdict, not a claim)

```sh
devenv shell -- rustc --edition 2021 --crate-type lib \
  /tmp/metis-plugin-registry-round-3/shape/probe_routed_trait.rs -o /tmp/probe_routed_trait.rlib
# exit 1; full stderr in probe_routed_trait.stderr.txt:
#   error[E0191]: the value of the associated types `Answer`, `Deferred`,
#                  `Cancellable` and `Fault` in `four_arm::Plugin` must be specified
#   error[E0038]: the trait `routed_rpitit::Plugin` is not dyn compatible
#                  ...because method `call` references an `impl Trait` type in its return type
devenv shell -- rustc --edition 2021 --crate-type lib \
  /tmp/metis-plugin-registry-round-3/shape/probe_routed_boxed_ok.rs -o /tmp/probe_routed_boxed_ok.rlib
# exit 0, no diagnostics: the boxed routed shape is dyn compatible
```

### The four-configuration run

`run-configs.sh` in the copy builds and runs each configuration and records the
build exit code, the run exit code, and the summary line; the outputs are
`run-on-debug.txt`, `run-on-release.txt`, `run-off-debug.txt`,
`run-off-release.txt`, and `run-log.txt`.

```sh
cd /home/patric/Projects/metis-agent
# ON
devenv shell -- cargo build           -p proto-plugin-cdylib -p proto-host --manifest-path <copy>/Cargo.toml
<copy>/target/debug/proto-host        # 95 checks, 0 failures, exit 0
devenv shell -- cargo build --release -p proto-plugin-cdylib -p proto-host --manifest-path <copy>/Cargo.toml
<copy>/target/release/proto-host      # 95 checks, 0 failures, exit 0
# OFF
devenv shell -- cargo build --no-default-features           -p proto-plugin-cdylib -p proto-host --manifest-path <copy>/Cargo.toml
<copy>/target/debug/proto-host        # 20 checks, 0 failures, exit 0
devenv shell -- cargo build --release --no-default-features -p proto-plugin-cdylib -p proto-host --manifest-path <copy>/Cargo.toml
<copy>/target/release/proto-host      # 20 checks, 0 failures, exit 0
```

The build order matters: each `cargo build` overwrites the cdylib next to the
host binary, so a host run always follows the build of its own configuration.

### Launch and await numbers

Release, ON configuration, three consecutive runs (4, 5, 6) on an otherwise
idle machine:

| Measurement | run 4 | run 5 | run 6 | FINDINGS3 reference |
| --- | --- | --- | --- | --- |
| launch+await, ready-on-first-poll, per call | 1064 ns | 1288 ns | 1257 ns | 864 / 826 ns (create+poll+drop) |
| launch+await, full path, p50 | 8115 ns | 6893 ns | 7043 ns | 5120 / 4510 ns (full path) |
| direct host-polled full path, p50 (same run) | 5891 ns | 5200 ns | 5280 ns | 5120 / 4510 ns |
| direct `catch_unwind` shim ready path (same run) | 577 ns | 896 ns | 896 ns | n/a |
| direct future create+poll+drop, hand vtable (same run) | 540 ns | 806 ns | 806 ns | 864 ns |
| 8 concurrent plugin futures, total wall | 93 us | 102 us | 101 us | n/a |

The OFF release run happened to land on a quieter machine moment and measured
8 concurrent futures in **23 us (2947 ns/future)**. All eight futures are
launched before any is driven, so all eight are in flight at once; the wall time
is dominated by the host's single timer thread and by `drive_all`'s round-robin,
which is honest and is one of the costs named below.

The zero-plugin-thread proof is the plugin's own counter read back through the
seam (`metis.future_counters`, itself a host-polled future, so the read cannot
spawn a thread): `threads_spawned` delta 0 and `poll_threads` 1.

## Deleted surface

`legacy-callbacks` OFF, as built. "Gone" means compiled out of the OFF build and
unreferenced; "keep" means it must stay. Binary sizes are from `deleted-surface.txt`.

| Item | Where it lives | What it was | OFF | Why |
| --- | --- | --- | --- | --- |
| `invoke` work path | plugin table | `fn(instance, capability, len, request_id, payload, host, out) -> Status`, returns `Pending`, completes later | gone (drained) | replaced by `invoke_future`; in the prototype the field survives as an inline `metis.*` control plane because the landed table type is fixed |
| `complete` callback | host table | `fn(user_data, request_id, status, payload)` | gone (unreferenced) | the poll return is the completion; nothing calls it |
| `MetisCompleteFn` | ABI type | host callback alias | gone (unreferenced) | would be deleted in a real v2 |
| `HostAdapter` slot/waker/`Await`/`Completed` | landed `plugin-contract::authoring::host_adapter` | pending registry, bounded wait, dropped-completion path | unreferenced by the OFF host | the future driver replaces it; the landed crate is read-only, so it is not removed here |
| `Executor` / `Task` / `SpawnRefused` | `proto-registry` | the primitives-side spawn seam | gone | nothing spawns work; the host is the executor |
| `PoolExecutor` / `IdlePolicy` | `proto-sdk` | bounded worker pool | gone (crate not compiled) | no plugin worker pool |
| `ThreadPerCallExecutor` | `proto-sdk` | one thread per call | gone (crate not compiled) | no plugin threads |
| `RequestState`, `Job`, `plan`, `run_job`, `complete`, `complete_guarded`, `finish`, `set_executor`, `executor`, `default_executor`, `DEFAULT_POOL_WORKERS`, `DRAIN_DEADLINE` | cdylib `lib.rs` | per-request cancel token, job plan, worker bodies, executor management | gone (34 gate sites) | the futures-only build serves capability calls through `invoke_future` |
| legacy capability descriptors (`tool.echo` ... `tool.future`) | cdylib `capabilities()` | 13 advertised capabilities | gone | their `invoke` path no longer exists; only the 5 future capabilities remain |
| legacy harness sections | host `main.rs` | 75 checks: registry cost, taxonomy, descriptor size, name squatting, async/cancel/never/concurrency/lifecycle, versioning, typed families, executor baselines | gone (6 gate sites) | they drive `invoke`/`Pending`/`complete` |
| `progress`, `emit_event` payloads used by legacy jobs | cdylib | progress before completion | `emit_event` kept (instance-start event); legacy progress gone | `progress` has no caller without legacy jobs |
| schemas / configure / accounting / counters | cdylib `metis.*` | protocol | **keep**, inline | the registry still needs the capability list and schemas to load at all |
| `service_complete` | plugin table | host's answer to a service request | **keep** | the future seam's only inbound service path |
| `MetisFutureHandle`, `MetisFutureVtable`, `MetisWaker`, `MetisWakerVtable` | ABI | lazy future + waker | **keep** | the whole seam |

Measured size effect (debug / release), same target directory:

| Artifact | ON | OFF | Delta |
| --- | --- | --- | --- |
| `libproto_plugin_cdylib.so`, debug | 64,953,560 B | 12,803,920 B | -52,149,640 B (-80%) |
| `libproto_plugin_cdylib.so`, release | 1,554,520 B | 924,328 B | -630,192 B (-40%) |
| `proto-host`, debug | 83,189,048 B | 80,835,568 B | -2,353,480 B (-3%) |
| `proto-host`, release | 7,585,288 B | 7,072,328 B | -512,960 B (-7%) |

Dependency closure left the OFF cdylib: the direct `proto-registry` and
`proto-sdk` edges (and `proto-sdk`'s nested `proto-registry`), from `dep-tree.txt`.

## v2 sketch

The whole contract, as types and signatures. Every field is a pointer, an
integer, or a function pointer. One-line notes say kept, changed, or new, and
what the entry replaced. This is deliberately a clean v2, not an append: with
futures as the only call path there is nothing to keep the one-way compatibility
with.

### Plugin table (v2)

```c
/* The plugin's one export, still discovered through metis_plugin_entry_v2. */
typedef struct MetisPluginTableV2 {
    size_t size;                  /* kept: size-prefixed, presence by size    */
    uint32_t abi_major;           /* kept                                     */
    uint32_t abi_minor;           /* kept                                     */
    MetisCreateInstanceFn  create_instance;   /* kept: now takes the v2 host table */
    MetisDestroyInstanceFn destroy_instance;  /* kept                              */
    MetisListCapabilitiesFn list_capabilities;/* kept: descriptor listing          */
    MetisInvokeFutureFn    invoke_future;     /* changed: promoted from appended; the ONLY call entry */
    MetisCancelFn          cancel;            /* kept: cancels a request_id, resolves the future Cancelled */
    MetisReleaseBufferFn   release_buffer;    /* kept: plugin frees only what it allocated */
    MetisShutdownFn        shutdown;          /* kept: quiesce; the host drops futures before this */
    MetisServiceCompleteFn service_complete;  /* changed: promoted from appended; host answers a service_request */
} MetisPluginTableV2;
/* DELETED: invoke. A launch is invoke_future; a completion is a poll return. */

typedef MetisStatus (*MetisInvokeFutureFn)(
    void *instance,
    const uint8_t *capability, size_t capability_len,  /* kept: routed by name */
    uint64_t request_id,                                /* kept: host-chosen id */
    MetisBuffer payload,                                /* kept: host-owned request bytes */
    const MetisHostApiV2 *host,                         /* changed: v2 host table */
    MetisFutureHandle *out);                            /* kept: plugin-owned future */

typedef MetisStatus (*MetisServiceCompleteFn)(
    void *instance, uint64_t request_id, MetisStatus status, MetisBuffer payload);
    /* changed: no longer an append; the host's only answer to service_request */
```

### Host table (v2)

```c
typedef struct MetisHostApiV2 {
    size_t size;                              /* kept                             */
    void *user_data;                          /* kept                             */
    MetisProgressFn progress;                 /* kept: progress before a future resolves */
    MetisEmitEventFn emit_event;              /* kept                             */
    MetisLogFn log;                           /* kept                             */
    MetisServiceRequestFn service_request;    /* changed: promoted from appended; timers/IO */
} MetisHostApiV2;
/* DELETED: complete. A future's poll return is the completion; a callback that
   duplicates it is exactly the second protocol this round removes. */
```

### Future and waker types (kept)

```c
typedef struct MetisFutureHandle { void *ctx; const MetisFutureVtable *vtable; } MetisFutureHandle;
/* kept: opaque plugin-owned ctx; the host never dereferences it            */

typedef struct MetisFutureVtable {
    size_t size;                              /* kept: size-first            */
    MetisFuturePollFn poll;                   /* kept: Ok ready / Pending / terminal */
    MetisFutureDropFn drop;                   /* kept: exactly once          */
} MetisFutureVtable;

typedef struct MetisWaker { void *data; const MetisWakerVtable *vtable; } MetisWaker;
/* kept: host-owned state; the plugin may wake from any thread              */

typedef struct MetisWakerVtable {
    size_t size;                              /* kept                        */
    MetisWakerCloneFn clone;                  /* kept                        */
    MetisWakerWakeFn wake;                    /* kept: consumes              */
    MetisWakerWakeByRefFn wake_by_ref;        /* kept                        */
    MetisWakerDropFn drop;                    /* kept: consumes              */
} MetisWakerVtable;
```

### Authoring trait (v2)

```rust
/// One request shape for every capability, resolved by name.
pub struct Request<'a> {
    pub capability: &'a str,   // new: the routed name replaces the four arms
    pub body: &'a [u8],        // new: erased bytes, the plugin decodes by schema
}

/// One response shape for every capability.
pub struct Response { pub body: Vec<u8> }   // new: replaces four associated Response types

/// What a plugin author implements. Four arms, four capability-name consts and
/// four associated `Shape` types are gone.
pub trait Plugin: Default + Send + Sync + 'static {
    /// The whole declaration. There is no second, per-arm declaration.
    fn capabilities(&self) -> Vec<CapabilityDescriptor>;

    /// Routes one call by capability name and runs it. One method, one request,
    /// one response.
    fn call(
        &self,
        call: Call,                                   // kept: id, cancel token, progress/event/log
        request: Request<'_>,                         // new
    ) -> impl Future<Output = Result<Response, PluginError>> + Send;

    /// kept: a cancel was accepted; the token is already marked.
    fn cancel(&self, call: Call) -> Result<(), PluginError> { let _ = call; Ok(()) }

    /// kept: life cycle pair.
    fn started(&self, host: &Host) -> Result<(), PluginError> { let _ = host; Ok(()) }
    fn stopped(&self, host: &Host) { let _ = host; }
}
```

If `dyn Plugin` is ever needed, the only change is to box the return:
`fn call<'a>(&'a self, call: Call, request: Request<'a>) -> BoxFuture<'a, Result<Response, PluginError>>`,
which the probe shows compiles. The four-arm shape does not get that for free.

## The honest cost of the simplification

What the host must still do by hand, and which of it could move into the SDK:

1. **Poll loop.** The host owns "when plugin code runs", so it owns the poll
   loop. This must stay host-side: it is the unload-safety property. The SDK can
   provide the loop, but the host must own its lifetime.
2. **Waker plumbing.** Cloning, dropping, and the fact that the plugin may wake
   from any thread. This is mechanical and *could move into the SDK*: the host
   should hand in one host waker and the SDK should bridge it to `std::task`.
   This lane wrote that bridge (`drive_all`, `Signal`) and paid **1.7 to 2.2 us
   per call** for doing it the naive way (a fresh `Signal` allocation and an
   extra condvar round trip per call). A real executor would do it once.
3. **Cancel.** The host must decide the deadline, call `cancel(request_id)`, and
   never drop while a poll is in flight. The deadline policy is a host policy
   (the reclaim lane owns it); the mechanics *could move into the SDK* as a
   `cancel_with_deadline` helper.
4. **Buffer release.** Every plugin-owned buffer, including one delivered on a
   poll the host abandons. Mechanical; *could move into the SDK* as the poll
   return value (ready buffer is consumed by the poll shim).
5. **Teardown ordering.** Stop producers, join the timer thread, drop every
   future and waker clone, then unload. The host owns this and must keep owning
   it; the SDK should never unload a library it did not load.
6. **Request-id allocation.** Purely mechanical; *should move into the SDK*.
7. **`catch_unwind` at poll and drop.** Mechanical; *should move into the SDK*;
   the OFF build today only has it because the plugin author's vtable does.
8. **Schema derivation and typed facades.** All *could move into the SDK*; they
   are exactly what the four-arm trait was blocking, and the routed trait
   unblocks.
9. **Service handlers (timers/IO).** The host must answer `service_request`. The
   timer wheel vs thread policy is a host decision; the routing plumbing *could
   move into the SDK*.

The load-time arm-versus-descriptor check is the one real thing given up by name
routing: the adapter used to compare `capabilities()` against the four consts and
fail loudly at instance creation. With name routing, a capability the
implementation does not `match` is `NotFound` at call time (or the SDK can call a
new defaulted `handles(name) -> bool`). That is cheaper than a four-capability
ceiling.

## Recommendation

Adopt the routed single-method trait and the futures-only table, and delete the
legacy callback path. The trait simplification is the clear win: it removes the
cap of four that is blocking `metis-plugin-sdk` from pinning author-facing API,
it removes four consts and four associated types per plugin, and the probe shows
it costs nothing in object safety that the four arms were providing (they were
not: both shapes are non-dyn-compatible, and the routed one is strictly easier to
box). The futures-only table is also the right long-term shape, but this lane
proves the decommission is smaller than it looks *and* more entangled than the
north star implies: `invoke` cannot vanish while `metis.schemas` is how the host
learns the capability list, and the landed `HostAdapter` discards an inline
payload, so the OFF build needed a direct table call for the protocol. So:
**delete `complete` now**, **keep `progress`/`emit_event`/`log`**, **fold
`invoke_future`/`service_complete`/`service_request` into the core of a v2
table**, and **move the `metis.*` control plane onto `invoke_future`** so
`invoke` can be deleted too. The host-side wrapper cost (~1.7 us/call in this
prototype, an artifact of a naive waker bridge) belongs in the SDK's executor,
not in every host. Do not keep the four arms "for dyn": the probe says the
reason does not exist.

## Unproven

Everything here is not driven or not measured, and is never described as
verified:

- **The full 95-check suite was not run in the OFF configuration.** The OFF
  harness is an independently written 20-check futures-only harness, so the OFF
  configuration runs **75 fewer checks** than ON. It proves the legacy path
  compiles out and the futures-only plugin still loads, routes, launches,
  hazards, and unloads, but it does not prove the 75 legacy checks would pass in
  some re-expressed form.
- **The `invoke` entry could not be structurally deleted.** The table types live
  in the read-only `plugin-contract`/`proto-abi` crates, so OFF keeps the field
  and drains it. A real v2 must delete the field, which this lane only sketches.
- **The landed `HostAdapter` inline-payload defect.** `HostAdapter::invoke` reads
  and releases an inline `Ok` payload but does not return it; the OFF host works
  around it by calling `invoke` directly against the table. A real host must
  return the inline payload, or the `metis.*` control plane cannot be served at
  all. Not fixed here (the crate is read-only for this lane).
- **`metis.*` over futures is sketched, not driven.** OFF keeps `metis.*` on the
  inline `invoke`. Moving it onto `invoke_future` is the last step to delete
  `invoke`, and it was not built.
- **The `drive_all` concurrency number is not a real executor.** It is a
  round-robin poll loop with one notification flag; it is not work-stealing and
  it is not the daemon's executor. Its 23 to 102 us for eight futures includes
  the host's single timer thread, which serialises `service_request`s.
- **The launch/await overhead is not decomposed.** The 1.7 to 2.2 us over the
  direct path is attributed to the std-waker bridge by inspection, not by a
  separate measurement of `Waker::from(Arc)` alone.
- **`async-ffi` was not touched by this lane.** The FFI future object is the
  existing prototype code; this lane did not exercise its `Drop`-panic abort,
  and the async-ffi lane owns that question.
- **The 8-concurrent check with a nonzero timer delay was not run** after the
  `delay_ms` change; the check uses `delay_ms: 0`, so timer-thread serialisation
  under a real delay is unmeasured here.
- **No C implementer and no C header regeneration** for the v2 sketch; the sketch
  is Rust-typed and has not been through `cbindgen` or the C layout asserts.
- **Nothing was measured under a release profiler or on a quiet machine for
  every run.** The ready-path spread (947 to 1288 ns) and the concurrency spread
  (23 to 102 us) are host-load sensitive.

## Contract deltas

For every ABI type, table entry, or trait item this lane would add, change, or
remove, named exactly.

**Append (to the current landed/appended table, before a clean v2):**

- Nothing to add. Every type needed already exists in `proto-abi`: the appended
  `invoke_future`, `service_complete`, `service_request`, and the
  `MetisFutureHandle`/`MetisFutureVtable`/`MetisWaker`/`MetisWakerVtable` types.
  The v2 is a re-cut of existing entries, not an addition.

**Change (in a v2 cut):**

- `MetisPluginTableV2.invoke_future`: promoted from appended to the call entry;
  signature unchanged from `MetisInvokeFutureFn`.
- `MetisPluginTableV2.service_complete`: promoted from appended; signature
  unchanged from `MetisServiceCompleteFn`.
- `MetisHostApiV2.service_request`: promoted from appended; signature unchanged
  from `MetisServiceRequestFn`.
- `MetisCreateInstanceFn`: parameter type becomes `const MetisHostApiV2 *`
  (was `const MetisHostApiV1 *`).
- Authoring `Plugin`: `capabilities` kept; `call(Call, Request<'_>) -> impl
  Future<Output = Result<Response, PluginError>> + Send` replaces `answer`,
  `deferred`, `cancellable`, `fault`; `cancel`, `started`, `stopped` kept
  unchanged.
- New authoring types `Request<'a> { capability: &'a str, body: &'a [u8] }` and
  `Response { body: Vec<u8> }` replace the four `Shape` implementations.

**Remove (in a v2 cut):**

- `MetisPluginTableV1.invoke` and the `MetisInvokeFn` alias.
- `MetisHostApiV1.complete` and the `MetisCompleteFn` alias.
- Authoring `Plugin::ANSWER`, `DEFERRED`, `CANCELLABLE`, `FAULT`;
  `type Answer`, `type Deferred`, `type Cancellable`, `type Fault`; and the four
  default methods `answer`, `deferred`, `cancellable`, `fault`.
- Authoring `trait Shape` (it existed only to parameterise the four arms).
- `plugin-contract::authoring::host_adapter` in full (`HostAdapter`, `Await`,
  `Completed`, the slot map, the waker), once nothing calls `invoke`/`complete`.
- `plugin-contract::authoring::executor` in full.
- Primitives `Executor`, `Task`, `SpawnRefused` from `proto-registry` (these are
  prototype-side; the landed crate has its own executor module to remove).
- `proto-sdk`'s `PoolExecutor`, `ThreadPerCallExecutor`, `IdlePolicy`.

**Keep unchanged:**

- `MetisBuffer`, `MetisStatus`, the seven `METIS_*` log constants, `MetisFuture*`,
  `MetisWaker*`, `create_instance`, `destroy_instance`, `list_capabilities`,
  `cancel`, `release_buffer`, `shutdown`, `progress`, `emit_event`, `log`,
  `MetisHostApiV2.size`/`user_data`, and the append-only `size` presence rule.

**Open operator decisions (not settled by the written spec):**

1. **Does `metis.*` move onto `invoke_future`, and does the capability list
   travel through a future too?** If not, `invoke` cannot be deleted. This lane
   keeps `metis.*` on inline `invoke`; the spec does not say which way to go.
2. **Does cancellation stay on a plugin-table `cancel(request_id)`, or become a
   waker/handle-level operation?** This lane keeps the existing entry; the
   reclaim lane may have a view, and SEAM-SPEC fixes only the behaviour.
3. **Does `progress` survive the futures-only cut, or does progress become an
   event?** The seam spec keeps progress; a future could emit progress as an
   event through `emit_event` and drop one host callback. Not measured.
4. **Is the v2 a clean cut or an append?** SEAM-SPEC item 9 says this is open.
   The sketch assumes clean, because a futures-only contract has nothing to stay
   compatible with; the round's other lanes may argue otherwise.
