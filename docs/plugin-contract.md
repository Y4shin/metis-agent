# The metis plugin contract (v1)

Status: approved by the operator on 2026-10-13 as good enough for a v1, on the
understanding that real use will surface what is wrong with it. This is the
contract text, not a proposal against an earlier shape.

Evidence base: the throwaway prototype at `spikes/plugin-registry`, with
`FINDINGS4.md` and `round-3/` for the measurements, and
`docs/plugin-registry-handoff.md` for the working history. Section 14 records
what changed relative to the shape the prototype carried, compressed. Section 15
lists what is open for implementation.

## 1. The idea

A plugin is a native shared library (`.so`, `.dylib`, `.dll`) that the daemon
loads into its own process. It advertises named **capabilities**, which are the
tools the daemon can call. Calling one works like this:

- the daemon calls `invoke_future` for a capability;
- the plugin hands back a **future**: an opaque, plugin-owned context plus a
  small C vtable;
- the daemon polls that future on its **own** async runtime and awaits the
  answer;
- the daemon drops the future when it is done.

So a plugin needs no async runtime, no executor, and no worker thread. The daemon
owns when plugin code runs, which is also what makes unloading safe: the plugin
is never executing code the daemon does not know about.

If a plugin must wait (a timer, IO readiness, a child process), it does not block
and it does not spawn. It asks the host through `service_request`, returns
`Pending` from its poll, and wakes the host's waker when the answer arrives
through `service_complete`.

## 2. Vocabulary

| Term | Meaning |
| --- | --- |
| instance | One loaded, configured occurrence of a plugin. A plugin may have several. |
| capability | A named callable, declared by the plugin. |
| descriptor | Name, version, and the JSON Schema for one capability's request and response. |
| call | One invocation: a capability, a request id, request bytes, and a per-call context. |
| request_id | A `u64` the host chooses. Unique among in-flight calls, and the key for cancel, progress and services. |
| host table | The daemon's callback table, passed to the plugin. |
| service | A host facility the plugin awaits: today a timer, later IO. |
| future | The plugin's pending answer: opaque `ctx` plus poll/drop vtable. |
| waker | The host's "wake me and poll again" object, handed to the plugin during a poll. |
| buffer | A length-carrying byte span with an explicit owner. |
| envelope | One header byte in front of every payload, versioned and codec-tagged. |

## 3. What may cross the boundary

Only pointers, integers, function pointers, and byte buffers. No Rust struct with
private fields crosses. Two size-prefixed tables (plugin table, host table), and
two vtables carried by pointer (future, waker).

Both tables are **size-prefixed**: the reader checks `size` and only touches the
entries that size covers. Presence is decided by size, never by whether a slot's
pointer looks non-null.

## 4. The export

A plugin exports exactly one symbol. The daemon resolves it, checks `size` and
the ABI version, and reads the table.

```
metis_plugin_entry_v1() -> *const MetisPluginTableV1
```

The symbol and the ABI major stay `v1` and 1, because this revision lands before the first release (section 15).

## 5. The plugin table

| Entry | Shape | Contract |
| --- | --- | --- |
| `size` | `usize` | Bytes of the table the plugin provides. Read first, always. |
| `abi_major`, `abi_minor` | `u32` | Major is part of the symbol name; minor is advisory. |
| `create_instance` | `(host: *const MetisHostApiV1, out_instance: **mut c_void) -> MetisStatus` | Builds an instance, stores the host table, runs the plugin's `started`. No instance on failure. |
| `destroy_instance` | `(instance) -> MetisStatus` | Releases the instance. Only valid after `shutdown`. |
| `list_capabilities` | `(instance, out_payload: *mut MetisBuffer) -> MetisStatus` | Returns a plugin-allocated payload listing capabilities, each with its name, version, and request and response schemas. |
| `invoke_future` | `(instance, capability: *const u8, capability_len: usize, request_id: u64, payload: MetisBuffer, host: *const MetisHostApiV1, out: *mut MetisFutureHandle) -> MetisStatus` | The only call entry. On success `*out` holds a plugin-owned future. The request payload stays valid for the call only. |
| `cancel` | `(instance, request_id: u64) -> MetisStatus` | Marks that call's cancellation token. The plugin's future is expected to resolve as `Cancelled` rather than be dropped mid-flight. |
| `release_buffer` | `(buffer: MetisBuffer) -> MetisStatus` | The host frees a plugin-allocated buffer by calling this. The host never frees plugin memory itself. |
| `shutdown` | `(instance) -> MetisStatus` | Quiesces the instance. The host drops every future before calling it, so nothing may arrive after it returns. |
| `service_complete` | `(instance, request_id: u64, status: MetisStatus, payload: MetisBuffer) -> MetisStatus` | The host's answer to an earlier `service_request`. The plugin wakes its future's waker from here. |

