---
type: findings
title: Plugin authoring trait spike
status: stable
---

# Plugin authoring trait spike

Question: does a throwaway Rust authoring trait, roughly what `plugin-contract`
will ship, translate into the primitive ABI through a thin wrapper without
dragging trait concepts across the boundary?

Answer: yes, it translates, and the trait module itself never mentions the ABI
(no `spike_abi` import, no `Metis*` type). The translation is mechanical, but it
is not small for a four method trait, and the trait bends toward the ABI in nine
places, catalogued below. The spike is conclusive, not abandoned.

Source: `spikes/plugin-authoring-trait/` (three crates: `spike-authoring`, the
trait plus the hand-written translation both ways; `spike-trait-plugin`, a
`cdylib` that implements the trait; `spike-trait-consumer`, a binary that loads
it and drives requests through the host adapter). Throwaway prototype code, not
product code, not a member of the root workspace, not wired into CI. The ABI
primitives are the committed ones from `spikes/plugin-abi-mechanics`
(`spike-abi`, `spike-payload`), depended on by path; nothing under
`spikes/plugin-abi-mechanics` was modified.

## Environment

| Tool | Version |
| --- | --- |
| cargo | 1.101.0-nightly (29c5daa1a 2026-10-07) |
| rustc | 1.101.0-nightly (080cc2714 2026-10-11) |
| libloading | 0.9.0 |
| ABI under test | the committed v1 table from `spikes/plugin-abi-mechanics`, 72 bytes, `metis_plugin_entry_v1` |
| Host | Linux x86_64 |

All commands were run through `devenv shell`.

## The trait that was tried

`spike-authoring::traits::Plugin`, 33 code lines (73 raw), in
`crates/authoring/src/traits.rs`:

```rust
pub trait Plugin: Send + Sync + 'static {
    fn capabilities(&self) -> Vec<CapabilityDescriptor>;

    fn echo(&self, call: Call, request: EchoRequest) -> impl Future<Output = Result<EchoResponse, PluginError>> + Send;

    fn complete(&self, call: Call, request: CompletionRequest) -> impl Future<Output = Result<CompletionResponse, PluginError>> + Send;

    fn uncancellable(&self, call: Call, request: EchoRequest) -> impl Future<Output = Result<EchoResponse, PluginError>> + Send;

    fn cancel(&self, call: Call) -> Result<(), PluginError>;
}
```

Three capability methods plus a cancellation method, addressed through one
generic `invoke` by capability name (`tool.echo`, `provider.complete`,
`tool.uncancellable`), with two capability kinds in play (D9). The plugin crate
implements it in 51 code lines, of which the entire ABI wiring is six lines:

```rust
static TABLE_V1: MetisPluginTableV1 = table::<Tools>();

#[unsafe(no_mangle)]
pub extern "C" fn metis_plugin_entry_v1() -> *const MetisPluginTableV1 {
    &raw const TABLE_V1
}
```

That works because `table::<P: Plugin + Default>()` is a `const fn` that
instantiates the seven exported adapters monomorphised on `P`. No macro is
needed at this size, which answers the hand-written versus generated
alternative: the per-table-entry boilerplate is written once, generically, not
once per plugin. The remaining hand-written per-plugin cost is the trait
implementation itself.

## Translation size in lines

Non-blank, non-`//` lines (raw counts in parentheses), measured with the
command in the reproduction section:

| Part | Crates | Code lines |
| --- | --- | --- |
| Plugin side adapter (`plugin_adapter.rs`) | authoring | 287 (374) |
| Host side adapter (`host_adapter.rs`) | authoring | 376 (474) |
| Plugin buffer ownership, D6 (`alloc.rs`) | authoring | 61 (80) |
| Minimal async executor (`executor.rs`) | authoring | 57 (81) |
| Per-request context (`call.rs`) | authoring | 52 (93) |
| The trait (`traits.rs`) | authoring | 33 (73) |
| Payload shapes added here (`payload.rs`) | authoring | 17 (28) |
| Consumer plugin, trait implementation | plugin | 51 |
| Consumer plugin, table static and entry point | plugin | 6 |
| Consumer driver and checks | consumer | 199 (257) |

