# FINDINGS: lane async-ffi (round 3, open item O4)

Throwaway prototype. Not product code. Every number here is one I drove in this
copy (`/tmp/metis-plugin-registry-round-3/asyncffi`); everything I did not drive
is in `Unproven`.

## Status

**COMPLETE.** Read this first: the finding is that `async-ffi` should be
**dropped**, on one hard measured fact (a destructor panic inside a plugin
future aborts the whole process, and no containment from outside the crate can
stop it) plus a comparison that shows dropping it costs the operator nothing
that matters, including in the one place it could have cost something, namely
the author's `async fn` shape. The replacement helper is written and driven in this copy, not
just described.

Check counts, whole workspace built before the host in each profile:

| Profile | Checks | Failures | Exit code |
| --- | --- | --- | --- |
| debug | 100 | 0 | 0 |
| release (run 1) | 100 | 0 | 0 |
| release (run 2) | 100 | 0 | 0 |
| release (run 3) | 100 | 0 | 0 |

The 92 baseline checks are untouched: every `PASS` line in the pre-change
baseline output is still present, and the 8 new lines are all from this lane
(`comm` between the baseline and the new release output shows no baseline line
missing). Unit tests are unchanged and green: 3 `proto-abi` layout tests and 2
`proto-host` ordering-ledger tests, `cargo test` exit 0.

## Answers at the top

1. **Do we need the `async-ffi` crate at all?** No. It is not on the wire (zero
   ABI types, `cbindgen` header has no mention of it), it costs ~23 KB (2.0%) of
   cdylib and one dependency, it buys nothing measurable per call, and it makes
   one whole class of plugin bug fatal: a panic in any destructor inside the
   author's future calls `std::process::abort`, which the host cannot contain,
   and which is reachable even when the plugin returned `Ok` for both the
   invocation and the poll.
2. **What does it cost?** Text delta 12,868 bytes (1.5% of a 1.12 MB cdylib),
   of which the crate's own symbols are 2,725 bytes over 25 symbols; +1
   dependency in the graph (25 versus 24 packages, and it is a leaf); marginal
   rebuild time indistinguishable (1.20 to 1.31 s with, 1.21 to 1.37 s without);
   per call a wash to a few percent slower than a hand-rolled object that does
   the same work across three runs, and unrankable on the host-timer path. Its
   real cost is not bytes or nanoseconds, it is the abort.
3. **Where does it help?** `async-ffi` contains a panic in `poll` itself
   (`FfiPoll::Panicked`, re-raised on the plugin side and caught by our shim,
   measured as check 1). It does not help with a panic in `drop`, and its own
   docs say the abort there is deliberate because unwinding across FFI is UB.

## What was actually driven

### 1. The drop-panic probe: exit status, stderr, reachability, containment

Built: three probe capabilities in the plugin cdylib that hold a destructor
which panics, and one that panics in `poll`:

- `future.ffi_panic_drop`, the `async-ffi` object over a future that never
  completes and holds a panicking guard (the abandoned-pending case);
- `future.ffi_panic_drop_ok`, the `async-ffi` object over a future that answers
  `Ok` on the first poll and panics when the object is dropped;
- `future.ffi_panic_poll`, the `async-ffi` object over a future that panics on
  the first poll;
- `future.async_panic_drop` and `future.async_panic_drop_ok`, the same two drop
  cases on the hand-rolled object (no crate);