## 6. The host table

| Entry | Shape | Contract |
| --- | --- | --- |
| `size` | `usize` | Bytes of the table. Scope of the callback set. |
| `user_data` | `*mut c_void` | Host state, passed back verbatim to every callback. |
| `progress` | `(user_data, request_id, payload: MetisBuffer) -> ()` | Optional intermediate output while a call is still running. |
| `emit_event` | `(user_data, topic: *const u8, topic_len: usize, payload: MetisBuffer) -> ()` | Fire-and-forget notification, not tied to a call's completion. |
| `log` | `(user_data, level: i32, message: *const u8, message_len: usize) -> ()` | Levels 0 to 3 (debug, info, warn, error). |
| `service_request` | `(user_data, service: *const u8, service_len: usize, request_id: u64, payload: MetisBuffer) -> MetisStatus` | The plugin asks for a timer or IO. The host answers later through `service_complete`, with the same `request_id`. |

There is deliberately no `complete` callback. A call completes when a poll
returns `Ok`; a callback that duplicates that would be a second protocol for the
same event.

## 7. The future

```c
typedef struct MetisFutureHandle { void *ctx; const MetisFutureVtable *vtable; } MetisFutureHandle;

typedef struct MetisFutureVtable {
    MetisStatus (*poll)(void *ctx, MetisWaker waker, MetisBuffer *out_value);
    MetisStatus (*drop)(void *ctx);
} MetisFutureVtable;
```

`poll` returns exactly one of three things:

- `Ok`: the call is done and `out_value` holds a **plugin-allocated** buffer with
  the response, released later through `release_buffer`.
- `Pending`: not done yet. The plugin will call `waker.wake` when it is. Polling
  again before a wake is allowed but wasteful.
- any other status: the call failed and the future is terminal. The host stops
  polling and drops it.

`drop` is called by the host exactly once, and never while a poll is in progress.

## 8. The waker

```c
typedef struct MetisWaker { void *data; const MetisWakerVtable *vtable; } MetisWaker;

typedef struct MetisWakerVtable {
    MetisWaker (*clone)(const MetisWaker *waker);
    void (*wake)(MetisWaker waker);
    void (*wake_by_ref)(const MetisWaker *waker);
    void (*drop)(MetisWaker waker);
} MetisWakerVtable;
```

The waker the host passes is valid until the future is dropped. The plugin may
clone it (each clone is one `clone` call) and must `drop` every clone. `wake` may
be called from any thread, must be cheap and non-blocking, and may arrive after
the future was dropped, which the host detects and records rather than crashing
on.

## 9. Status space, buffers, payloads

`MetisStatus`: `Ok`, `InvalidArgument`, `NotFound`, `PermissionDenied`,
`Cancelled`, `Internal`, `Unsupported`, `Pending`. This space is frozen; the
registry layer adds meaning on top of it rather than adding values.

`MetisBuffer { ptr, len, cap, owner, reserved }` with `owner` one of `NONE`,
`HOST`, `PLUGIN`. **Each side frees only what it allocated.** The owner field
says who must free, and the other side never does.

