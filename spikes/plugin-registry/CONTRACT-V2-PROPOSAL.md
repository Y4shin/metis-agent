# Plugin contract v2: proposal for approval

Throwaway prototype material, but this document is the shape the operator asked
to approve before the prototype folds it in. It is written to be annotated: mark
each decision keep, change, or reject, and add what you want different.

The north star for the round was: **the plugin just gives the host async tasks
that the host can launch and await on its own runtime.** Everything below is
measured against that, and "delete" is preferred over "add".

Evidence lives in `FINDINGS4.md` and `round-3/`. Every number in the decisions
was re-run by the parent, not relayed from a lane.

## D1. One call path: a clean v2 table with no legacy callbacks

**Propose:** delete `invoke` and `complete` outright and make the futures path the
table's core, as a clean v2 rather than another append. `invoke_future`,
`service_complete` and `service_request` stop being appended entries and become
the table.

**Evidence:** with the legacy path compiled out, the suite is green at 20 checks
in the OFF configuration against 95 in ON, and the release cdylib drops 40 percent
(1,554,520 to 924,328 bytes), the debug cdylib 80 percent, the release host 7
percent. The dependency closure loses two crate edges.

**Cost:** a recompile of every plugin, and A11's "append now, v2 when the future
seam replaces `Pending`/`complete`" condition is exactly what has now happened, so
this is A11's own trigger, not a reversal. Both protocols coexisting is what
carries the surface the 40 percent measures.

**Rejected alternative:** keep the append and the dual protocol for one more
cycle. It keeps the landed `HostAdapter` and the executor seam alive, which is
where most of the deleted surface is.

## D2. One authoring method, routed by capability name

**Propose:** replace the four-arm vocabulary (`ANSWER`, `DEFERRED`,
`CANCELLABLE`, `FAULT` consts plus four associated `Shape` types) with one
`call`, one request shape and one response shape:

```rust
pub struct Request<'a> { pub capability: &'a str, pub body: &'a [u8] }
pub struct Response { pub body: Vec<u8> }

pub trait Plugin: Default + Send + Sync + 'static {
    fn capabilities(&self) -> Vec<CapabilityDescriptor>;
    fn call(&self, call: Call, request: Request<'_>)
        -> impl Future<Output = Result<Response, PluginError>> + Send;
    fn cancel(&self, call: Call) -> Result<(), PluginError> { Ok(()) }
    fn started(&self, host: &Host) -> Result<(), PluginError> { Ok(()) }
    fn stopped(&self, host: &Host) {}
}
```

**Evidence:** the four-arm shape caps a plugin at four capabilities, which is the
defect blocking the SDK from pinning author-facing API. A routed prototype with
eight capabilities passes 4 of 4 tests including `the_capability_cap_of_four_is_gone`.
The compiler, not the lane, decides dyn compatibility: four-arm fails `E0038`
plus `E0191`, routed fails `E0038` as a return-position `impl Trait` and compiles
clean once boxed.

**Cost:** the authoring surface is bytes plus a capability name, so per-capability
typing has to come from the SDK's schema-derived facades instead of associated
types. That is a real ergonomics debt and it is the thing to argue about.

**Open sub-decision:** whether the erased `call` should ship alone, or ship with a
generated typed facade as the recommended author experience from day one.

## D3. Drop `async-ffi`; the SDK owns the future object

**Propose:** remove the `async-ffi` dependency and ship one SDK helper that boxes
any `Future<Output = Result<Vec<u8>, MetisStatus>> + Send`, polls it through the
host waker bridge inside `catch_unwind`, and takes the waker clone and pending
entry out before dropping the box inside `catch_unwind`.

**Evidence:** reproduced in the parent. A destructor panic in a plugin future
aborts the daemon with SIGABRT (signal 6) through the crate, in all three cases
tried, **including a plugin whose poll returned `Ok`**. The shim's own
`catch_unwind` is already in that path and does not help, so no host-side
containment exists. The hand-written object contains the same panic and exits 0.
The crate is not on the wire, costs 2.0 percent of the cdylib, and does not win
per call (899/902/738 ns against 825/818/593 ns).

**Cost:** authors lose `.into_ffi()` and use `Box::pin(...)`; the helper is 99
lines smaller and has half the `unsafe` of the crate path.

