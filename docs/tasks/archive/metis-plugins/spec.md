---
type: spec
title: Plugin bundles, signing, and the host side of the plugin system
status: stable
---

## Problem Statement

A plugin today is a shared library that the daemon maps. Nothing about that
artifact is distributable or safe to install. The operator cannot tell that a
plugin came from someone they trust, cannot tell whether it was built for this
daemon, cannot hand it configuration, cannot observe what it is doing beyond its
response payload, cannot cancel work it started, and cannot release it safely.

The plugin author has the mirror-image problem. There is no artifact to ship, no
way to state which ABI the plugin needs, no way to declare what it offers, and no
way for the daemon to refuse a bundle for a clear reason rather than a crash.

The daemon is left unable to answer three questions. May I run this code? Does it
fit me? And can I get rid of it again afterwards?

## Solution

A plugin ships as one signed file with the extension `.metis`. The file carries a
small versioned container header, an X.509 certificate chain over Ed25519 keys, a
payload signature, and a ZIP archive. The archive holds a manifest at its root
and one library per target it supports.

The daemon verifies the chain against a set of root keys and a revocation list
that are compiled in and can be overridden through its configuration. It refuses
anything it cannot verify, and a signature that is absent or does not verify
stops the daemon rather than being worked around. What it can verify, it then
validates: the manifest, the archive's extraction rules, the library for this
host's target, the ABI, and the manifest's capability list against what the
library reports. A failure that means "this bundle does not work here" disables
that one plugin with a stated reason and leaves the daemon running.

A plugin that loads successfully is initialised with its configuration and the
host context before it is registered, so nothing can dispatch to it before it is
ready. Its capabilities enter the registry, calls are dispatched through them,
and everything it says outside its response (progress, events, logs, service
requests) reaches the daemon through one sink. Calls are cancellable, plugins
load at start-up and are released at shutdown, and an instance that will not
release is reported rather than forced.

The whole path is proved in CI by a real sample plugin: the existing fixture,
packaged into a signed bundle with a hand-written manifest, loaded end to end
through the same entry the daemon uses.

## User Stories

1. As an operator, I want to install a plugin by placing one file in the plugin
   directory, so that installation is a file copy and not a build step.
2. As an operator, I want the daemon to refuse a bundle whose signature does not
   verify, so that altered code cannot run even if it was signed once.
3. As an operator, I want the daemon to refuse to start when a bundle in my
   plugin directory does not verify, so that I learn about tampering rather than
   discovering it later.
4. As an operator, I want an expired certificate to be treated as a failure to
   verify, so that a stale bundle cannot be replayed indefinitely.
5. As an operator, I want to revoke a key through configuration, so that I can
   act on a leaked key without rebuilding the daemon.
6. As an operator, I want to hold two root keys at once, so that I can rotate
   without a window where nothing verifies.
7. As an operator, I want the revocation list to name a key by something derived
   from the key itself, so that entries keep their meaning when files are renamed.
8. As an operator, I want the daemon to tell me which bundles it disabled and
   why, so that I can fix the installation rather than guess.
9. As an operator, I want a plugin built for another ABI to be disabled rather
   than to crash the daemon, so that one bad bundle does not take Metis down.
10. As an operator, I want a bundle with no library for my machine's target to be
    disabled with that reason, so that a multi-target bundle is not an error on
    every host that lacks one target.
11. As an operator, I want an archive that tries to escape the extraction
    directory, or that carries a symlink, to be refused, so that a hostile bundle
    cannot write outside its own directory.
12. As an operator, I want a bundle carrying a container version this build does
    not know to be refused and shown as needing a newer daemon, so that I can tell
    a stale daemon from a broken bundle.
13. As an operator, I want to see each plugin's own version in what the daemon
    reports, so that I can tell which build I am running.
14. As an operator, I want the plugin's own version to never gate loading, so
    that a version number I chose for my own reasons cannot disable a working
    plugin.
15. As an operator, I want to disable a capability without unloading its library,
    so that a routine toggle does not cost a reload.