Every payload is wrapped in a one-byte envelope: the low nibble is the version
(currently 1), bit 4 is the codec (0 is JSON), and bits 5 to 7 are reserved and
must be zero. The envelope constants and the presence predicates belong in the
generated C header, not in a plugin author's head.

## 10. A call end to end

**Ready on the first poll.** The daemon picks a capability, allocates a request
id, and calls `invoke_future`. The plugin builds its `ctx` and returns `Ok` with
a future handle. The daemon polls once: `Ok` plus a plugin-allocated response
buffer. The daemon reads the bytes, calls `release_buffer`, and calls the
handle's `drop`.

**Needing a timer.** Same start, but the first poll returns `Pending`. The plugin
has already called `service_request("timer", ...)` with the call's request id and
returned. The host arms the timer and parks, rather than blocking. When it fires,
the host calls the plugin's `service_complete` with the same request id, the
plugin's waker is woken, and the daemon polls again. This time `Ok` plus the
response. Drop, release, done.

**Cancelled.** The daemon calls `cancel(request_id)`. The plugin's future
resolves as ready with `Cancelled`, so the daemon polls once more, observes the
terminal status, and drops. If the plugin misses the deadline the daemon may
still drop, but it records a protocol violation.

## 11. The rules

1. **Ownership.** Per entry, one allocator and one freer: ready buffers are
   plugin-allocated and freed through `release_buffer`; request payloads are
   host-owned and valid for the call only; a `service_request` payload is
   plugin-owned and freed by the host through `release_buffer`; a
   `service_complete` payload is host-owned and borrowed for the call.
   `release_buffer` is reentrant: the host may call it from inside
   `service_request`.
2. **Single poller.** Only the daemon polls a future, one poll at a time. Two
   concurrent polls on one handle is a protocol violation.
3. **No blocking in poll.** A poll returns `Pending` and asks for a service. A
   poll that blocks is detected by a watchdog, recorded, and the plugin is
   quarantined.
4. **Waker liveness.** A wake from any thread is legal. A wake after the future
   is dropped is a recorded violation, never a dangling dereference.
5. **Panic containment.** The SDK's poll and drop shims wrap the author's future
   in `catch_unwind`. A panic becomes `Internal`, leaves the future terminal, and
   drop stays legal. No path may reach `process::abort`. Host callbacks are
   panic-guarded the same way.
6. **Quarantine, then unload.** A plugin with an abandoned, still-running poll is
   quarantined: no new futures are accepted from it, its library stays resident,
   and its counters stay honest. Unload happens only when nothing is inside
   plugin code and every future and waker clone has been dropped. There is no
   fixed wait.
7. **Teardown order.** Stop producers, join, drop every future and waker clone,
   then unload. The ordering is observable, and the host proves it at shutdown.
8. **Versioning.** Tables are size-prefixed and presence is decided by size.
   Capability names are instance-qualified as `<instance-name>.<tool-name>`, and
   version gating is per capability.
9. **Validation both directions.** The host validates a request against the
   capability's published schema (`invalid_argument` on violation) and validates
   a plugin's response against the plugin's own published schema (an internal
   error attributed to that plugin, with no quarantine).

## 12. The Rust authoring surface

```rust
pub struct Request<'a> { pub capability: &'a str, pub body: &'a [u8] }
pub struct Response { pub body: Vec<u8> }

pub trait Plugin: Default + Send + Sync + 'static {
    /// Everything this plugin advertises, with schemas.
    fn capabilities(&self) -> Vec<CapabilityDescriptor>;

    /// One capability, routed by name. One request in, one response out.
    fn call(&self, call: Call, request: Request<'_>)
        -> impl Future<Output = Result<Response, PluginError>> + Send;

    /// A cancel was accepted; the call's token is already marked.
    fn cancel(&self, call: Call) -> Result<(), PluginError> { Ok(()) }

    /// Instance start and stop.
    fn started(&self, host: &Host) -> Result<(), PluginError> { Ok(()) }
    fn stopped(&self, host: &Host) {}
}
```

