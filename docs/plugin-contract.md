# The metis plugin contract (v1)

Status: approved by the operator on 2026-10-13 as good enough for a v1, on the
understanding that real use will surface what is wrong with it. This is the
contract text, not a proposal against an earlier shape.

Evidence base: the throwaway prototype at `spikes/plugin-registry`, with
`FINDINGS4.md` and `round-3/` for the measurements. The decision this contract
settles is recorded in `docs/adr/0002-plugin-contract-futures-only.md`, and the
landed shape is described in the `metis-foundations` architecture spec. Section 14
records what changed relative to the shape the prototype carried, compressed.
Section 15 records the decisions the implementation ticket took.

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
| descriptor | Name, kind, version, and the JSON Schema for one capability's request and response. V1 knows the kinds `tool` and `provider`; a kind this build does not know refuses that one capability and leaves the rest of the plugin usable. |
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

The symbol and the ABI major stay `v1` and 1, because this revision lands before the first release (section 15). The ABI minor is 2, because the revision below replaced the instance-creation entry with the asynchronous initialisation entry under that unchanged symbol and major.

## 5. The plugin table

| Entry | Shape | Contract |
| --- | --- | --- |
| `size` | `usize` | Bytes of the table the plugin provides. Read first, always. |
| `abi_major`, `abi_minor` | `u32` | Major is part of the symbol name; minor is compared additively, so a newer minor is tolerated and an older one is refused. |
| `init_plugin` | `(host: *const MetisHostApiV1, payload: MetisBuffer, out_instance: **mut c_void, out_future: *mut MetisFutureHandle) -> MetisStatus` | Constructs and initialises an instance. The one payload is host-owned and carries the plugin's own configuration and the host context (section 18); on `Ok` both out parameters are filled, the instance and a plugin-owned future that must resolve `Ok` before the instance is valid. There is no separate instance-creation entry. |
| `destroy_instance` | `(instance) -> MetisStatus` | Releases the instance. Only valid after `shutdown`. |
| `list_capabilities` | `(instance, out_payload: *mut MetisBuffer) -> MetisStatus` | Returns a plugin-allocated payload listing capabilities, each with its name, version, and request and response schemas. |
| `invoke_future` | `(instance, capability: *const u8, capability_len: usize, request_id: u64, payload: MetisBuffer, host: *const MetisHostApiV1, out: *mut MetisFutureHandle) -> MetisStatus` | The only call entry. On success `*out` holds a plugin-owned future. The request payload stays valid for the call only. |
| `cancel` | `(instance, request_id: u64) -> MetisStatus` | Marks that call's cancellation token. The plugin's future is expected to resolve as `Cancelled` rather than be dropped mid-flight. |
| `release_buffer` | `(buffer: MetisBuffer) -> MetisStatus` | The host frees a plugin-allocated buffer by calling this. The host never frees plugin memory itself. |
| `shutdown` | `(instance) -> MetisStatus` | Quiesces the instance. The host drops every future before calling it, so nothing may arrive after it returns. |
| `service_complete` | `(instance, request_id: u64, status: MetisStatus, payload: MetisBuffer) -> MetisStatus` | The host's answer to an earlier `service_request`. The plugin wakes its future's waker from here. |

Eight entries and no more. There is no counters entry and no counter of any kind
is reachable through the table.

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

The waker the host passes is one owning reference for that poll, and the host
releases it when the poll returns, so the plugin must not release it. The plugin
may clone it (each clone is one `clone` call and one owning reference) and must
`drop` every clone. One clone is consumed by exactly one `wake` or `drop`, so a
second use of one consumed clone is refused rather than acted on. `wake` may be
called from any thread, must be cheap and non-blocking, and may arrive after the
future was dropped, which the host detects and records rather than crashing on.

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
   `service_complete` payload is host-owned and borrowed for the call; the
   `init_plugin` payload is host-owned and borrowed for the entry, so a plugin
   that needs it afterwards copies it. `release_buffer` is reentrant: the host
   may call it from inside `service_request`.
2. **Single poller.** Only the daemon polls a future, one poll at a time. Two
   concurrent polls on one handle is a protocol violation.
3. **No blocking in poll.** A poll returns `Pending` and asks for a service. A
   poll that blocks is detected by a watchdog, recorded, and the plugin is
   quarantined.
