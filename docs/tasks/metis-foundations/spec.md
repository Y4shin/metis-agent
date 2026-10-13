---
type: spec
title: Metis foundations
status: stable
---

# Metis foundations

## Problem Statement

The repository is a scaffold. Nine crates exist, the contracts tree holds empty
placeholders, and no code has been written. Nothing can talk to the daemon:
there is no wire contract, no client, no plugin interface, and no way to add a
capability.

Every other effort is stuck behind that. The daemon effort cannot serve an API
that has not been defined, the plugin effort cannot host a plugin whose ABI does
not exist, and the surface effort cannot show anything without a client. Starting
on the agent loop or a UI leads straight back to the same missing piece: what
exactly does the daemon speak, and how does a plugin plug in?

The user is therefore blocked not on effort but on interface. They cannot
demonstrate Metis doing anything real, and they cannot delegate any of the
remaining work, because there is nothing stable to build against.

## Solution

Make the wire and the plugin interface real, and prove them.

The wire contract is hand-written Protobuf, generated into committed Rust and
spoken over ConnectRPC, so one message set serves both sides and any client
language can consume the contract. A UI-independent client speaks that contract
over a Unix domain socket, a local named pipe, or an explicitly enabled TCP/TLS
listener, holds the credential, resumes event streams from a cursor, and reports
typed errors. Authentication starts from the operating system's own file access
control on the local socket, escalates to per-surface tokens minted through
pairing with a preshared key, and never puts a credential in a URL. The plugin
interface is a small versioned C ABI with a Rust authoring trait on top of it,
so a plugin written in Rust looks like ordinary async Rust while the ABI stays
stable and implementable from other languages.

A walking skeleton then proves the whole path: one request and one event stream
over the local transport, generated from the contract, driven by the real
client. When it runs, every other effort has something concrete to build on.

## User Stories

1. As the maintainer, I want a versioned Protobuf contract to be the single
   source of truth, so that the daemon API has one definition that every
   consumer derives from.
2. As the maintainer, I want the contract to stay free of Rust-only concepts, so
   that a client in another language can be generated from it.
3. As the maintainer, I want `buf lint` and `buf breaking` to run in CI, so that
   a contract change which breaks existing clients is caught in review rather
   than at runtime.
4. As the maintainer, I want generated code committed to the repository with a
   regenerate-and-diff check, so that reviewers can read what was generated and
   builds need no generator.
5. As the maintainer, I want the generator plugins pinned to exact versions, so
   that regeneration is reproducible across machines and over time.
6. As the maintainer, I want one documented regeneration command, so that any
   developer can regenerate the contract code and see the resulting diff.
7. As a surface developer, I want a single UI-independent client crate, so that
   a terminal, a desktop application, and Android share one implementation.
8. As a surface developer, I want the client to expose one API regardless of
   transport, so that I do not branch on the platform.
9. As a user on Linux or macOS, I want the client to reach the daemon over a Unix
   domain socket by default, so that local use needs no network exposure.
10. As a user on Windows, I want the client to reach the daemon over a local
    named pipe, so that the desktop transport works without TCP.
11. As a user, I want the endpoint to come from a platform default with an
    environment variable override, so that I can point a client at a
    non-default daemon.
12. As a remote user, I want an explicitly enabled TCP/TLS listener, so that I
    can reach my daemon from another device.
13. As a user, I want remote access to be off unless I enable it, so that
    exposure is always a deliberate act.
14. As a user behind a reverse proxy, I want the daemon to accept plaintext from
    a named proxy and to warn me otherwise, so that a proxy can terminate TLS
    without me fighting the daemon.
15. As a user, I want the bind address to have no default, so that enabling
    remote access cannot silently expose every interface.
16. As a user, I want TLS material read at startup and a restart required to
    change it, so that the behaviour is simple and predictable.
17. As a user with a publicly trusted certificate, I want the client to verify
    against the operating system trust store, so that a rotated certificate
    keeps working.
18. As a user with a self-signed certificate, I want the client to accept a
    pinned public key learned during pairing, so that I do not have to build a
    private CA.
19. As a user, I want no client certificates and no certificate-based client
    authentication, so that the credential is the only thing I manage.
20. As a user, I want the local socket to be owned by a configured user and
    group with mode 700 or 770, so that only the accounts I choose can connect.