- reusing the existing `future.panic_drop`, the hand-written vtable over a
  future whose author `drop` panics (lane 2's hazard).

All six names are host-internal and deliberately not advertised, so the
advertised-capability count of 18 and every routing check are unchanged.

Each aborting case runs in a **child process**: `proto-host --probe-child <kind>
<cdylib>` re-executes the host binary, drives one future through the real
cdylib and the real ABI, prints `PROBE-CHILD ... invoke=Ok poll=<status>` and
flush, and only then drops. The parent records the child's exit status, stdout
and stderr and prints one check per case. The raw child runs are saved verbatim
in `artifacts/probe-child-*.txt`.

Measured, release:

```
$ target/release/proto-host --probe-child ffi-drop-panic
stdout: PROBE-CHILD kind=ffi-drop-panic invoke=Ok poll=Pending
stderr: thread '<unnamed>' panicked at .../future.rs:389:9:
        probe: a destructor inside the author's future panicked
        async-ffi: abort due to panic across the FFI boundary in Future::drop
exit status: 134 (SIGABRT)
```

- **Exit status: 134, that is killed by SIGABRT (signal 6), for all three
  `async-ffi` cases.** For `ffi-drop-panic-ok` the marker is
  `invoke=Ok poll=Ok`: the plugin's `invoke_future` returned `Ok` and its `poll`
  returned `Ok`, and the process still died at drop with no status returned.
  So **yes, the drop-panic path is reachable from a plugin that only ever
  returned `Ok`**. The trigger is a destructor inside the wrapped future, which
  need not be code the author wrote (any type the future holds, including a
  third-party crate's).
- **The stderr is the crate's own abort message**, `async-ffi: abort due to
  panic across the FFI boundary in Future::drop`, written by its `DropBomb`
  before `std::process::abort()` (async-ffi 0.5.1 `src/lib.rs:177-184` and
  `:551-552`).
- **The abort is not containable at all from outside the crate.** Two attempts
  are driven. First, the panic already happens *inside* the plugin's own
  `ffi_drop`, which wraps the author's drop in `catch_unwind` (the seam's
  boundary rule); the process dies anyway, and the child never reaches its
  `after-drop` line. Second, `ffi-drop-panic-thread` calls the same plugin drop
  on a worker thread inside `catch_unwind` and joins it with the handle; same
  exit status 134 and no `contained=true` line. This is consistent with the
  source: `DropBomb::drop` calls `std::process::abort`, which is not unwinding,
  so neither `catch_unwind` nor a thread boundary nor a panic hook can
  intercept it. There is no flag or feature that turns it into an error: 0.5.1's
  only features are `abi_stable` and `macros`, both off by default, and the
  `DropBomb` abort is unconditional.
- **The same panic on our own objects is contained.** `hand-drop-panic` (the
  hand-written vtable) and `async-drop-panic` and `async-drop-panic-ok` (the
  hand-rolled async object) all exit 0 and print
  `after-drop status=Ok(Ok)`, with the panic visible only on stderr.
- **A panic in `poll` through `async-ffi` is contained in-process** (check 1,
  `future.ffi_panic_poll`): first poll `Internal`, second poll `Internal`
  (terminal), drop `Ok`, process alive. This is the crate's documented
  behaviour and it works; the asymmetry with `drop` is the whole finding.

### 2. The two-implementation comparison

Full numbers are in `artifacts/footprint-and-lines.txt`. Summary:

| Axis | `async-ffi` object | hand-rolled async object | hand-written vtable |
| --- | --- | --- | --- |
| plugin-side lines (total / code / with `unsafe`) | 271 / 213 / 13 | 172 / 133 / 7 | 309 / 244 / 17 |
| ABI types needed across the boundary | identical to the others | identical | identical |
| external crates | `async-ffi` 0.5.1 (leaf) | none | none |
| cdylib footprint | 1,139,912 B, text 829,874 | (measured with the crate off: 1,117,184 B, text 817,006) | same build |
| crate-attributed symbols | 25 symbols, 2,725 B | 0 | 0 |
| ready-path per call, 20k calls (three runs) | 899 / 902 / 738 ns | 880 / 885 / 584 ns | 825 / 818 / 593 ns |
| host-timer path per call, 2k calls (three runs) | 4776 / 6069 / 7226 ns | 4530 / 6457 / 7079 ns | 4431 / 6222 / 6652 ns |
| panic in `poll` | contained | contained | contained |
| panic in a destructor inside the future | **SIGABRT, process dies** | contained | contained |

Reading it:

- **Lines.** The `async-ffi` object region is 271 lines (213 code, 13
  `unsafe`) against 172 lines (133 code, 7 `unsafe`) for the hand-rolled object
  that does the same job, both on top of 265 shared lines (waker bridge,
  instrumentation, pending map, cancel). So dropping the crate and shipping the
  helper **removes about 99 lines and half the `unsafe`** from the plugin side,
  rather than adding surface.
- **ABI types.** Zero difference. Both need `MetisFutureHandle`,
  `MetisFutureVtable`, `MetisWaker`, `MetisWakerVtable` and the payload types;
  the `cbindgen` header mentions nothing from the crate, and the cdylib exports
  exactly one dynamic symbol (`metis_plugin_entry_v1`) in both builds.
- **Footprint.** 22,728 bytes of file (2.0%) and 12,868 bytes of text (1.5%),
  and that delta also contains our own async-ffi object code (4,868 B). The
  crate's own text is on the order of 8 KB, about 1%. No new `NEEDED` library
  either way. This is small: the footprint is not a reason to drop it.
- **Compile time.** 1.201/1.311 s with and 1.210/1.365 s without, for the
  marginal recompile of the cdylib crate. Indistinguishable. The crate adds one
  package to the graph (25 versus 24) and has no dependencies of its own.
- **Per call.** Three release runs, and the honest answer is that this axis
  does not decide anything. On the ready path (20k iterations) the `async-ffi`
  object measured 899, 902 and 738 ns and the hand-rolled object 880, 885 and
  584 ns, so the within-run delta (async-ffi minus hand-rolled) was +19 ns
  (+2%), +17 ns (+2%) and +154 ns (+26%): a wash to about a quarter slower,
  with the runs taken under different machine load (the host-timer loop moved
  from 5032 ns to 7382 ns between runs). The hand-written vtable beat the `async-ffi`
  object in all three runs (825, 818, 593 ns against 899, 902, 738 ns), and that
  gap is the cost of wrapping the author's future in a `std::task::Waker`, which
  both the crate and our helper pay and the hand-written vtable does not. On the
  host-timer path the async-ffi object swung from +5% to -6% against the
  hand-rolled object between runs, so that path cannot rank them at all: the
  timer round trip is 1.7 to 2.0 us and the rest is scheduling.
- **Cost that did not separate them.** `into_ffi` allocates once and the
  crate's owned-waker clone allocates once (documented in its own docs); our
  helper's `Box::pin` allocates once and our raw-waker bridge allocates once per
  std-waker clone, so the allocation counts match on paper. Read in the source,
  not counted at runtime.

## Unproven

- **The second abort path is read, not driven.** A panic in a host waker
  callback is also fatal: every entry of the crate's owned-waker table is
  wrapped in `DropBomb::with("Waker::clone" | "Waker::wake" | "Waker::wake_by_ref"
  | "Waker::drop")` (async-ffi 0.5.1 `src/lib.rs:267, 284, 290, 297, 313, 328`),
  and the borrowed-waker table's `wake` and `drop` are a bare
  `std::process::abort` (`:333-334`). I did not pass a deliberately unwinding
  host waker into an `async-ffi` future, so that path is source-read only. It
  matters because it means the crate's safety depends on every host callback
  never unwinding, forever.
- **Platform and toolchain scope.** Driven on x86_64 Linux, Rust nightly 1.101,
  with `panic = unwind` (this workspace sets no `[profile]`, so the abort is the
  crate's, not the profile's). Not driven on macOS, Windows, or a
  `panic = abort` build.
- **A real daemon holding many plugins was not run.** The abort was measured in
  a child process that loads the cdylib and drives the ABI; the "the daemon must
  not die because of one plugin" consequence is that same fact applied to a
  process that does not fork, not a second measurement.
- **Leak behaviour of a contained destructor panic.** Contained is not the same
  as clean: if the author's destructor panics halfway, whatever it had not
  released is leaked. My probe's guard allocates nothing, so the leak was not
  measured; the hand-written object's equivalent hazard (lane 2, check 2) does
  prove that the buffer the shim took out first is still released.
- **Cold compile cost.** The devenv shell reuses compiled dependency artifacts,
  so a build from an empty cargo cache could not be measured; only the marginal
  recompile of the cdylib crate is measured. A true cold build adds one
  dependency-free crate to the graph.
- **The helper is a prototype, not an API.** What I wrote is
  `BoxedTask`/`AsyncTaskCtx`/`make_async`/`async_poll`/`async_drop` inside the
  throwaway cdylib, with the author's future typed concretely. It is not
  generic, not documented for authors, and has no `Drop`-panic leak policy.
- **No C or other non-Rust implementer was involved here.** That is lane
  ownership's question. This lane only shows that the crate is not on the wire,
  so it neither helps nor hinders a non-Rust implementer.
- **`async-ffi` versions other than 0.5.1.** The stderr string, the `DropBomb`
  wrappers and `ABI_VERSION = 2` are as read in 0.5.1.
- **The feature-off build is footprint-only.** It compiles, loads through the
  real ABI and answers `not_found` for the async-ffi-backed capability, but the
  suite is not run against it, and it advertises capabilities it cannot serve
  (`tool.poll_future`, `tool.future_ready`, `tool.future_ready_noshim` are still
  in the descriptor list). That is a measurement artifact, stated here so nobody
  reads it as a shippable configuration.

## Recommendation

**Drop `async-ffi`.** The one thing the crate does that we cannot do ourselves
is turn an author's `async` block into a pollable object, and that is 172 lines
of plugin-side code (fewer than the 271 lines the crate-backed object occupies,
and half its `unsafe`), which I wrote and drove in this copy: the author still
writes `async move { host_service.await; ... }`, the SDK wraps it with
`Box::pin(...)` instead of `.into_ffi()`, and the object, the poll shim and the
drop shim are ours, so a panic in any destructor inside the author's future
lands in our `catch_unwind` and becomes `Internal` while `drop` still returns
`Ok`. Everything else the crate might have earned is not there: the ABI delta is
zero because `FfiFuture` never crosses, the footprint delta is about 2% of a
1.12 MB cdylib of which the crate is under 1%, the compile-time delta is inside
the noise, one dependency and 25 symbols, and the per-call delta is not
separable from a hand-rolled object in the direction that matters (across three
runs the crate-backed object was a wash to about a quarter slower on the ready
path, and both shapes are slower than a pure `MetisWaker` state machine by the
cost of the `std::task::Waker` bridge they share). Against that stands the
measured asymmetry: a panic in `poll` is contained, a panic in a destructor
inside the future is SIGABRT and
nothing outside the crate can contain it, and it fires for a plugin whose
`invoke_future` and `poll` both returned `Ok`. A daemon that loads many plugins
cannot accept a per-plugin abort that our own boundary rules are supposed to
catch, so the containment rule we would have to impose if we kept the crate is
"no destructor inside any plugin future may ever panic, and no host waker
callback may ever unwind", which is not enforceable across a plugin boundary.
What an author loses if it is dropped is exactly the `.into_ffi()` call: the two
shapes are `async move { ServiceWait { shared }.await?; build(body) }.into_ffi()`
(1 line of wrapping, 271 lines of object code) against
`Box::pin(async move { ServiceWait { shared }.await?; build(body) })` (1 line of
wrapping, 172 lines of object code). A non-Rust implementer gains nothing and
loses nothing, since neither shape shows on the boundary. The daemon gains the
whole class of case back: one plugin's destructor panic becomes an `Internal`
status with a live process, the same as every other panic the seam already
contains. The minimum helper the SDK should ship is the hand-rolled async
object: accept any `Future<Output = Result<Vec<u8>, MetisStatus>> + Send +
'static`, box it behind the opaque `ctx`, poll it through the host waker bridge
inside `catch_unwind`, and in `drop` take the stored waker clone and the pending
entry out *before* the author's future is dropped and then drop the box inside
`catch_unwind`, which is what makes the drop-panic case contained and testable
(checks 7 and 8). Surface to delete: the `async-ffi` dependency, the
`FfiFuture`/`into_ffi` shape, and the crate's `DropBomb` abort. Surface to keep:
the whole ABI, `MetisFutureHandle`/`MetisFutureVtable`/`MetisWaker`/
`MetisWakerVtable` exactly as SEAM-SPEC has them, the host-waker to std-waker
bridge, the shared pending map and cancel, and the panic-in-poll containment
(the crate's `FfiPoll::Panicked` redirection is replaced by our shim, which is
already the boundary rule).

## Contract deltas

Nothing in the ABI changes. Named exactly:

ABI types: none added, none changed, none removed. `MetisFutureHandle`,
`MetisFutureVtable` (`size`, `poll`, `drop`), `MetisWaker`, `MetisWakerVtable`
(`size`, `clone`, `wake`, `wake_by_ref`, `drop`), `MetisFuturePollFn`,
`MetisFutureDropFn` and the appended table entries `invoke_future`,
`service_complete`, `service_request` all stay as written in `SEAM-SPEC.md`,
which is what makes this a delete rather than a rewrite.

Host table and plugin table entries: none added, none removed.

Host-side trait items: none. `FutureHost`/`HostFuture` are unchanged by this
lane except one accessor I added for the containment attempt,
`HostFuture::raw_parts()`, which is probe-only and would not ship.

Plugin-side (SDK) items to add if the crate is dropped:

- a public task-object helper, name it `FfiTask` (or `into_task_object`), that
  takes any `F: Future<Output = Result<Vec<u8>, MetisStatus>> + Send + 'static`,
  boxes it, and hands back `MetisFutureHandle` with our `poll`/`drop` vtables;
- the poll shim: `catch_unwind` around the author's `poll`, a panic becoming
  `MetisStatus::Internal` and leaving the object terminal;
- the drop shim: take the stored waker clone and the pending-map entry out
  first, then `catch_unwind` around the box drop, always returning `Ok`.

Plugin-side items to remove if the crate is dropped:

- the `async-ffi` dependency;
- `FfiFuture` / `FutureExt::into_ffi` / `async_ffi::FfiPoll` and the crate's
  `DropBomb` abort from the plugin object path;
- the `tool.future_ready_noshim` capability, which exists only to measure the
  share of our own `catch_unwind` shim on the async-ffi object and has no other
  use (the same measurement on the helper is a one-line change to the helper's
  vtable, not a capability).

Open operator decisions, recorded rather than settled here:

1. Whether the SDK's task helper is generic over the author's output type or
   fixed at `Result<Vec<u8>, MetisStatus>` (the ABI only carries bytes, so the
   fixed form is enough today, but the typed facades may want more).
2. What the helper does with a halfway-panicked destructor: contain and report
   `Internal` (what I drove), or contain plus a leak-attribution record. The
   leak itself cannot be reclaimed without an author-side `Drop` contract.
3. Whether the host should also refuse to load, or quarantine, a plugin that
   ships its own abort-on-panic wrapper. Nothing in our ABI can tell.