16. As an operator, I want disabling to leave in-flight work alone, so that
    turning something off does not silently discard a running call.
17. As an operator, I want configuration to be read at start-up and a change to
    mean a restart, so that a plugin never observes two different
    configurations in one lifetime.
18. As an operator, I want plugin start-up to be bounded by the daemon's own
    shutdown deadline, so that one plugin that will not release cannot keep the
    process alive.
19. As an operator, I want the daemon to report which plugins it could not
    release at shutdown, so that I can decide whether to remove one.
20. As an operator, I want plugins to load at start-up and be released at
    shutdown, so that the lifecycle has no in-between state to reason about.
21. As a plugin author, I want to ship one file, so that signing covers everything
    a user installs and there is no second artifact to keep in step.
22. As a plugin author, I want to write a manifest in a plain text format, so that
    I can build a plugin without the SDK.
23. As a plugin author, I want the manifest to declare my plugin's identity, its
    version, the ABI it needs, its library paths per target, and its capabilities
    with their schemas, so that one file describes the bundle.
24. As a plugin author, I want to declare a minimum ABI minor, so that a daemon
    older than my plugin's needs refuses it instead of reading entries I added.
25. As a plugin author, I want a newer ABI minor in the same major to be accepted,
    so that the daemon can grow the table without breaking my plugin.
26. As a plugin author, I want the daemon to check my declared capability list
    against what my library reports, so that a stale manifest cannot mislead it.
27. As a plugin author, I want to receive my configuration and the host context
    in one call before my plugin serves anything, so that I have one place to set
    up and no ordering to guess.
28. As a plugin author, I want that call to be asynchronous and to return my
    instance only when it succeeds, so that I can read a file or open a socket
    during setup and report a typed failure if I cannot.
29. As a plugin author, I want my instance type to need no `Default`, so that I am
    not forced to invent a meaningless empty state.
30. As a plugin author, I want the manifest to declare my configuration schema, so
    that the daemon can tell me when it is handing me a configuration I cannot
    read.