21. As a user on Windows, I want the pipe restricted to my account and SYSTEM
    with remote clients rejected, so that no other account or machine can
    connect.
22. As a user with several devices, I want to pair each surface with a preshared
    key, so that every device gets its own token.
23. As a user, I want to pair a phone over the network, so that a device with no
    local socket access can still be set up.
24. As a user, I want to close pairing by removing the preshared key and
    restarting, so that I can stop accepting new surfaces.
25. As a user, I want to revoke one device by removing its token and restarting,
    so that a lost device is cut off without disturbing the others.
26. As a user, I want the preshared key to come from configuration as a literal,
    an environment variable, a keystore item, or a permission-checked file, so
    that it can be rotated without code changes.
27. As a user, I want a secret file to be refused unless only its owner can read
    it, so that a misconfigured secret fails loudly.
28. As a user, I want every paired client to have the same access, so that
    permission decisions stay with agents and not with devices.
29. As a user, I want the credential sent as request metadata and never in a
    URL, so that logs and proxies do not record it.
30. As a client developer, I want missing credentials and invalid or revoked
    credentials to be distinguishable errors, so that the UI can say "pair me"
    rather than showing a generic failure.
31. As a client developer, I want the client to keep its credential in the
    platform keychain when possible and an owner-only file otherwise, so that
    the token is not casually readable.
32. As a surface developer, I want events on a server-streaming RPC, so that I
    do not hand-roll framing, heartbeats, or resume parsing.
33. As a surface developer, I want the stream request to carry a cursor, so that
    a reconnect resumes instead of replaying everything.
34. As a surface developer, I want the stream to begin with a snapshot, so that
    a resync is a fresh stream and not a separate endpoint.
35. As a surface developer, I want the stream to select channels, so that a
    client receives only what it displays.
36. As a surface developer, I want ordering within a stream to be structural, so
    that I never sort or discard events to fix the order.
37. As a user, I want a brief disconnect to lose nothing while the daemon still
    holds the events, so that a reconnect is invisible.
38. As a user, I want an expired cursor to be reported as such, so that the
    client refetches rather than showing a stale view.
39. As a user, I want a daemon restart to require only a resync, so that I do not
    manage a durable event log.
40. As a user, I want a client that cannot keep up to be disconnected rather than
    silently skipped, so that I always see a coherent view.
41. As a surface developer, I want to cancel work with an explicit idempotent
    call, so that stopping a task does not depend on dropping a connection.
42. As a user, I want closing a UI to leave daemon work running, so that long
    tasks survive a window being closed.
43. As a surface developer, I want correlation identifiers in request metadata,
    so that a support log can tie a call to its response.
44. As a plugin author, I want a small primitive ABI rather than a C rendering
    of the Rust trait, so that trait changes do not force plugin rebuilds.
45. As a plugin author, I want a versioned entry point and an append-only
    function table, so that an older plugin still loads on a newer daemon.
46. As a plugin author, I want one generic invoke addressed by capability name,
    so that adding a capability does not move the ABI.
47. As a plugin author, I want to supply tools and providers, so that a plugin
    can extend both what the agent can do and which models it can use.
48. As a plugin author, I want to describe my capabilities with a name, a kind, a
    version, and a schema reference, so that the host can gate each capability
    independently.
49. As a plugin author, I want a Rust authoring trait with adapters, so that I
    write ordinary async Rust and never touch the C boundary.
50. As a plugin author, I want a cancellation token on the per-request context,
    so that a call blocked on I/O stops promptly when it is cancelled.
51. As a plugin author, I want the adapter to complete asynchronously through the
    host, so that I do not have to reason about a synchronous fast path.
52. As a plugin author, I want a generated C header, so that I can implement the
    ABI from another language.
53. As a plugin author, I want payloads to carry a format header, so that a
    different codec can be introduced later without breaking the ABI.
54. As a plugin author, I want unknown payload fields ignored, so that a newer
    host and an older plugin interoperate.
55. As a plugin author, I want each side to release only what it allocated, so
    that an allocator mismatch cannot corrupt memory.
56. As a plugin author, I want a panic in my plugin to surface as an error, so
    that it does not unwind into the host.
57. As the maintainer, I want layout assertions plus a compiled C check in CI, so
    that the generated header cannot drift from the Rust layout.