4. **Waker liveness.** A wake from any thread is legal. A wake after the future
   is dropped is a counted violation, not a step in the ordering ledger: the
   host's record counts it and names the request id of the call it belongs to,
   and it is never a dangling dereference. One clone is consumed by exactly one
   `wake` or `drop`, so a second use of one consumed clone cannot release the
   call's state twice.
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

/// The two named parts of the initialisation payload.
pub struct Initialisation { /* config: Value, host_context: Value */ }
pub struct HostContext { pub instance_name: String, /* ... */ }

pub trait Plugin: Send + Sync + 'static {
    /// Construct and initialise, as one fallible asynchronous call. This is the
    /// only way to obtain a plugin value: `Default` is not part of the trait, so
    /// an uninitialised value is not representable.
    fn init(host: Host, initialisation: Initialisation)
        -> impl Future<Output = Result<Self, PluginError>> + Send
    where
        Self: Sized;

    /// Everything this plugin advertises, with schemas.
    fn capabilities(&self) -> Vec<CapabilityDescriptor>;

    /// One capability, routed by name. One request in, one response out.
    fn call(&self, call: Call, request: Request<'_>)
        -> impl Future<Output = Result<Response, PluginError>> + Send;

    /// A cancel was accepted; the call's token is already marked.
    fn cancel(&self, call: Call) -> Result<(), PluginError> { Ok(()) }

    /// Instance teardown.
    fn stopped(&self, host: &Host) {}
}
```

An author writes `async move { ... }` and decodes the request by capability name.
Per-capability typing comes from the SDK's schema-derived facades, so the trait
itself stays one method with no per-capability associated types. Initialisation
is where an author reads its configuration and the host context, and it may
await a host service rather than block.

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
7. The `metis.*` control plane is unwound. Configuration becomes the lifecycle
   entry `init_plugin` and its trait counterpart, which is where a plugin reads
   its configuration and the host context; the schema-fetch member is redundant
   because `list_capabilities` already returns schemas; and `counters` is
   dropped, because no consumer exists and both candidate needs are covered by
   the daemon's request-id policy and the host's own waker counts. The table
   therefore gains the initialisation entry and loses the counters entry the
   plan named.

## 15. Decisions the implementation ticket took

These were open when the contract was approved. `plugin-contract-v1` closes them
as recorded here, so an implementer does not reopen them.

- **Export name and version.** The symbol stays `metis_plugin_entry_v1` and the
  ABI major stays 1, because this revision lands before the first release and no
  plugin exists in the wild to be compatible with. The entry order does change,
  so a stale pre-release plugin would be silently misread if it loaded:
  `abi_minor` becomes 1 and the comparison refuses a minor older than the one
  this build knows, which refuses that stale plugin instead.
  `plugin-compatibility` landed the additive half: the comparison is `found >=
  known`, so a newer minor inside major 1 is tolerated and an older one is still
  refused. Additive change inside major 1 stays append-only by `size`.
- **Typed facades.** Not in this revision. The erased `call` plus published
  schemas lands; generated typed facades belong to the SDK effort.
- **The two vtable `size` fields.** Deleted, with the residual recorded: a
  wrong-size plugin vtable is that plugin's own bug and is not detectable at
  load.
- **Control plane.** `list_capabilities` stays synchronous on the table.
  Configuration became a lifecycle entry rather than a capability, and the
  revision replaced instance creation with it: `init_plugin` constructs and
  initialises in one futures-shaped call, so an instance that exists has
  completed initialisation with its configuration. The counters member was
  dropped rather than moved, for the reason section 14 item 7 records.
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

## 17. The bundle container

A bundle is one file that holds a plugin's archive, the certificate chain that
authorises it, and the signature over both. `plugin-runtime`'s `verify_bundle`
turns such a file into a borrowed view of it (the parsed header, the
leaf-authorising certificate's common name, and the archive bytes; nothing is
copied and nothing is extracted) or into a typed refusal. This section fixes the
bytes, so that a second implementation can be written from this text alone.

### 17.1 The container

The file begins with a 40-byte header, then the chain region, then the signature,
then the archive. Every field is big-endian, and the header is fixed width
because the signature's location is computed from it.

| offset | size | field |
| --- | --- | --- |
| 0 | 8 | magic, the ASCII bytes `METISBND` |
| 8 | 2 | container format version, `u16`, which must equal this build's exactly, currently 1 |
| 10 | 2 | signing suite, `u16`, currently 1 for X.509 over Ed25519 |
| 12 | 2 | header length, `u16`, which must equal 40 |
| 14 | 2 | reserved, `u16`, which must be zero |
| 16 | 4 | chain length, `u32` |
| 20 | 8 | signature offset, `u64`, which must equal the header length plus the chain length |
| 28 | 4 | signature length, `u32` |
| 32 | 8 | ZIP length, `u64` |

The file's length must equal the signature offset, plus the signature length,
plus the ZIP length. Nothing else can be hidden in the file: the header, the
chain and the archive are all signed, so the only bytes outside the signature's
range are the signature's own, and an appended or prepended byte is a refusal
rather than something a lenient reader forgives.

The magic and the container version are the only fields pinned across versions,
because the version decides the rest of the header's shape and therefore where
the signature is. The version is one integer that must match this build's
exactly, and the manifest does not repeat it.

### 17.2 The signed message and the signature

The signed message is, in order: the domain tag, the header's 40 bytes, the chain
region's bytes, then the archive's bytes. The signature's own bytes are deleted
from the message rather than zero-filled, so a signature never covers itself. The
domain tag is the 27 ASCII bytes `METIS-PLUGIN-BUNDLE-PAYLOAD`, and it carries no
container version, so a version bump does not change what a signature covers.

The signature is raw Ed25519 (RFC 8032), 64 octets, made by the key of the
leaf-authorising certificate (17.4). Verification is strict: a key or signature
encoding that is not the canonical one, and a small-order key, are refused, so
the non-canonical and weak-key cases RFC 8032 names cannot be used to make one
signature verify under two keys.

### 17.3 The chain region

The chain region is a concatenation of whole DER X.509 version 3 certificates,
the leaf-authorising certificate first and the anchor last. Certificates are
self-delimiting, so the region must be consumed exactly: bytes after the last
whole certificate are a refusal, not padding. A region holds at most eight links,
and the count is derived from the region rather than carried.

The region is read by a strict DER decoder, which refuses anything that is not in
DER's canonical form before the value is interpreted. That is what makes the link
check sound: a child's signed body is re-encoded rather than kept, and
re-encoding is only faithful for input that was canonical to begin with.

A link other than the anchor is verified against the link above it. The child's
issuer name must equal the parent's subject name, and the child's own signature
must verify under the parent's key.

### 17.4 The anchor, the profile and validity

The last link is the anchor, and it is trusted by its key rather than by who
signed it. A certificate that carries a root key's public key says nothing an
attacker could not also say, and what such a certificate cannot do without the
root's private key is sign the link below it. The anchor's key id must be one the
trust store holds.

The profile this project owns, applied to every link:

- a certificate that authorises another key carries `basicConstraints` with
  `CA:TRUE`. In a chain longer than one link, that is every link but the
  leaf-authorising one;
- the leaf-authorising certificate is an end entity: it does not carry `CA:TRUE`;
- `basicConstraints` (OID 2.5.29.19) is the only extension this build
  understands, so any other critical extension refuses the chain and any other
  non-critical extension is ignored. A producer may therefore carry its own key
  identifier, and this build will not trust it (17.5);
- the leaf-authorising certificate's common name is the plugin's name, and it is
  what the caller compares against the manifest. An absent or empty common name
  is a refusal.

Validity is each certificate's own `notBefore` and `notAfter`, at second
granularity, against the host's clock, with 60 seconds of skew tolerated at each
end. A window that has not started and one that has passed are both trust
failures.

### 17.5 Key ids and revocation

The key id of a public key is the first 16 bytes of its SHA-256, taken over the
raw 32-byte Ed25519 public key and never over its SPKI wrapper.

An identifier a certificate carries is never trusted. The subject key identifier
extension, the authority key identifier extension and every other carried value
decide nothing: each key id this build compares is recomputed from the public key
it holds. A trust store's roots are registered by public key, and their ids are
computed the same way, so a caller cannot register a root under an id of its own
choosing.

The revocation list names keys by that id. No key of any link may appear in it,
the anchor included, so that revoking a root key refuses every chain anchored at
it. A chain that carries an identifier the list names, while its own keys compute
to ids the list does not, is unaffected.

The compiled-in root set is empty. A daemon starts trusting nothing, and its
roots and revocations are its own.

### 17.6 The refusals

The checks run in one order, and each refusal answers for itself rather than for
a later consequence:

1. the container: the magic, the container version, the signing suite, the header
   length, the reserved field, the chain and signature spans, and the whole-file
   span;
2. the chain region: an empty region, a certificate that does not decode, an
   encoding that is not canonical, bytes after the last whole certificate, and
   more than eight links;
3. the anchor's key id, then the revocation list;
4. every link's validity window;
5. the profile: the extensions, the CA constraint, the end entity leaf and the
   leaf's common name;
6. the links: the issuer and subject rule, then each parent's signature;
7. the payload signature: its length, then its verification.

A refusal is one of two kinds, and `BundleError::is_trust_failure` answers which:

| Kind | Refusals | What it means |
| --- | --- | --- |
| A trust failure | an empty chain, an unknown root, a revoked key, an expired or not-yet-valid certificate, a malformed signature, a signature that does not verify, a broken link, a link signature that does not verify, every chain profile violation, and a leaf with no usable common name | The file claims a signature this daemon does not trust, which is evidence that something is wrong. Start-up stops. |
| A file this build cannot read | a file shorter than the header, bad magic, an unknown container version, an unknown signing suite, a bad header length, a non-zero reserved field, bytes before the header, a chain or whole-file span mismatch, a malformed certificate, a non-canonical encoding, trailing bytes in the chain region, and more than eight links | There is no evidence of tampering, only a bundle that does not work here. That one bundle degrades. |

Bytes before the header means a declared signature offset that lies before the
header's end, which would place the chain region's start inside the header. The
offset cannot both do that and equal the header length plus the chain length, so
this check runs first.

### 17.7 Dependencies

The trusted parse path takes `ed25519-dalek` 3 over `curve25519-dalek` 5,
`x509-cert` 0.3 over `der` 0.8 (`spki` 0.8, `const-oid` 0.10), and `sha2` 0.11.
All of them are pure Rust, none has a build script that compiles C, and none is
feature-enabled beyond what these checks need, so the path that reads untrusted
bytes adds no build step. Measured at `plugin-runtime` for the `bundle`
verification ticket:

- `cargo tree -p plugin-runtime --edges normal --prefix none | grep -v '(*)' |
  sort -u | wc -l`: 66 crates before, 85 after, so the parse path adds 19
  including its four direct ones. The `grep -v '(*)'` drops the marker lines
  `cargo tree` prints for a repeated subtree, which `sort -u` would otherwise
  count a second time; run without it the same pipe prints 77 and 101.
- The same pipe without `--edges normal`, which includes the test-only signer:
  76 crates before, 101 after.
- The only added crate with a build script is `curve25519-dalek`, whose script
  selects cfg flags from the rustc version and compiles nothing.

The test signer that builds the committed vectors is a dev-dependency, on
`x509-cert`'s `builder` feature and on `ed25519-dalek`'s `alloc` and `pkcs8`
features, so no shipped build compiles a signing path.

### 17.8 The archive and the extraction rules

A bundle's archive is a ZIP file. The archive root carries `manifest.toml`, and
each target the bundle supports is a directory under `lib/` named by that
target's string. A target is one string, the operating system and the
architecture joined by a hyphen exactly as `std::env::consts::OS` and
`std::env::consts::ARCH` spell them, so `linux-x86_64` and `macos-aarch64`. A
target's library path is `lib/<target>/<file name>`, and a target directory
holds exactly one file.

`plugin_runtime::bundle::extract(verified, target)` turns a verified bundle into
a staging directory, or into a typed refusal. It stages the root manifest and
this host's target directory and nothing else, so a bundle that carries several
targets stages only the files this host's load needs. Every entry is validated
whether or not this host's target needs it, and an entry outside those two
places is validated and skipped rather than written.

The refusals, in the order they are decided:

1. the bytes are not a ZIP archive this build can read;
2. the archive holds more than 256 entries;
3. an entry's name is absolute (a leading `/` or `\`, or a Windows volume such
   as `C:`), carries a `..` component, is empty once its separators are
   removed, or carries a NUL byte;
4. an entry is a symbolic link, a directory (its name ends in `/`, or its
   declared type says so), or any other type that is not a regular file, so a
   hard link, a device, a socket and a FIFO are all refused;
5. an entry declares permission bits other than `0600`, the mode this host
   writes. A declaration of `0600` is accepted with or without the
   regular-file type bit beside it, and so is an entry whose external
   attributes are zero, which declares no mode at all. A producer that declares
   a mode must declare the mode the host itself would write; the `zip` writer
   declares `0o644` when a producer does not choose one, so a producer sets it;
6. an entry declares an uncompressed size above 64 MiB, or the entries'
   declared sizes add up to more than 256 MiB between them. The per-entry limit
   is checked first, so the archive total answers for an archive whose entries
   are each inside their own limit;
7. an entry this host stages does not read: its checksum disagrees, its stream
   is malformed, or its compression method is one this build does not read;
8. the archive holds no file directly under `lib/<target>/` (the bundle carries
   no library for this host), or it holds more than one (which one is the
   library is not decided).

Only the declared sizes are summed and checked for the entries this host does
not stage, and only the staged entries' bytes are read and checksummed, so a
target this host does not load is never decompressed.

The limits are the ones `plugin_runtime::bundle` exports: `MAX_ENTRIES` (256),
`MAX_ENTRY_SIZE` (64 MiB) and `MAX_UNCOMPRESSED_SIZE` (256 MiB). A bundle
carries one file per target plus the manifest, so the entry count leaves room
for a producer that also carries assets while keeping the walk over the archive
bounded, and the size limits bound what an archive can cost this host before a
byte of it is written.

**Staging.** `extract` creates one directory under `std::env::temp_dir()`, named
`metis-plugin-stage-` followed by this process's id, the current time in
nanoseconds and a counter, so a sweep after a crash can find what a killed
process left and two processes staging at once cannot pick the same name. A
staged directory is `0700`
and a staged file is `0600`; no mode an entry declares ever reaches the
filesystem. A refusal removes the staging directory before it is answered, so a
refused extraction leaves nothing behind and what an extraction staged before
the refusal is removed with it. A successful extraction returns a
`StagedBundle`, which owns the directory and removes it when it is dropped, so a
caller holds it until the load is finished with the library.

`StagedBundle::root()` is the staging directory, `::manifest_path()` is
`<root>/manifest.toml` and `::library_path()` is the staged library of this
host's target. Whether a manifest is there is the manifest step's question
rather than extraction's: extraction answers where the file would be, and a
bundle with no root manifest still stages, so the step that reads the manifest
can refuse it for what it is.

**Dependencies.** The archive is read by the `zip` crate with its default
features off and only its pure-Rust deflate backend on, so no bzip2, zstd,
lzma, AES or C zlib path enters the reader; stored entries need no backend at
all. Measured at `plugin-runtime` for the extraction ticket:

- `cargo tree -p plugin-runtime --edges normal --prefix none | grep -v '(*)' |
  sort -u | wc -l`: 85 crates before, 91 after, so the extractor adds six,
  `zip` itself the only direct one, beside `flate2`, `zlib-rs`, `crc32fast`,
  `indexmap` and `typed-path`.
- The same pipe without `--edges normal`, which includes the test-only signer:
  101 crates before, 107 after.
- `crc32fast` is the only added crate with a build script, and its script reads
  `rustc --version` to select cfg flags and compiles nothing.

### 17.9 The manifest

A bundle's root carries `manifest.toml`, and every bundle has one. The text is
TOML, written by an author or by the SDK's generator, and the runtime reads it
before it maps a library: `plugin_runtime::manifest` turns the text into a
validated `Manifest` or into a typed refusal. The format belongs to the author,
which is why it is documented here as fields to fill in rather than as a
serialisation of any type, and why the C fixture's bundle is packaged by writing
one by hand.

| field | type | meaning |
| --- | --- | --- |
| `name` | string | The plugin's identity. It must be the name the bundle's leaf-authorising certificate carries (17.4). |
| `version` | string | The plugin's own version. It is recorded and never gates a load. |
| `abi_major` | integer | The ABI major the plugin is built for. It must equal this build's. |
| `min_abi_minor` | integer | The oldest ABI minor the plugin needs. The minor this build knows must not be lower than it. There is no range syntax. |
| `libraries` | table | One library path per target, relative to the archive root, spelled the way 17.8 fixes it: `<target> = "lib/<target>/<file name>"`. |
| `capabilities` | array of tables | The capability list the host trusts for discovery. |
| `capabilities[].name` | string | The name `invoke_future` addresses, without the instance qualifier the host adds. |
| `capabilities[].kind` | string | `tool` or `provider`. A kind this build does not know is refused for that one capability. |
| `capabilities[].version` | integer | The capability's own version, which the host gates per capability. |
| `capabilities[].request_schema` | table | The JSON Schema of this capability's request body. |
| `capabilities[].response_schema` | table | The JSON Schema of this capability's response body. |
| `config_schema` | table | The JSON Schema the daemon resolves this plugin's configuration against. It is the schema of the `config` part of the initialisation payload (18). |

A schema is declared as the TOML that stands for the JSON document: a table is
an object, an array is an array, and a string, an integer, a float or a boolean
is the JSON value of the same kind. TOML has no null, so a schema that needs one
says so another way: `type = "null"` is a string and is fine, while a `null`
default cannot be written here at all. A TOML date-time is not used, because it
has no JSON counterpart. The fields may appear in any order, and a field this
build does not know is ignored rather than refused, so a manifest a newer
producer wrote stays readable here.

```toml
# The plugin's identity and its own version.
name = "dev.metis.example-plugin"
version = "1.0.0"