31. As a plugin author, I want to be told which class of failure my setup produced
    (my fault or configuration's fault), so that I can report it usefully.
32. As a plugin author, I want to receive a request id with every call, so that I
    can report progress for the right call and answer the right service request.
33. As a plugin author, I want progress, events and logs to reach the daemon
    without a second protocol, so that I do not have to invent one.
34. As a plugin author, I want to ask the host for a timer or IO and be answered
    later with the same request id, so that I can do asynchronous work without
    blocking my poll.
35. As a plugin author, I want my buffers released through my own release entry,
    so that allocation and free stay on my side of the boundary.
36. As a plugin author, I want the host to hand the daemon its own copy of a
    payload, so that my buffer can be released as soon as the callback returns.
37. As a plugin author, I want to know that a callback happens inside my poll
    frame, so that I understand why the sink must not block.
38. As a plugin author, I want a wake after my future was dropped to be harmless,
    so that a race in my own code cannot cause a dangling write.
39. As the daemon, I want one asynchronous entry that loads a bundle and returns
    the instance when it is ready, so that start-up is one call rather than a
    sequence every caller must get right.
40. As the daemon, I want a bundle whose manifest is absent or invalid to stop
    start-up, so that I never run a half-described plugin.
41. As the daemon, I want a plugin that does not initialise to be unloaded and
    replaced by a disabled placeholder, so that the failure is visible and the
    library is gone.
42. As the daemon, I want a refused unload during that recovery to stop start-up,
    so that I do not continue in a state I cannot describe.
43. As the daemon, I want to receive configuration and host context from the
    runtime's own types, so that the daemon decides the values and the runtime
    delivers them.
44. As the daemon, I want to check the ABI major through the entry symbol, so that
    a library built for another major is refused before any table is read.
45. As the daemon, I want to read a newer minor's table and stop at my own
    layout, so that a plugin built against a newer daemon still serves the
    capability I know.
46. As the daemon, I want a capability whose descriptor version I do not support
    to be refused on its own, so that the rest of the plugin keeps working.
47. As the daemon, I want one sink for progress, events, logs and service
    requests, so that there is one boundary to implement and one to test.
48. As the daemon, I want the instance name on every callback, so that I can tell
    which plugin spoke when several share one library.
49. As the daemon, I want payloads delivered as copies owned by me, so that I can
    forward them onto my own stream without a lifetime rule.
50. As the daemon, I want a sink that cannot fail and must not block, so that a
    slow destination never stalls a plugin's poll.
51. As the daemon, I want a panic inside my sink to be caught and recorded, so
    that one bad log line cannot take down a plugin.
52. As the daemon, I want events from one plugin in the order that plugin emitted
    them, so that I can render a sequence faithfully.
53. As the daemon, I want a host with no sink to be legal, so that tests and the
    ABI harness need no destination.
54. As the daemon, I want to run without a thread or a runtime started by the
    runtime crate, so that all scheduling decisions stay mine.
55. As the daemon, I want to cancel a call by request id, so that a client that
    walked away does not leave work running.
56. As the daemon, I want a plugin to resolve a cancelled call as `Cancelled`, so
    that cancellation is a result rather than an abrupt teardown.
57. As the daemon, I want to own the deadline for each call, so that the timeout
    policy is mine and not a hidden default.
58. As the daemon, I want an unacknowledged cancel to be recorded and to leave the
    plugin serving, so that a slow call does not disable a working plugin.
59. As the daemon, I want cancelling an unknown or already finished request id to
    answer `NotFound` without touching the plugin, so that the boundary stays
    cheap and the plugin is not asked about state I hold.
60. As the daemon, I want to cancel every call on an instance at shutdown, so that
    teardown has one loop to run.
61. As the daemon, I want a dropped call to release its buffers and cancel its
    service requests, so that nothing is ever delivered into a released context.
62. As the daemon, I want one asynchronous entry that releases an instance,
    unloads its library, and reports what it did, so that shutdown is as
    checkable as start-up.
63. As the daemon, I want an instance that cannot be released to stay mapped and
    be reported, so that I never unmap a library that may still be running.
64. As the daemon, I want a quarantined instance to keep its library resident, so
    that an abandoned poll cannot become a dangling pointer.
65. As a plugin author, I want the daemon to wait for my future to be dropped
    before it unmaps my library, so that my code is never freed underneath me.
66. As the operator, I want the C fixture and its hand-written manifest to load
    through the same path as any other bundle, so that the format is proved
    writable without the SDK.
67. As a maintainer, I want the sample plugin to be packaged and loaded in CI, so
    that a regression in the bundle format fails a pipeline rather than a user's
    installation.
68. As a maintainer, I want the bundle's bytes to be tested against committed
    vectors, so that a second implementation can be checked against ours.
69. As a maintainer, I want the host-facing frames asserted not to panic where
    that can be proved at build time, so that the boundary has fewer ways to
    abort the daemon.
70. As a maintainer, I want the plugin table's documentation, generated header
    and C static assertions to change in the same commit as the table, so that
    the C ABI cannot drift from its description.

## Implementation Decisions

### The bundle and its container

- A bundle is a ZIP archive with the extension `.metis`. Extraction refuses an
  absolute entry name, an entry name carrying `..`, a symlink or hardlink entry,
  entry permissions, and an entry over a size limit. A refused extraction disables
  the plugin rather than stopping the daemon.
- The file begins with a fixed-size big-endian header: magic, container format
  version, signing suite, header length, chain length, signature offset,
  signature length, a reserved field that must be zero, and the ZIP length. The
  file length must equal signature offset plus signature length plus ZIP length,
  so no byte escapes the signed range.
- The magic and the container format version are at fixed offsets and are the
  only header fields pinned across versions, because the version determines the
  rest of the header's shape and therefore where the signature is. The version is
  a single integer that must match this build's exactly. The manifest does not
  repeat it.
- The chain region holds a concatenation of DER certificates. Certificates are
  self-delimiting, so the verifier requires the region to be consumed exactly,
  with no trailing bytes.
- The payload signature is raw Ed25519 over the header, the chain and the ZIP
  bytes, with the signature field omitted rather than zero-filled, under a domain
  tag that is version stable and independent of the container format version.
- One archive holds one library per target, keyed by operating system and
  architecture together. A bundle holds one or more targets at the producer's
  choice.

### Signing and trust

- The signature scheme is Ed25519 with SHA-256 key ids.
- Each link is an X.509 v3 certificate over an Ed25519 key. The child's key is
  the certificate's public key, the parent signs the certificate, and the
  chaining rule is that the child's issuer equals the parent's subject.
- The profile the project owns: a certificate that authorises another key carries
  `basicConstraints` with `CA:TRUE`, the leaf-authorising certificate is an end
  entity, any critical extension the verifier does not understand refuses the
  chain, and the leaf-authorising certificate's common name is the plugin name and
  is checked against the manifest. Intermediate subjects are informational and are
  not scoped to a name, because a prefix restriction would not bound a hostile
  certificate holder.
- Validity is the certificate's own not-before and not-after, at second
  granularity, checked against the clock with a small documented skew. An
  out-of-window certificate is a trust failure.
- The daemon holds a set of root keys and a revocation list. Both are compiled in
  with empty defaults and both can be modified through the daemon's configuration
  file, so a rotation can overlap and a leaked key can be revoked without a
  rebuild.
- Every key id is recomputed from the public key as the first 16 bytes of its
  SHA-256. A carried identifier is never trusted. No computed id of any key in the
  chain may appear in the revocation list.
- A chain holds at most eight links, and the count is derived from the region
  rather than carried.
- Parsing uses a strict DER decoder, which refuses non-canonical input before it
  is interpreted. This is what makes re-encoding the signed certificate body safe
  for the path check.
- The refusal rules for the container are part of the contract: an unknown
  container version, a mismatched signature offset and length, bytes before the
  header, trailing bytes in the chain region, and more than eight links.

### The manifest

- The manifest is a plain text file at the archive root, required in every bundle.
  It declares the plugin's identity, the plugin's own version, the ABI major and
  the minimum ABI minor the plugin needs, the library path per target, the
  capability list with each capability's name, kind, version, request schema and
  response schema, and the configuration schema.
- The manifest is generated from the same source as the plugin's own capability
  declaration, and the daemon cross-checks the declared list against what the
  library reports. A disagreement disables the plugin. A plugin built without the
  SDK writes the manifest by hand, and the C fixture is the proof that this is
  practical.
- The capability list in the manifest is what the daemon trusts for discovery, and
  the library's own listing is the cross-check.

### Initialisation, configuration and host context

- Configuration is a lifecycle step, not a capability, so the control-plane
  member that fetched a schema is gone and the schema lives in the manifest.
- One entry replaces the old instance-creation entry. It is asynchronous, it
  receives one payload carrying the configuration and the host context, and it
  returns two out-parameters: the instance and a future. The instance is valid
  only if that future resolves successfully.
- The host-side shape is an associated asynchronous constructor returning the
  plugin's own error type, and the instance type is not required to implement
  `Default`.
- A plugin's setup failure falls into two classes: one that means the plugin is
  defective and one that means the configuration does not match what the plugin
  declared. Both are distinguishable by the caller.
- The daemon reads its configuration at start-up. There is no live
  reconfiguration, so a configuration change is a daemon restart.
- A plugin that blocks inside its setup call hangs the daemon. This is accepted
  and documented: the ABI is not a sandbox.
- One asynchronous runtime entry loads a bundle: it verifies, extracts, validates,
  selects the target library, checks the ABI, initialises, and registers. Callers
  do not drive those steps.

### Compatibility

- The ABI major is part of the entry symbol and must match. A newer minor inside
  the same major is tolerated by the size-prefixed table, which stops at this
  build's own layout. An older minor is refused before any entry is read.
- The manifest states the ABI major and the minimum minor the plugin requires. The
  host refuses a differing major or a host minor lower than the required one. No
  range syntax.
- A capability whose descriptor version this build does not support is refused on
  its own, and the rest of the plugin keeps serving.
- The plugin's own version is informational. It never gates a load, and a loaded
  instance keeps working when a newer version appears on disk.
- The checks run in one order: magic, container version, signature and chain,
  manifest shape, capability cross-check, the library for this host's target, the
  ABI, the per-capability descriptor versions, then registration. Each failure
  reports itself and never a later consequence.
- The outcomes follow the daemon's start-up policy. A trust failure, an absent or
  invalid manifest, and a refused unload stop start-up. An extraction refusal, a
  missing target library, an ABI mismatch, and a manifest that disagrees with the
  library disable that plugin. An unknown container version disables it too, with
  a placeholder identified by the bundle's file name, because there is no
  manifest to name it and no archive to read.

### The disabled placeholder

- A disabled plugin is a registry entry built from what is known. When the
  manifest was readable it carries the plugin's name, its capabilities and the
  reason; when only the file was readable it carries the file's name, no
  capabilities, and the reason.
- A disabled entry is listable and excluded from agent execution contexts. An
  invocation that reaches one answers with the existing disabled route error and
  its reason.

### The host, the sink and the callbacks

- One sink trait carries progress, events, logs and service requests. It is set on
  the host when the host is built, and the default implementation records into the
  accessors the existing tests read, so a host with no sink is legal.
- Every callback names the instance. Progress and service requests also carry the
  request id, an event carries its topic and payload, and a log carries its level
  and message. Events carry no request id, because the frozen callback signature
  has none, so a plugin that wants correlation puts an id in its payload.
- The log level is a typed value over the four documented levels.
- Payloads are delivered as owned copies taken before the plugin's buffer is
  released. A sink never holds a reference into plugin memory and never calls the
  plugin's release entry.
- A sink method is infallible and must not block. A panic inside one is caught in
  its own frame and recorded on the ordering ledger like any other callback panic.
  The runtime keeps no queue on a sink's behalf, so a daemon that wants
  backpressure puts its own queue behind the sink. A sink must not call back into
  the runtime or the plugin, because it is called inside the plugin's poll frame.
- Delivery is per instance and in emission order, under that instance's lock.
  Callbacks from different instances have no ordering relationship, and nothing is
  promised about a callback's position relative to its call's completion.
- A service request's answer is the status the frozen callback expects, so the
  sink accepts or refuses synchronously, and the daemon answers later through the
  runtime's existing service-completion entry.
- Absence of panics in the host-facing frames is asserted at build time where the
  tool allows it: the four callbacks, the waker's wake and clone, the poll and
  drop wrappers, the buffer release path, and the sink delivery path. This is an
  additional compile-time assertion, not a replacement for the per-frame catch.

### Cancellation

- The caller that starts a call owns its deadline and passes nothing to the
  runtime, which holds no timer and no default deadline.
- Cancelling marks the plugin's cancellation token through the frozen table entry.
  The plugin is expected to resolve the call as `Cancelled` on its next poll.
- Cancelling a request id the host does not hold answers the frozen `NotFound`
  status, and the plugin is not called. An id belonging to another instance cannot
  occur, because cancelling is a method on the instance and the in-flight set is
  per instance. Cancelling a live id again is accepted.
- If the plugin does not resolve as cancelled before the caller's deadline, the
  caller drops the future. The runtime records the unacknowledged cancel on the
  ledger and the instance keeps serving. Quarantine remains reserved for an
  abandoned poll and an unreleasable teardown, because the drop path reclaims the
  plugin's context and a slow call is not an unsafe one.
- The instance-wide form of cancellation is a host-side loop over the instance's
  own in-flight set. The plugin table does not change.
- Dropping a call releases every plugin buffer the host still holds for that call
  before the context is reclaimed, and cancels that call's outstanding service
  requests, so no completion is ever delivered into a released context.
- A call the plugin cancels reaches its caller as the frozen `Cancelled` status.
  A call the caller dropped yields nothing to that caller. No new error variant is
  introduced for cancellation.

### Teardown, disable and shutdown

- One asynchronous runtime entry releases an instance: it unregisters the
  instance, cancels or drains in-flight calls to the caller's deadline, calls the
  plugin's shutdown entry, drops every future, destroys the instance, unloads the
  library, and reports the ledger entries it added.
- Plugins load and initialise at daemon start-up and are released at daemon
  shutdown. There is no live teardown and no voluntary deinitialisation of a
  running plugin.
- An instance that cannot be released stays mapped and is reported. This covers a
  shutdown that returns a non-ok status, a quarantined instance whose abandoned
  poll never returns, and an unload refused for any other reason.
- Disabling is registry state only. It refuses new calls and leaves the instance,
  its in-flight calls and its library alone. Enabling accepts calls again. Neither
  tears anything down.
- The daemon bounds its own shutdown with a deadline, waits for clean teardown of
  every instance, and on expiry reports which instances could not be released and
  exits, leaving those libraries mapped.
- Loading a degraded plugin attempts an unload, and a refused unload stops
  start-up, because at that point the daemon's state is not yet describable.

### The runtime's own contract

- The runtime starts no thread and no runtime of its own. Reclaim of a plugin
  future belongs to the caller on the poll return, and an abandoned poll
  quarantines the instance.
- The ordered ledger of lifecycle steps stays the mechanism that makes unload
  safety checkable.
- The error surface stays as it is: one type for load and dispatch, one for
  routing and validation, one for registration, layered over the frozen status
  space. Staleness, ABI and trust are three distinct typed errors.
- Three defects found by the hazard prototype are fixed in the authoring bridge:
  the value handed to the plugin's poll leaks one reference per call because it is
  never released; the wake-after-drop record cannot name the call it belongs to
  because the call state carries no request id; and a second wake on one consumed
  clone over-releases it.
- The wake-after-drop record is a counter rather than a ledger line, so the
  contract's wording about it is adjusted rather than extended.
- A drop after unload is a fault reachable only through the raw table, because the
  instance ties every future to itself and the unload refuses while one is alive.
  It lands as a fault test in a subprocess, and no outstanding-clone gate is added
  to the unload.
- The counters surface is removed from the contract and from the version, because
  the word traced to a spike's plugin-side metrics capability, no consumer exists,
  and both candidate needs are met elsewhere.

### Contract amendments

- The plugin table gains the initialisation entry and loses the counters entry. The
  ABI minor is bumped because the table changes, and the header carries the major
  used in the entry symbol.
- The contract text, the generated C header, the slot constants, the layout
  definitions, the C static assertions, the fixture, the C plugin and the ABI
  harness all change together with the table.
- The contract documents the container header, the certificate chain, the domain
  tags, the key-id rule, the revocation rule and the refusal rules, so that a
  second implementation can be written from the contract alone.

## Testing Decisions

- A good test here exercises the runtime's public surface with real bytes: a real
  bundle file, a real signed chain, a real cdylib, and a real poll cycle. No test
  reimplements the ABI, and no test asserts a private detail of the verifier or
  the loader.
- The seams are deliberately few. One new seam carries the whole feature: the
  runtime's public entry points, driven by integration tests that build a bundle on
  disk and load it. The existing raw seams stay where they are and are not
  duplicated: the ABI harness, which loads the fixture library directly and drives
  the table, and the C plugin build, which compiles the fixture under two
  compilers with warnings as errors.
- The end-to-end gate for the sample plugin reuses the same seam rather than adding
  a binary: a packaging step writes a signed bundle from the fixture's library and
  a hand-written manifest, and an integration test loads that bundle through the
  same entry the daemon uses, dispatches one capability, observes one event, and
  releases the instance cleanly.
- Prior art to follow: the ABI harness's shape of a real host that prints one line
  per check and exits non-zero on failure; the runtime's existing integration tests
  and their support module, which build the fixture cdylib before loading it so a
  stale library cannot be tested; the header regeneration check, which fails when
  the generated artifact and its source disagree; and the fixture's own dual
  compiler build.
- Tests to write, by module. The runtime's bundle path: extraction refusals,
  signature and chain acceptance and refusal, container version mismatch, missing
  target library, manifest absence, invalid manifest, manifest and library
  disagreement. The compatibility path: major mismatch, newer minor tolerated,
  older minor refused, capability descriptor version refused alone, plugin version
  ignored. The lifecycle path: initialisation with configuration and host context,
  the two setup failure classes, registration, dispatch, the disabled placeholder
  and its route error, disable and enable, teardown ordering, the ledger contents,
  and the report for an instance that cannot be released. The callback path:
  delivery of each callback kind, the instance name on each, the copy semantics,
  per-instance ordering, a panic inside the sink, and a host with no sink. The
  cancellation path: `Cancelled` resolution, an unknown id answering `NotFound`
  without touching the plugin, an unacknowledged cancel being recorded without
  quarantine, and the release of buffers and service requests at drop.
- A committed signing vector is tested at the byte level: one known-good bundle and
  the refusals that must follow from altering one field, so a second
  implementation can be checked against our bytes and our rules.
- The vector and the sample bundle are signed with a test key that exists only in
  tests and in the harness. The daemon's compiled-in root set stays empty, so no
  shipped build ever trusts the test key, and the tests install it as a root
  explicitly.
- The fault test for a drop after unload runs in a subprocess, because the
  behaviour under test is a fault.
- The panic-free assertion runs in a dedicated CI step with the feature enabled in
  a profile that links, unwinds and optimizes enough for the check to be provable.
  It is documented as the only place the assertion holds, so nobody reads it as a
  guarantee in the development loop.
- The existing checks continue to gate every change: formatting, the lints with
  warnings denied, the test suite, the contract lint and breaking-change checks,
  the header regeneration check, the C plugin build, the ABI harness, and the
  contract's own no-default-features build.

## Out of Scope

- The plugin authoring SDK: the export macros, the associated constructor
  generation, the manifest generator, and the signing CLI. That work belongs to
  the plugin SDK effort. This effort defines what such a tool must write and
  consumes it through a script in its own tests.
- The daemon's own wiring: plugin discovery, the configuration file's resolution,
  the realtime stream that events are forwarded onto, a user-visible cancel
  request, and the deadline values themselves. The runtime offers the boundary;
  the daemon's effort consumes it.
- Hot unloading, meaning replacing a mapped library while the daemon runs.
- Sandboxing, resource limits, and any claim that a plugin is safe to run. The ABI
  is not a sandbox, and a plugin that blocks hangs the daemon.
- Non-plugin ways to extend the daemon, the core plugin set, the network
  contract, workflow step implementations, and the agent loop.
- Live reconfiguration of a loaded plugin.
- Multi-target bundles beyond selecting the one library that matches this host.

## Further Notes

- One defect sits in this effort's territory and is not a decision: the contract's
  build script compiles a C translation unit whose static assertion message form
  MSVC rejects, so the runtime cannot be compiled for the Windows target and the
  Windows CI job is narrowed to the API contracts crate. The tickets absorb it,
  and the Windows job widens when it is fixed.
- The spec pins what the decision records left at spec level: the container
  header's field widths, offsets and magic value; the manifest's exact field names
  and types; the configuration file's representation of root keys and revocation
  entries; the signing crate used by the issuing side; the exact trait and method
  names of the sink; where the log level type lives, given that the ABI passes an
  integer; and the daemon's default shutdown and call deadlines.
- The dependency cost is measured rather than estimated. The trusted parse path
  gains a strict DER decoder and a certificate parser, both without a C build step,
  on top of the Ed25519 verifier. A C toolchain was deliberately kept out of the
  path that reads untrusted bytes.
- The glossary's terms are used throughout: daemon, plugin, bundle, capability.
  "Extension" and "integration" are not used for a plugin.