**Rejected alternative:** keep the crate and forbid panicking destructors in
plugin futures. That rule cannot be enforced across a plugin boundary, so it
would be a documented way for any third-party plugin to kill the daemon.

## D4. Unload rule: quarantine, with one reclaim on a late poll

**Propose:** adopt rule (b) quarantine. `abandoned_active` is a single field with
two jobs: the leak counter and the quarantine predicate. New futures are refused
while it is nonzero, the mapping stays resident, unload is allowed only when it
clears, and a late poll return reclaims the `ctx` box and the waker clone exactly
once. Delete the fixed 900 ms wait.

**Evidence:** reclaim proven once-only (24-byte late buffer released,
`blocking_polls_started/returned = 1/1`, double reclaim refused, poll and drop
after reclaim refused), whole-run balance `futures_created == futures_dropped ==
62021` and `waker_clones == waker_releases == 46031`, zero leaks, and the unload
ledger refusing then allowing in the same run. ABI delta zero.

**Cost:** one `AtomicUsize`, two driver fields, three per-`HostFuture` fields and
one heap signal per watchdogged poll. A poll that never returns quarantines the
plugin forever and leaks one thread and one signal, by design and safely.

**Open sub-decision:** who runs reclaim in the real daemon. The prototype runs it
on the caller; a daemon wants a reaper thread, which is designed but not built.

## D5. The service entries get contract text, not folklore

**Propose:** write the ownership clause as contract text: for the ready
`out_value` buffer, for `service_request` and for `service_complete`, who
allocates, who frees, with which call, and when the pointer stops being valid;
plus the five rules the C implementer had to guess. Put the envelope constants
and presence predicates in the generated C header.

**Evidence:** a hand-written C plugin (1072 lines, gcc and clang, zero
diagnostics under `-Wall -Wextra -Werror`, one dynamic symbol) drove the ready
path, a host-answered timer, cancel and a drop with the reply in flight, and its
ledger read back after unload is `allocs=12 frees=12 live=0 double_frees=0`,
`waker_clones=3 waker_releases=3`, `futures_created=10 futures_freed=10`.

**The five rules the contract never stated:** the payload envelope byte, the
one-call request lifetime, the borrowed-but-by-value waker, two differently sized
host tables (`48` and `56`), and `release_buffer` being reentrant inside
`service_request`.

**Consequence found by the same lane:** the landed Rust `service_complete` frees a
plugin-owned answer, which contradicts the clause. Fix in `plugin-contract`.

## D6. Delete the two unread `size` fields

**Propose:** `MetisFutureVtable.size` and `MetisWakerVtable.size` are written and
never read, so a wrong-size vtable is undetectable. Either read them as presence
predicates or delete them.

**Evidence:** the C implementer had to fill them in and got no benefit; the
lane's table dump shows both sizes present and no reader.

**Also in scope:** the landed `HostAdapter::invoke` reads and releases an inline
`Ok` payload but does not return it. If D1 lands, `HostAdapter` is deleted and
the pending-slot leak (open item O7) disappears with it, so the O7 ticket is
moot rather than fixed.

## D7. Move the `metis.*` control plane onto the futures path

**Propose:** re-express the `metis.*` control calls (schemas, configure, counters)
as capabilities on `invoke_future`, so `invoke` can be structurally deleted
rather than left in place as an inline control channel.

**Evidence:** the shape lane could compile the legacy path out entirely, but in
the prototype `invoke` survives as an inline `metis.*` control plane because the
landed table type is fixed. This is the one place the deletion is not yet
complete, and the lane names it as an open operator decision.

**Cost:** the control plane becomes asynchronous like everything else, so
`create_instance` cannot synchronously ask for a schema list during that call.
Worth deciding deliberately.

## What this does not settle

- No C implementer was built for the v2 table; the C evidence covers today's
  table. If D1 and D2 are approved, the C implementer should be rebuilt against
  v2 as the acceptance test for the new shape.
- Real daemon executor integration: the host timer is still one thread per driver
  and serializes `service_request`.
- Packaging, signature checks, hot unload, schema evolution, large and nested
  payloads, and the error taxonomy (open item O8) are untouched by this round.
- Whether the entry symbol becomes `metis_plugin_entry_v2` and how the major
  version bump interacts with the frozen status space.