58. As the maintainer, I want the plugin contract's provisional decisions to be
    revisable during implementation, so that the first real plugin can correct
    what the prototypes could not settle.
59. As the maintainer, I want a walking skeleton that performs one request and
    receives one event over the local transport, so that the contract, the
    generated code, the transport, and the client are proven together before
    anything is built on them.
60. As the maintainer, I want that skeleton to run on Linux in CI, so that a
    transport regression fails the build immediately.
61. As the maintainer, I want the Windows pipe path compiled for the Windows
    target in CI, so that it cannot silently rot on a platform we do not test.
62. As the maintainer, I want the static linking posture checked, so that the
    daemon does not acquire a dynamically linked system dependency by accident.
63. As the maintainer, I want formatting, linting, and tests to include generated
    code with documented allowances, so that the gates stay honest rather than
    being quietly skipped.
64. As the maintainer, I want no JVM or other heavyweight generator runtime in
    the everyday toolchain, so that a fresh checkout can build and test.

## Implementation Decisions

**Contract and generation.** The contract is spec-first. Hand-written Protobuf
under the versioned package `metis.v1` is the source of truth in the contracts
tree, generated with `buf` and pinned plugins. Two plugin tracks are used: the
Protobuf message and view generator at `buffa` 0.9.2 and the ConnectRPC service
generator at `connectrpc/rust` 0.9.0, both resolved as BSR remote plugins, so no
plugin binaries are installed locally. `buf lint` runs in STANDARD mode and
`buf breaking` runs in FILE mode against `main`. Generated trees are committed
and a regeneration script regenerates into a temporary directory and diffs,
failing non-zero when the committed output is stale. Raw `buf generate` output
does not satisfy the repository formatting gate, so the regeneration script
applies a formatting step to the generated trees, and the generated trees carry
the small set of allow attributes the ConnectRPC spike measured (derivable
implementations, single-binding matches, and the refining impl-trait allow per
generated implementation).

**`api-contracts`.** This crate holds the generated message set, the generated
Connect client, and the generated server stubs. It is the one message set both
sides use, which is what removes the duplicate-DTO and conversion-layer cost the
OpenAPI path paid. Generated code is mounted rather than edited. The crate also
carries a small handwritten transport module, holding the listener and connector
glue for the Unix socket, the Windows named pipe, and the TCP/TLS transports, so
that the skeleton, the client, and later the daemon share one implementation of
the accept loop and the connector wiring instead of writing it twice.

**`client`.** The client is the only UI-independent way to reach the daemon. It
wraps the generated Connect client and injects the transport, so callers see one
API over all three transports. It holds the credential as request metadata, maps
the daemon's error codes to typed errors that distinguish unauthenticated from
invalid or revoked, resumes a stream from a stored cursor, reconnects with
jittered backoff, and reports an expired cursor distinctly so the caller can
resync. It stays free of Tauri and terminal UI dependencies.

**The daemon's listener is not in this effort.** The skeleton carries a minimal
service implementation over the shared transport listeners, enough to prove the
wire. The configurable, proxy-aware, ACL-configured listener belongs to the
daemon effort, which will use the same transport helpers.

**Authentication.** Local IPC is authenticated by the host's own access control:
the daemon owns the Unix socket and its directory as a configured owner and group
with mode 700 or 770, and on Windows the pipe descriptor is
`D:P(D;;GA;;;NU)(A;;GA;;;<daemon-user-SID>)(A;;GA;;;SY)`, which mirrors mode 0700
and leaves the residual that any process running as the same user can connect.
The connection handshake declares the protocol version, the surface name, and the
client version for logging, and is not authentication. Remote clients hold an
opaque per-surface token minted at pairing; the daemon stores only its hash and
revocation requires a restart. A token grants the full exposed surface, with no
per-client permissions, because agent permissions are resolved server-side from
the agent context. The credential travels as `Authorization` request metadata on
every call, including the event stream, and never in a URL. Pairing is gated by a
preshared key that comes from configuration as a literal, `$ENV:`, `$KEYSTORE`,
or `$SECRET_FILE:` reference; the secret-file form is refused unless the owner can
read it and group and others have no access. Pairing is reachable on both local
IPC and the remote listener. Tokens do not expire in V1; an expiry or refresh
mechanism is additive later.