So the translation is 833 code lines (1102 raw) for both directions, against a
33 line trait and a 51 line plugin implementation. Broken down:

* 106 lines are the four host callbacks, each wrapped in `catch_unwind`, each
  having to copy a plugin payload, validate the D4 header, and release the
  buffer through the plugin's allocator.
* 51 lines are counters, logs, and the accounting parser that only exist to let
  this spike's checks observe the run. A real host would not put them inside the
  adapter.
* 25 lines are the dispatch match, the only place the ABI knows about the
  trait's methods: three arms, about eight lines per capability.
* 12 lines are the error to status mapping.
* 52 lines are `Call`, which exists because the ABI is asynchronous and out of
  band rather than because the trait wants a context object.
* The rest is the pending registry, the worker list, the seven exported
  adapters, the slot and await machinery, the D6 allocator, and the executor.

The honest reading: 782 code lines of translation excluding the spike's own
observation scaffolding, of which roughly 90 lines are the parts that are
actually about the trait (dispatch, error mapping, context). Everything else is
ABI mechanics that any authoring layer would have to carry, since no trait can
abstract away buffer ownership, an out-of-band completion callback, or panic
containment. The claim "the ABI is decoupled from the trait" is supported on the
trait side (`traits.rs` is clean) and costs about 90 lines on the adapter side
plus a fixed 690 lines of ABI plumbing.

Marginal cost of one more capability: one line in `capabilities()`, one method
on the trait, and roughly eight lines in the dispatch match. No ABI change, no
table change, no version bump. The per-method function table alternative would
put one entry per capability into the ABI table, which is append-only (D8), so
each plugin's capability set would either grow the shared table or force a major
version to retire an entry, and the host would still have to match the
capability name to the entry. The generic `invoke` costs one 25 line match and
keeps the ABI independent of the capability set.

## Where the trait bent toward the ABI

1. **`Call` exists only because of the ABI.** Every method takes a per-request
   context carrying a request id, a cancellation flag, and a progress route. A
   plain Rust trait would take `&self` and the request. This is the largest
   single addition to the authoring surface (52 lines).
2. **The trait dictates the executor's threading model.** `Send + Sync +
   'static` on the plugin and `+ Send` on every capability future, because
   `invoke` must return pending (D11) and the adapter therefore moves the future
   onto a worker thread. The trait says nothing about executors but rules out any
   plugin whose future borrows something non-`Send`.
3. **An async trait cannot produce D11's inline completion.** Because a trait
   `async fn` (RPITIT) has no way to hand back a synchronous result, the adapter
   always returns `Pending`, and one worker thread plus one callback is the only
   path per call. Measured: five requests, five pending returns, zero inline
   returns, zero buffers on pending returns. Recovering the inline shape would
   need the adapter to poll the future once under `invoke` and fall back to
   pending, which changes the call's cost model in a way the trait does not
   express today.
4. **Cancellation is a poll flag plus an adapter registry, not a trait
   concept.** The ABI's `cancel(request_id)` has no context, so the adapter keeps
   `id -> Call` and hands the plugin a rebuilt `Call`, which is not the same
   object that the capability method received. Anything the plugin stored
   outside `Call` is invisible in `cancel`. There is also no waker: the plugin
   must poll `Call::is_cancelled()`, so a plugin blocked in I/O will not notice a
   cancellation promptly.
5. **The error type is the ABI's status space.** `PluginError` mirrors
   `MetisStatus` minus `ok` and `pending`, and one of its variants carries
   protocol instructions: after a successful cancel, returning
   `PluginError::Cancelled` makes the adapter suppress the completion (D12). A
   control-flow decision rides on an error variant because the ABI's
   cancel/completion race has nowhere else to express "I am dropping this".