# The ABI this plugin is built for, and the oldest minor it needs.
abi_major = 1
min_abi_minor = 2

# One library path per target, relative to the archive root.
[libraries]
linux-x86_64 = "lib/linux-x86_64/libexample.so"

# The capability list, with each capability's schemas written inline.
[[capabilities]]
name = "tool.echo"
kind = "tool"
version = 1

[capabilities.request_schema]
type = "object"
required = ["echo"]
properties = { echo = { type = "string" } }

[capabilities.response_schema]
type = "object"

# The schema the daemon resolves this plugin's configuration against.
[config_schema]
type = "object"
properties = { instance = { type = "string" } }
```

**What is checked, in this order, and what is refused.**

1. the manifest is there. A bundle with no root manifest is the absent refusal
   and not the invalid one, because extraction stages a bundle that carries no
   manifest on purpose (17.8);
2. the text is the format. A field that is missing, a value of the wrong type,
   and text that is not TOML at all are one refusal that names what is wrong;
3. the name the manifest declares is the name the bundle's leaf-authorising
   certificate carries, so a bundle cannot declare an identity its own chain
   does not carry;
4. the ABI demand. A differing `abi_major`, and a `min_abi_minor` newer than the
   minor this build knows, are the landed version errors, `MajorMismatch` and
   `MinorMismatch`, because this is the landed rule read from the plugin's side
   rather than a second rule. Both are decided before the library is opened, so
   a plugin this build cannot serve is refused before any byte of it is read;
5. the library for this host's target. The declaration is resolved against the
   staging directory, and only this host's target is resolved: a manifest that
   declares other targets, or none, is judged by what it declares for this host,
   so a multi-target bundle is never asked for the files this host does not
   load. A manifest that declares no library for that target, or whose
   declaration names a file the bundle does not carry there, is refused, and the
   refusal names the target.

The plugin's own version is recorded and gates nothing. The capability list is
what the host trusts for discovery, and the load entry cross-checks it against
what the library reports, so a manifest that disagrees with its library disables
that plugin rather than serving it. The cross-check is the capability names: the
two declarations have to carry the same ones, and a name carried by one side and
not the other refuses the load with a refusal that names that capability.
Nothing else about a capability is cross-checked there, because the manifest's
schemas are declared text an author writes while the library derives its own,
and a capability's kind and version are gated where a call routes. A library
whose table reports an older minor
than this build's, or is too short for an entry this build needs, is refused by
the version gate (17.6) rather than by the manifest.

**Dependencies.** The manifest is read by `toml` 0.9 with its default features
off and only `parse`, `serde` and `std` on, so no display or writing path is
compiled in. Measured at `plugin-runtime` on this ticket's landing branch before
it and after it:

- `cargo tree -p plugin-runtime --edges normal --prefix none | grep -v '(*)' |
  sort -u | wc -l`: 84 crates before, 90 after, so the reader adds six: `toml`
  itself, `toml_parser`, `toml_datetime` and `serde_spanned`, and `winnow`
  twice, at 0.7.15 for `toml` and at 1.0.4 for `toml_parser`.
- The same pipe without `--edges normal`, which includes the test-only signer:
  98 crates before, 104 after.
- None of the six has a build script that compiles anything, so the reader adds
  no build step.

## 18. The initialisation payload

`init_plugin` receives one host-owned payload buffer, carried in the envelope of
section 9. The body is one JSON object with two named parts:

| field | shape | meaning |
| --- | --- | --- |
| `config` | object | The plugin's own configuration, as the daemon resolved it. Its schema is the one the manifest declares. |
| `host_context` | object | What the host tells the plugin about itself. |

The host context is permissive and forward-compatible: a plugin ignores context
fields it does not know, so a host may add one without breaking an older plugin.
The fields this build carries are `instance_name` (the name the plugin is being
initialised as), `data_dir` (a directory the plugin may keep its own state in,
absent when the host set none), and `capability_version` (the highest capability
descriptor version the host serves). Nothing secret belongs in the context; the
host decides what it discloses, and the runtime never reads the daemon's
configuration.

A failed initialisation falls into two classes, and the status the future
resolves with is what tells them apart. A configuration the plugin cannot read
is `invalid_argument`: the plugin is telling the host that what it was handed
does not match what it declared. Every other terminal status is the plugin's own
defect. On either, the host drops the future and then calls `destroy_instance` on
the instance pointer, which must tolerate a failed initialisation.

## 19. Start-up outcomes

A daemon starts from a directory of bundles. Every regular file directly in the
directory is one bundle, named by its file's stem, and nothing is scanned
recursively and no discovery file is read. Each bundle is loaded through the one
host-side load entry, and the refusal that entry answers decides one of three
outcomes.

| Outcome | The refusals that produce it | What the daemon does |
| --- | --- | --- |
| stop | a trust failure, a bundle file that cannot be read, a manifest that is absent, unreadable, invalid, or not the name the bundle's chain carries, and a plugin that could not be unloaded | stop start-up |
| disable | a container this build cannot read, any extraction refusal, a missing or ambiguous target library, a manifest that declares no library for this host or one the bundle does not carry, an ABI mismatch, a manifest the library disagrees with, and a registration the host refuses | that one plugin does not start, and the daemon runs |
| disable, as a placeholder | an initialisation that failed, whether the configuration was refused or the plugin itself is defective | that one plugin does not start, its instance is unloaded, and it is listed |

Three rules follow from that table, and they are the policy rather than an
implementation of it.

**A bundle's identity before its manifest is read is its file's stem.** The
daemon resolves the configuration for a bundle by the name it found the file
under, and a placeholder whose manifest could not be read is named by that stem,
so a plugin is always told the identity its configuration was resolved under.
Once the manifest is readable, the name the manifest declares is the registration
and routing identity, which is why the load entry rejects a manifest whose name
disagrees with the name the bundle's chain carries.

**A trust failure stops and a file this build cannot read does not.** The split
is the predicate of section 17.6, `is_trust_failure`: a signature this daemon does
not trust is evidence that something is wrong, while a container this build
cannot parse is one bundle that does not work here. The unknown container version
is the case that decides the rule, because it degrades before the bundle's
manifest is ever read.

**A failed initialisation is unloaded, and a refused unload stops start-up.** The
host drops the plugin's future and then destroys the instance the entry handed
out, which is the order section 18 fixes. If that destroy refuses, the daemon is
left holding an instance it cannot describe, so start-up stops rather than
continuing with part of its plugin set.

**A disabled plugin is listed and not offered.** It appears with its name, the
refusal that disabled it, and the capabilities its manifest declared. The name is
the manifest's when a manifest was readable, and the bundle file's stem when none
was, in which case there are no capabilities to list. Its capabilities are
excluded from what an agent is offered, and a call for one of them is answered as
the disabled route error carrying that reason rather than as unknown, so a caller
tells a plugin that was turned off from one that was never loaded.