An author writes `async move { ... }` and decodes the request by capability name.
Per-capability typing comes from the SDK's schema-derived facades, so the trait
itself stays one method with no per-capability associated types.

## 13. Who owns what

**The daemon owns:** the poll loop and its runtime, request id allocation policy,
cancel deadlines, the unload decision, teardown ordering, and the registry
(routing, version gating, schemas, enable and disable).

**The SDK owns:** the waker bridge between the host's waker and `std::task`,
`catch_unwind` at poll and drop, consuming the ready buffer on the poll return,
cancel-with-deadline mechanics, schema derivation, typed facades, and the
boxed-future helper that replaces `async-ffi`.

## 14. What changed relative to the shape the prototype carried

1. `invoke` and `complete` are gone: futures are the only call path, as a clean
   revised v1 table. The legacy path and the executor seam it needed are what the 40
   percent release-cdylib reduction measures.
2. The four-arm authoring vocabulary (four capability consts, four associated
   request types) became one `call` routed by capability name, which removes the
   four-capability ceiling.
3. `async-ffi` is dropped: a destructor panic inside a plugin future aborts the
   daemon with SIGABRT even for a plugin that returned `Ok`, and no host-side
   `catch_unwind` can contain it. The SDK ships its own boxed-future helper.
4. Unload is quarantine plus one reclaim on a late poll, replacing the fixed
   900 ms wait, with one field serving as both the leak counter and the
   quarantine predicate.
5. The ownership and lifetime rules above, plus the envelope in the header,
   became contract text after a hand-written C plugin exposed them as guesswork.
6. Two vtable `size` fields that were written and never read are deleted, so
   neither vtable carries a `size` field.
7. The `metis.*` control plane (schemas, configure, counters) moves onto the
   futures path so `invoke` can be deleted structurally rather than surviving as
   an inline side channel.

## 15. Decisions the implementation ticket took

These were open when the contract was approved. `plugin-contract-v1` closes them
as recorded here, so an implementer does not reopen them.

- **Export name and version.** The symbol stays `metis_plugin_entry_v1` and the
  ABI major stays 1, because this revision lands before the first release and no
  plugin exists in the wild to be compatible with. The entry order does change,
  so a stale pre-release plugin would be silently misread if it loaded:
  `abi_minor` becomes 1 and the loader refuses a minor it does not know, which
  refuses that stale plugin instead. `plugin-compatibility` later relaxes that to
  additive tolerance inside the major. Additive change inside major 1 stays
  append-only by `size`.
- **Typed facades.** Not in this revision. The erased `call` plus published
  schemas lands; generated typed facades belong to the SDK effort.
- **The two vtable `size` fields.** Deleted, with the residual recorded: a
  wrong-size plugin vtable is that plugin's own bug and is not detectable at
  load.
- **Control plane.** `list_capabilities` stays synchronous on the table, and only
  the per-call control capabilities (configure, counters) move onto the futures
  path, so `create_instance` never needs a schema fetch mid-call.
- **Reclaim.** Caller-driven on the poll return, which is what the prototype
  proved. A daemon-side reaper thread stays daemon work, and it is the one part
  of rule 6 not proven under load.

## 16. Recorded decisions this contract supersedes

These are still written in the effort maps and need amending, because they
describe the shape this contract replaces:

- `metis-plugins`, Decisions: "do not pass futures across the ABI" and
  "plugin-owned executors hidden behind the Rust SDK are the initial preferred
  approach".
- `metis-plugin-sdk`, Decisions: the same plugin-owned executor decision.
- `metis-foundations`, Decisions: the ABI table described as instance lifecycle,
  capability listing, one generic `invoke`, `cancel`, `release_buffer` and
  `shutdown`, plus the `complete` callback.

Two `metis-foundations` fog items are also answered here: the four-arm authoring
vocabulary question (answered by section 12, the runtime dispatch method
alternative) and the note that the inline `invoke` result has no in-tree
exerciser (answered by deleting `invoke`).