**TLS.** rustls is used on both sides with static linking preferred and no
dynamically linked system libraries on the daemon. Certificate and key come from
configuration as PEM references, with first-enable self-signed generation as a
convenience; ACME is not implemented, because a reverse proxy owns publicly
trusted certificates. Clients check the operating system trust store first and
fall back to a pinned public-key fingerprint learned during pairing, with
hostname verification always on and no CA-file configuration. Plaintext on the
TCP listener is allowed; the daemon warns unless reverse-proxy mode names trusted
proxy addresses, from which forwarded headers are honoured. The bind address has
no default. Configuration and TLS material are read at startup, and changing
either requires a restart.

**Realtime.** Events arrive on a server-streaming RPC. The request selects one or
more channels (`daemon`, `session:<id>`, `task:<id>`, `workflow_run:<id>`) and
carries a cursor as `last_event_id`. The stream's first message is a snapshot, so
a resync is a fresh stream. The daemon assigns every event a daemon-wide
monotonic identifier from a bounded in-memory ring; an identifier older than the
ring produces a distinct error that means "resync", and a daemon restart
expires every cursor, because the stream is ephemeral and is not the source of
truth. There is one delivery queue per stream, so ordering is structural and
observed order equals event-id order across channels. A consumer that cannot keep
up meets bounded backpressure and then a plain stream drop rather than silently
skipped events, and the client answers a drop with a resync. Heartbeats are
ordinary stream messages. Cancellation is an explicit idempotent RPC carrying the
target request identifier; dropping a stream cancels nothing by itself, and
closing a UI never stops daemon work. The protocol major version is the proto
package.

**Plugin contract.** The ABI is a small primitive set, not a C rendering of the
Rust trait, so that trait changes do not move the ABI. The exported entry point
is versioned by symbol name itself, and the function table it returns is
append-only within a major version, with its size as the first field, so a newer
host reads an older plugin's shorter table. The table carries instance creation
and destruction, capability listing, one generic `invoke(instance, capability,
request_id, payload, host_api)`, `cancel`, `release_buffer`, and `shutdown`. The
host table carries a `user_data` pointer and the callbacks `complete`,
`progress`, `emit_event`, and `log`. Capability descriptors name a capability
with a kind, a version, and a schema reference; V1 kinds are `tool` and
`provider`. The status space is a frozen integer enum including a `pending`
member, since an asynchronous result has no other way to report that it will
arrive through `complete`, and a separate detail buffer carries an error string.
Each side releases only what it allocated: plugin-allocated buffers go back
through the plugin's `release_buffer`, host-allocated buffers are freed by the
host, and the host never frees plugin memory, including when it drops a payload
it does not use. Panics are contained into an internal-error status on both sides
of the boundary, because a host callback is itself an `extern "C"` frame reached
from plugin code.

*From the mechanics prototype, the payload envelope is a single header byte:*

```text
bits 0-3  payload format version
bit 4     codec, 0 = UTF-8 JSON, 1 = MessagePack
bits 5-7  reserved, must be zero
```

Only JSON is used initially. A codec this build does not know returns
`unsupported`, while a bad version or non-zero reserved bits return
`invalid_argument`, so the two failures stay distinguishable. Payload evolution
within a major version is additive: unknown fields are ignored and new fields are
optional.

`plugin-contract` holds the primitives, the payload types, the authoring trait,
and both adapters, with the authoring side behind a feature gate so the runtime
depends on primitives and payload types alone. The authoring trait exposes a
cancellation token on the per-request context, so a call blocked on I/O stops
promptly, and keeps a poll flag as a convenience. The trait adapter always
returns `pending` and completes through the host callback, which is what the
prototype measured; the ABI keeps its inline-result path for plugins that
implement the ABI directly. The authoring request and response types are
serialisation types, the trait requires its implementation to be default
constructible, and it imposes `Send + Sync + 'static` on the plugin and `Send` on
every capability future because the worker hop moves the future. The generated C
header is committed, layout is asserted in Rust and re-asserted by a C
translation unit compiled in CI, and the assertions are written in terms of the
pointer size so they are not silently 64-bit specific. `metis-plugin-sdk`
re-exports the trait and adapters and owns the async executor, helpers, core
plugins, and CLI.