6. **The authoring surface is a serde surface.** Requests are decoded and
   responses encoded by the adapter, so the trait's request and response types
   must be `DeserializeOwned` and `Serialize`, and `Call::progress` is generic
   over `Serialize`. The trait cannot be generic over arbitrary Rust types, and
   serde becomes a hard dependency of the authoring layer, which makes D3's
   placement of serialisation load-bearing.
7. **The descriptor names and the dispatch names are separate declarations.**
   The plugin reports `tool.echo` from `capabilities()` while the adapter
   dispatches on the string literal `"tool.echo"` in its match. Nothing checks
   that the two sets agree; a missing arm surfaces only as `not_found` at call
   time. One fix to record: a single table of `(name, kind, version, handler)`
   per plugin, or a check at instance creation that every descriptor has an arm
   and every arm has a descriptor.
8. **The adapter needs `Default`, and the trait has no lifecycle hook.** The
   table is instantiated as `table::<P: Plugin + Default>()`, so construction
   cannot fail and cannot take configuration. There is no `started` or `stopped`
   hook either: the ABI's `emit_event` had no route through the trait, and this
   spike therefore emitted zero events (`event_callbacks = 0`) where the
   mechanics spike used `emit_event` for a ready event.
9. **Process-wide mutable state moves into the adapter.** Two `OnceLock`
   registries (pending requests, worker handles) plus two `AtomicPtr`s (host
   table, and later the host's own plugin table) and the allocation ledger are
   statics, not per-instance state. This is a placement bend rather than a
   signature bend, and it means two instances of one plugin share one pending
   registry. It is safe here only because request ids are host-supplied and
   unique.

Not leaks, for balance: the D4 payload header, the buffer owner tag, the
`release_buffer` contract, the length and capacity fields, the panic guard, and
the thread hop are all invisible to the author. `traits.rs` imports only `std`
and payload types.

## What the async path did, end to end

Always pending, never inline, as the task requires:

| Request | Capability | Result |
| --- | --- | --- |
| 1 | `tool.echo` | `invoke` returned `pending`, the plugin's future resolved on a worker, `complete` carried `ok` plus `{"echo":"hihihi","bytes":6}`, the consumer's awaiting task resolved |
| 2 | `provider.complete` | `pending`, one `progress` callback before the completion, `complete` carried `ok` plus `{"text":"hello (8)"}` |
| 3 | `provider.complete` | `pending`, cancel accepted, the plugin observed the flag after its delay, returned `Cancelled`, the adapter suppressed the completion, nothing reached the host |
| 4 | `tool.uncancellable` | `pending`, cancel accepted, the future completed anyway, the host dropped the later completion and still released its buffer through `release_buffer` |
| 5 | `tool.echo` after the cancellations | `pending`, `ok`, instance healthy |

Both halves of D12's revised rule are therefore exercised in one run: the plugin
may suppress its completion after a successful cancel (request 3), and the host
must drop a completion that arrives anyway (request 4), including releasing the
buffer of a payload it discards. The consumer's awaiting task is a real future
polled by the spike's own `block_on`, woken by the `complete` callback through a
waker parked in the adapter's slot, not a blocking sleep. The consumer's checks
that expect nothing to arrive use bounded waits (600 ms) plus a 30 s watchdog
thread, because the mechanics spike found that an unbounded wait on a completion
that never comes hangs forever.

Accounting cross-check on both sides of the boundary: the plugin reported
`allocs == frees`, `live=0`, `violations=0`, `suppressed=1`, and the host's own
release counter equalled the plugin's allocation counter, so every plugin buffer
was released by the plugin.

44 runtime checks passed and the consumer exited zero. The cdylib exports
exactly one symbol, `metis_plugin_entry_v1`.

## Reproduction

From the repository root, with `devenv` on PATH:

```sh
# 1. build the whole spike. This MUST precede the consumer run: `cargo run -p`
#    does not rebuild the cdylib member, and the consumer would load a stale
#    library. Re-measured here: after touching crates/plugin/src/lib.rs, the
#    consumer run left libspike_trait_plugin.so's mtime unchanged.
devenv shell -- cargo build --manifest-path spikes/plugin-authoring-trait/Cargo.toml

# 2. drive every request through the host adapter. Prints a PASS/FAIL line per
#    check, a summary line, and exits non-zero when anything failed.
devenv shell -- cargo run -q --manifest-path spikes/plugin-authoring-trait/Cargo.toml -p spike-trait-consumer

# 3. what the cdylib exports (one symbol, as D8 requires). `nm` is only on PATH
#    inside the devenv shell.
devenv shell -- nm -D --defined-only spikes/plugin-authoring-trait/target/debug/libspike_trait_plugin.so

# 4. the line counts quoted above
cd spikes/plugin-authoring-trait
for f in $(find crates -name '*.rs' | sort); do
  printf "%-45s raw=%4s code=%4s\n" "$f" "$(wc -l < "$f")" \
    "$(awk '/^[[:space:]]*$/{next} /^[[:space:]]*\/\//{next} {n++} END{print n+0}' "$f")"
done
```

`SPIKE_TRAIT_PLUGIN_PATH` points the consumer at a different `.so`.

## Remaining uncertainty

* The executor is this spike's own `block_on` (thread plus park/unpark) and a
  timer future, not Tokio. The callback to waker bridge is exactly the piece
  `metis-plugin-sdk` still owns, and a real runtime would additionally need to
  bound the wait without the watchdog thread used here.
* The trait's `+ Send` bound and the worker thread per call are untested against
  a plugin that needs real concurrency control. Cancellation is poll-style, and
  no test covers a cancel arriving in the window between the future resolving
  and the completion callback firing, which is the race D12 describes.
* One instance, one plugin, one consumer thread apart from plugin workers.
  Concurrent invokes, multiple instances, and reentrancy were not exercised, and
  multiple instances would share the adapter's pending registry (see bend 9).
* `Delay`'s timer thread is not tracked at shutdown. Every request here is
  drained before `shutdown`, so no timer thread could wake after `shutdown`
  returned and before the host unloaded the library. That ordering is a real
  constraint on a host, not a property of the adapter.
* A panic inside a trait method is not exercised. The mechanics spike proved
  `catch_unwind` in exported adapters; here such a panic would kill the worker
  thread and leave the host's pending record unfilled until a bounded wait
  expires, which is untested.
* The host adapter is the consumer's own thin adapter, not `metis-plugins`. Real
  host concerns (capability version gating per D5 and D10, progress fan-out,
  cancellation accounting under concurrency) were covered by the mechanics spike
  or left out entirely; this spike's host does not gate capability versions.
* Line counts are code lines (blank and `//` lines excluded) and include
  `use` blocks. They give scale and a per-capability marginal cost, not a
  threshold.
* `HostAdapter::new` leaks the adapter on purpose so the host table cannot move.
  A real host needs a cleanup path.
* Linux x86_64 only. Windows loader and allocator questions are untouched, as in
  the mechanics spike.
* The comparison against a macro-generated adapter and against a per-method
  function table is argued from the measured churn, not built as a second
  artifact. The per-plugin ABI wiring is already six lines, so a macro would have
  to earn its place on the trait implementation, not on the table.

## Verdict

Conclusive. A trait with several capability methods and a cancellation method
dispatches through one generic `invoke` addressed by capability name, the async
path works end to end and always pending, and the trait module stays free of ABI
types. The translation is mechanical rather than small: about 780 code lines, of
which about 90 are the parts that know about the trait. The trait bends in the
nine places above, and the two that `plugin-contract` should decide on before
pinning the authoring surface are cancellation (bend 4: poll flag and rebuilt
context, versus a cancellation future or signal) and the inline completion (bend
3: an async trait cannot return a synchronous result without a first-poll
attempt under `invoke`).
