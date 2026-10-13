# Host-polled future seam: prototype spec

Throwaway prototype spec. It exists so three independent implementations can be
built from one written contract and compared. It is not product code and not a
committed ABI.

## The one idea

`async-ffi`'s `FfiFuture` never crosses the boundary. The boundary carries an
opaque context pointer plus two C vtables we define and version ourselves, and
`async-ffi` is used only inside the plugin adapter to build the future object
that the context points at. The host is then the only executor: it polls the
plugin's future on its own runtime. Plugins need no runtime of their own, and
because the host decides when plugin code runs, unloading is safe.

## Boundary rule

Every field crossing the boundary is a pointer, an integer, or a function
pointer. The committed cbindgen header must express all of it, and the layout
asserts must hold in pointer-size terms. No Rust struct with private fields
crosses.

## New types (all `repr(C)`, all `size`-first where they are tables)

```
MetisFutureHandle { ctx: *mut c_void, vtable: *const MetisFutureVtable }

MetisFutureVtable {
    size: usize,
    poll: MetisFuturePollFn,
    drop: MetisFutureDropFn,
}

MetisFuturePollFn = unsafe extern "C" fn(
    ctx: *mut c_void,
    waker: MetisWaker,
    out_value: *mut MetisBuffer,
) -> MetisStatus;
    // Ok      -> ready, out_value holds a plugin-allocated buffer
    // Pending -> not ready, the plugin will call waker.wake when it is
    // anything else -> the call failed and the future is terminal

MetisFutureDropFn = unsafe extern "C" fn(ctx: *mut c_void) -> MetisStatus;

MetisWaker { data: *mut c_void, vtable: *const MetisWakerVtable }

MetisWakerVtable {
    size: usize,
    clone: unsafe extern "C" fn(waker: *const MetisWaker) -> MetisWaker,
    wake: unsafe extern "C" fn(waker: MetisWaker),        // consumes
    wake_by_ref: unsafe extern "C" fn(waker: *const MetisWaker),
    drop: unsafe extern "C" fn(waker: MetisWaker),        // consumes
}
```

## Appended table entries

Append only, after the existing last entry, with the existing size-prefix rule.

Plugin table (appended, in this order):

```
invoke_future: unsafe extern "C" fn(
    instance: *mut c_void,
    capability: *const u8,
    capability_len: usize,
    request_id: u64,
    payload: MetisBuffer,
    host: *const MetisHostApiV1,
    out: *mut MetisFutureHandle,
) -> MetisStatus;

service_complete: unsafe extern "C" fn(
    instance: *mut c_void,
    request_id: u64,
    status: MetisStatus,
    payload: MetisBuffer,
) -> MetisStatus;
    // delivery of a host-owned service the plugin awaited
```

Host table (appended):

```
service_request: unsafe extern "C" fn(
    user_data: *mut c_void,
    service: *const u8,
    service_len: usize,
    request_id: u64,
    payload: MetisBuffer,
) -> MetisStatus;
    // the plugin asks the host for a timer or an IO operation; the host answers
    // later through the plugin's service_complete, and the plugin wakes its
    // future's waker when that answer arrives
```

## Rules the prototype must enforce and demonstrate

1. **Ownership.** The plugin allocates `ctx` and owns it. The host never
   dereferences `ctx`; it only passes it back to `poll` and `drop`. The host
   calls `drop` exactly once, and `drop` must release every plugin-owned buffer
   the future still holds. After `drop`, the handle and every waker the plugin
   kept are invalid.
2. **Single poller.** Only the host polls. Two concurrent polls on one handle
   are a protocol violation. `poll` must not block: blocking work goes through
   `service_request`.
3. **Waker lifetime.** The waker the host passes is valid until the future is
   dropped. The plugin may clone it (each clone is a `clone` call) and must
   `drop` every clone. `wake` may be called from any thread, must be cheap and
   non-blocking, and waking after the future is dropped is a protocol violation
   that the host must detect rather than crash on.
4. **Ready and Pending.** Ready carries a plugin-allocated buffer in
   `out_value`, released through the plugin's `release_buffer` as today. Pending
   means the plugin will call `wake` when ready.
5. **Panic containment.** Our own `poll` and `drop` shims are the boundary and
   each wraps the author's future in `catch_unwind`. A panic becomes
   `MetisStatus::Internal`, leaves the future terminal, and `drop` remains
   legal. The host's waker bodies are panic-guarded like every other callback.
   No path may reach `std::process::abort`.
6. **Cancellation.** The host calls the existing `cancel(request_id)`. The
   expected protocol is that the plugin resolves the future with ready plus
   `Cancelled` instead of being dropped mid-flight. The host waits a deadline;
   if the plugin misses it, the host may still `drop` and must record a protocol
   violation. The host never calls `drop` while a poll is in progress: it first
   observes a poll return.
7. **Unload.** The library is unloaded only after every future has been dropped
   and no waker clone remains. The host owns this, because it owns the poll
   loop, and it must prove the ordering with observable markers.
8. **Legacy path stays alive.** A plugin that knows nothing about
   `invoke_future` is still driven through `invoke` returning `Pending` and the
   `complete` callback. A reader that only knows the old prefix must work
   unchanged. Cross-language implementers may ignore the new entries entirely.
9. **Versioning.** Appended entries only, size-prefixed, presence decided by
   `size`. Whether the future path should instead be a clean v2 table is an open
   question for the versioning lane, not an assumption.

## What the prototype must measure

- Per-call latency for the host-polled path, release build, against the known
  numbers: 26 microseconds thread-per-call, 5.4 microseconds pooled executor,
  486 nanoseconds synchronous native call.
- The per-poll cost of the `catch_unwind` shim, and the cost of creating and
  destroying a future per call versus reusing one.
- The cost of a `service_request` round trip for a timer.

## Lane questions

The three lanes implement this same spec in their own prototype copy and answer
one question each. Nobody edits another lane's copy.

- **Lane 1, seam and cost.** Can a capability be driven end to end by a
  host-polled future with no plugin runtime at all, and what does it cost? Also:
  a second capability whose vtable is written by hand with no `async-ffi`
  anywhere, as the stand-in for a C implementer, so the boundary is proven
  implementable without Rust sugar.
- **Lane 2, hazards.** Panic in poll, panic in drop, panic in a waker, cancel
  that resolves as `Cancelled` then drop, a plugin that ignores the cancel
  deadline, double drop, drop after unload, a waker waking after drop, and a
  blocking poll that must be detected rather than allowed to hang the host.
  Each becomes an observable check.
- **Lane 3, versioning and expressibility.** cbindgen emits the new structs into
  the committed header; the C layout asserts cover them; prefix compatibility
  holds in both directions; the legacy `Pending`/`complete` path still works
  alongside; and the cost of carrying two async protocols is written up honestly
  against the option of a clean v2 table.

## Hygiene

- Work only in your own copy. Read the repository and the other copies, never
  write to them. Delete nothing anywhere.
- `cargo` is not on the bare PATH: run
  `cd /home/patric/Projects/metis-agent && devenv shell -- cargo <args> --manifest-path <YOUR-COPY>/Cargo.toml`,
  and build the cdylib before running the host.
- Keep the existing host suite green where it still applies, print one line per
  check, and exit non-zero on failure.
- Write `FINDINGS-<lane>.md` incrementally in your copy. Anything not driven or
  not measured is listed as unproven, never described as verified.