**Walking skeleton.** One ordinary RPC and one event stream, generated from the
contract, served over the local transport and driven by the real client, as a
test and an example rather than as a product endpoint. It is the acceptance
proof for the effort: if it runs, the contract, the generated code, the
transport, the client, and the streaming shape are all real.

**Contract technology.** The decision to use Protobuf with ConnectRPC rather than
OpenAPI with JSON and server-sent events is recorded in ADR 0001, with both
measured results. The OpenAPI spike remains in the tree as the measured
counterfactual.

## Testing Decisions

A good test drives the public seam and asserts observable behaviour. It uses a
real transport on a temporary path rather than mocking sockets, it does not
assert on generated internals, and it does not restate the contract. Three seams
are used, the fewest that still catch the failures that matter.

**Wire seam.** The skeleton service plus the real client over a temporary Unix
domain socket. It covers the ordinary RPC, the stream and its snapshot, resume
from a cursor, the expired-cursor error, explicit cancellation, the slow-consumer
drop, and the typed authentication errors. This is the highest seam in the
effort, and the prototype harnesses are the prior art: the ConnectRPC spike
already drove exactly this shape, and the mechanics spike's host prints a
PASS or FAIL line per check and exits non-zero on failure, which is the pattern
to keep.

**Plugin ABI seam.** A host loading a fixture plugin compiled as a shared library
through `plugin-contract`. It covers symbol resolution, reading an older
shorter table, inline and pending completion, cancellation, per-side buffer
release including a dropped completion, panic containment, the authoring trait
translation, and the generated header's layout agreement with Rust. Prior art is
the two ABI prototypes, which reached this exact point and whose harnesses are
in the tree. The fixture library must be built before the harness runs, because a
plain `cargo run` of the harness does not rebuild the shared library and would
load a stale one.

**Contract gates.** `buf lint`, `buf breaking` against `main`, and the
regenerate-and-diff check, all in CI. These are gates rather than tests, and the
regeneration script from the ConnectRPC spike is the prior art. The workspace's
existing formatting, clippy with warnings denied, and test steps continue to
apply, with the generated trees covered rather than excluded.

Platforms: the full suite runs on Linux in CI. The Windows pipe path is compiled
for the Windows target, which is a compile-only check until a Windows runner
exists. TLS tests generate a self-signed certificate into a temporary directory
and exercise both trust paths, the operating system store case being asserted
against that temporary anchor rather than a public host.

## Out of Scope

- The real daemon: its configurable, proxy-aware listeners, orchestration,
  storage, scheduling, provider implementations, agent declarations, plugin
  adapters, and workflow hosting belong to the daemon effort, as does the
  terminal crate.
- The agent loop, sessions, and workflows (agent core), the plugin host runtime
  (plugin host), and the authoring SDK, core plugins, and CLI (plugin SDK).
- The Tauri and Android shells and the shared frontend (surfaces).
- Provider implementations beyond the provider capability contract.
- A standalone browser or JavaScript client, and iOS entirely.
- Any UI, including the terminal UI library choice.

## Further Notes

Fog handed to this spec, all recorded on the effort map:

- TLS over Connect with rustls is unverified, because the spike proved a static
  musl build without a handshake.
- The pinning and upgrade policy for the pre-1.0 `connectrpc` and `buffa` pair,
  which move together.
- Whether the Connect JSON codec is enabled for outside clients, and what that
  means for the auth and error model.
- Whether the generated code's formatting step is a script step or an exclusion.
- How the client distinguishes a deliberate slow-consumer drop from a transport
  failure, so the resync decision is not guesswork.
- Windows pipe verification: the exact error a remote client observes, whether a
  low-integrity same-user process is denied, and the Windows runner needed.
- The snapshot contents per channel, and the numeric constants for the ring size,
  the queue bound, the heartbeat interval, the idle timeout, and the backoff.
- Whether plugin-produced events get their own channels.
- The gaps the authoring prototype exposed: a lifecycle hook, a route for
  `emit_event`, a consistency check between descriptor names and dispatch names,
  and the error variant that carries control flow.

Deviations from `docs/rough-plan.md` are recorded as they happened: OpenAPI and
AsyncAPI were replaced by Protobuf with ConnectRPC (ADR 0001), WebSockets and
server-sent events were dropped, and the daemon composition layer and the
terminal gained an owning effort. The prototype sources under the spikes tree are
kept as reference for implementation.
