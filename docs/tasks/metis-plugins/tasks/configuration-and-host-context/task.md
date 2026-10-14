---
type: task
subtype: grilling
title: Per-plugin configuration delivery and the explicit host context
status: stable
workflow_state: done
blocked_by: []
mode: human
---

## Decision to settle

How the daemon's resolved configuration reaches one plugin instance, and what
the explicit host context contains:

- the delivery path: a `metis.configure` control capability on the futures
  path, a field on `create_instance`, or a second synchronous table entry,
  given that the contract already moved `configure` and the counters onto the
  futures path;
- when it runs relative to `create_instance`, `list_capabilities`, and the
  first real call, and what happens between creation and configuration;
- the body shape: one structure carrying both the plugin's own configuration
  and the host context, or two separate payloads;
- what the host context carries: the instance name, the host's supported
  capability kinds and versions, the data directory a plugin may use, the
  daemon's identity, the trust policy, or a subset;
- whether the plugin's configuration is validated against a schema the plugin
  publishes, and what a violation means;
- what a plugin may refuse: whether a configuration the plugin dislikes is an
  error that disables the instance, quarantines it, or unloads it;
- where the host's supported capability versions live, since the landed
  per-capability gate is applied by the registry with a policy constant that
  the harness has a second copy of;
- secrecy: the runtime never reads the daemon's configuration, so the body
  carries only what the daemon chose to hand over, and it must be clear what
  must never appear there.

## Parent decisions

Already settled and not to be reopened:

- The runtime never reads the daemon's configuration or secrets; the daemon
  supplies each plugin its own configuration plus an explicit host context.
- `docs/plugin-contract.md` section 15's control-plane paragraph, which moved
  `configure` and the counters onto the futures path as per-call control
  capabilities, is **superseded** by D1 below. Section 14 item 7 says the same
  thing and is superseded with it.
- The capability descriptor is the shared
  `plugin-contract::capability::CapabilityDescriptor`, and the runtime no
  longer carries a second declaration type.
- The manifest declares the capability list and is cross-checked at load, so
  the host knows what a plugin advertises before it configures it.

## Choices already known

- `create_instance` already takes the host table and runs the plugin's
  `started`, and `list_capabilities` already runs on the table, so a
  configuration step is the third step of instance creation, and whether it can
  await is open.
- The host records logs, events and pending services today; the host context
  is the return path the plugin gets, and it has no fields yet.
- The fixture advertises eight `tool.*` capabilities and no control
  capability, so the new configuration entry is new surface in the fixture, the
  C plugin and the harness.
- The host's supported capability version policy exists twice today, as
  `HOST_CAPABILITY_VERSION` in the harness and as the registry's gate
  argument. This task is where the runtime's own policy is decided, and the
  harness copy then has one owner.

## Decisions recorded

**D1, configuration is its own ABI call and its own trait method, not a
capability.** The operator's decision on 2026-10-14: instance configuration is
not delivered through `metis.configure` or any other `metis.*` capability. It is
a dedicated entry on the plugin table (the operator's working name is
`init_plugin`) with a matching method on the Rust authoring trait. This
supersedes `docs/plugin-contract.md` section 14 item 7 and the control-plane
paragraph of section 15, and it puts configuration in the lifecycle half of the
ABI beside `create_instance`, `list_capabilities` and `shutdown` rather than in
the call half.

Consequences this decision already fixes, before the open questions below are
answered:

- `docs/plugin-contract.md` must be amended in the same effort: the plugin
table's rows, the authoring surface in section 12, and the two superseded
paragraphs.
- The ABI changes. A new entry is either appended after `service_complete`,
  which grows the table past its 80 bytes and is legal by the size-prefix rule,
  or an existing slot is reshaped. Either way `abi_minor` moves, and the
  generated header, the slot constants, the Rust layout assertions in
  `crates/plugin-contract/src/abi/layout.rs`, the C translation unit at
  `crates/plugin-contract/c/layout_static_asserts.c`, the fixture, the C plugin
  and the ABI harness all follow.
- The map's claim that `plugin-contract` is owned by `metis-foundations` and
  only consumed here no longer holds for the table: this effort appends or
  reshapes one entry and bumps the minor.
- The fixture's missing `metis.configure` control capability is no longer a
  gap. What is missing is the new table entry and its harness checks.

**D2, one call constructs and initializes.** `init_plugin` replaces
`create_instance` rather than sitting beside it, so an instance that exists has
completed initialisation. On the Rust side the guarantee the operator asked for
is type-level: whatever the registry holds has already run `init` to completion,
so an initialized plugin value is the only shape that reaches the daemon.

**D3, the initialisation entry is futures-shaped.** `init_plugin` returns a
future the daemon polls on its own runtime, so initialisation may ask the host
for a service and return `Pending` instead of blocking.

**D4, one payload with two parts.** The initialisation payload carries the
plugin's own configuration and the host context as two named parts of one
versioned body.

**D5, the manifest declares the configuration schema.** The host validates the
configuration against a schema the manifest declares, before the library is
mapped, since the manifest is the only declaration document that already exists
and is required.

**D6, the host context is permissive and forward-compatible.** Include any host
information that could be useful to a plugin and whose disclosure is harmless,
rather than a deliberately minimal set. A plugin must ignore context fields it
does not know, so adding a field never breaks an older plugin. This is the same
rule the payload types already follow.

**D7, the runtime owns the supported capability version policy.** One value in
the runtime, which the daemon may narrow, replacing the harness's duplicate
constant.

**D8, initialisation runs before registration.** The registry only ever receives
an instance whose initialisation resolved `Ok`.

**D9, the `init_plugin` entry's return contract.** The entry takes the host table
and the payload and fills two out-params, an instance pointer and a future
handle. The plugin writes the instance before its poll returns `Ok`, and the
instance is valid exactly when that future resolved `Ok`. On any other
resolution the host drops the future and then calls `destroy_instance` on the
pointer, which must tolerate a failed initialisation.

**D10, initialisation is baked into construction on the Rust side.** A plugin
value that exists has completed initialisation. The operator rejected the
`default()`-then-`init(&mut self)` shape because the type system would still
permit an uninitialised value to be passed around unnoticed, so there is no
`Default`-then-`init` path at all: the only way to obtain a plugin value is
through a fallible, asynchronous constructor. The concrete mechanism, an
associated constructor on the trait, a factory trait, or a wrapper type, is
still open.

**D11, two error classes and a disabled placeholder.** An unexpected failure,
meaning a bug or an invariant violation, is fatal and the daemon refuses to
start. An expected environmental failure, such as an unreachable host, invalid
credentials or a missing file, does not stop the daemon: the instance is
destroyed and the registry receives a placeholder built from the manifest which
mimics the plugin's shape, with the same name and the same capability list and
schemas, registered disabled. Its capabilities are listable, so a surface can
show what is installed and what it can do, but they are excluded from agent
execution contexts, and invoking one returns the disabled error the registry
already has, `RouteError::Disabled`, which is `PermissionDenied` on the wire.
Consequence: on the disabled path the manifest's declaration is authoritative
and is not cross-checked against `list_capabilities`, because there is no
instance to ask. The cross-check still applies on the successful path, and the
manifest's required capability list is exactly what makes a shape-preserving
placeholder possible without loading anything.

**D12, `counters` is removed.** The `plugin-counters-purpose` research traced the
word to the round 3 spike's D7, where it named the prototype cdylib's plugin-side
metrics capability (`metis.future_stats` and `metis.future_counters`). No
`metis.counters` string exists anywhere in the tree, nothing consumes it, and both
candidate needs are already covered: request-id allocation policy is the daemon's
per contract section 13, and the host's waker counts and ordering ledger are host
state. So v1 gets no counters capability and no counters table entry, and the
contract amendment this effort owes drops the member. The fourth occurrence, the
"leak counter" in section 14 item 4, is the quarantine predicate and a different
thing. The plugin-side buffer accounting (`allocs`, `frees`, `live`, `violations`)
stays as the authoring surface's own ledger.

**D13, the host context is a shared Rust type.** A type in `plugin-contract`,
feature-gated beside the authoring surface, whose field names are the contract
and whose JSON shape is documented for a C author who decodes by field name.
Unknown fields are ignored on decode, per D6.

**D14, the daemon-facing entry is one async function.** The daemon hands the
runtime the plugin archive path, the plugin's configuration, the host context,
and the host's policy inputs (the trust policy, the supported capability
versions, the event and log sink, the instance name, and the initialisation
deadline) and receives a future whose output value is the loaded and initialized
plugin. The daemon does not drive extraction, manifest validation, library
mapping, initialisation and the capability cross-check step by step; the runtime
owns that sequence and keeps exposing its pieces underneath.

**D15, the associated constructor is the mechanism for D10.** The `Plugin` trait
gains an associated asynchronous constructor returning `Result<Self,
PluginError>`, and loses its `Default` bound, so the only way to obtain a plugin
value is a fallible asynchronous constructor.

**D16, the classes are decided by whether the daemon's own state is knowable.**
The plugin side judges a failure as environmental (an unreachable host, a rejected
credential, a missing file) or as a defect (a caught panic, an invariant
violation). The daemon degrades, meaning it disables that plugin and keeps
running, exactly when the failure is confined to the plugin and the daemon's own
state is known to be intact. It aborts startup when the failure leaves the
daemon's state unknown, or when it shows that the daemon's own inputs are wrong.
So a plugin defect aborts, because an in-process plugin that failed unexpectedly
may have left shared state inconsistent, and a missing archive path or an absent
or invalid manifest aborts, because either one means the daemon's plugin
configuration, or the path built from it, is wrong. The operator's emphasis:
an unknown state crashes the daemon even when the failure happens during
initialisation and looks confined to the plugin, because "confined to the
plugin" is exactly what cannot be verified.

**D17, the disabled path unloads the library whenever it can.** Degrading a
plugin disables it and destroys its instance, and it also unloads the library,
because a library left mapped is code an attacker can still reach through some
other bug. What a refused unload means at startup is still open.

**D18, the error class crosses the ABI as one out-param on the init entry.** The
class applies to initialisation only, and the Rust authoring error type carries
it regardless, so widening it to call failures later would not change the surface
an author writes against. Inference from the frozen status space was rejected,
because a plugin returning `NotFound` for a missing remote host would be
indistinguishable from one whose own lookup logic has a hole.

**D19, the daemon-facing function yields live or disabled.** Its normal output is
a value that is either the loaded, initialized plugin or a disabled placeholder
carrying the manifest's shape and the reason. Only genuinely fatal cases are
`Err`.

**D20, the five bundle-level load failures degrade.** A signature that does not
verify, an archive refused by the extraction safety rules, an ABI major or minor
mismatch, a bundle with no library for this host's target, and a manifest whose
capability list disagrees with `list_capabilities` all disable that plugin and
leave the daemon running. None of them touches the daemon's own state, and
refusing to start on any of them would hand a start-up denial of service to
whoever can drop a file into the plugin directory. Each is reported loudly and
the bundle's library is unloaded.

**D21, a refused unload on the disabled path aborts startup.** After a failure
that degrades a plugin, the instance is destroyed and the library is unloaded.
If that unload is refused, the daemon refuses to start, because a library that
cannot be released means plugin code may still be live and the daemon's state is
not known to be intact.

**D22, a blocking `init_plugin` is accepted and documented.** A plugin that
blocks inside the entry itself hangs the daemon: the deadline covers the
asynchronous part only, because a future that never arrives cannot be timed out.
The ABI is not a sandbox, and this is the cost of running native code in process.

**D23, no live reconfiguration in v1.** A configuration change means
constructing a new instance: unregister, shutdown, drop every future, destroy,
unload, then run the load function again. The invariant that an instance was
initialised with its configuration therefore holds by construction, and nothing
has to define what a reconfiguration does to a call already inside the plugin. A
`reconfigure` entry stays available later as an additive table change.

## Completion record

**The decision in the operator's terms.** Instance configuration is a lifecycle
concern, not a capability. `init_plugin` replaces `create_instance`: one
futures-shaped entry constructs and initialises, takes the plugin's configuration
and the host context in one payload, and yields the instance, so an instance that
exists has been initialised with its configuration. The Rust trait mirrors it
with an associated asynchronous constructor returning `Result<Self,
PluginError>`, and `Default` disappears so no other way to obtain a plugin value
exists. The daemon gets one asynchronous function taking the archive path, the
configuration, the host context and the host's policy inputs, and receiving
either the live initialised plugin or a disabled placeholder built from the
manifest. Start-up either degrades one plugin or refuses to start, decided by
whether the daemon's own state is known to be intact. A plugin that blocks inside
the entry hangs the daemon, which is accepted and documented.

**Alternatives considered and rejected.**

- Keeping `metis.configure` as a per-call control capability, the contract's
  original plan in section 14 item 7 and section 15. Rejected: configuration is
  part of the strict contract between plugin and daemon, so it must not be
  something a plugin may or may not declare.
- A separate configuration call after `create_instance`. Rejected: it leaves an
  uninitialised instance representable.
- `Default` followed by `init(&mut self)`. Rejected: the type system would still
  permit an uninitialised value to be passed around unnoticed.
- Classifying failures by inference from the frozen status space. Rejected: a
  `NotFound` for a missing remote host would be indistinguishable from a
  `NotFound` produced by a bug in the plugin's own lookup logic.
- A synchronous configuration entry. Rejected: configuration may need to wait on
  a host service, and the contract's rule for waiting already exists.
- Per-call counters, and a counters table entry. Rejected: the research found
  the term orphaned, with both plausible needs already covered elsewhere.
- Live reconfiguration. Rejected for v1, available later as an additive entry.
- Refusing to start on a bundle-level failure. Rejected: it would hand a start-up
  denial of service to whoever can write to the plugin directory.

**Constraints and rationale.** The classification exists because plugins run in
process and can leave the daemon's memory in any state. The question at every
failure is therefore whether the daemon's own state is knowable: a signature
failure, an extraction refusal, an ABI mismatch, a missing target library or a
declaration that disagrees with the library leaves the daemon untouched, while a
plugin defect, a wrong daemon-side input or an unload that cannot be performed
does not. The disabled placeholder is possible only because the manifest's
capability list is required and is read before the library is mapped, so the
plugin's shape survives a failure that stops it from running.

**Dependent-task implications.**

- `compatibility-policy` inherits the five degrade cases: an ABI or bundle
  format mismatch disables that plugin rather than stopping the daemon.
- `teardown-and-unload` inherits the disabled path, which is destroy the
  instance, unload the library, and refuse to start if the unload is refused.
- `host-callback-surface` inherits the event and log sink as an input of the
  load function.
- `metis-daemon` owns the start-up policy, the trust policy and its keys, the
  data directory, and the filtering of disabled capabilities out of agent
  execution contexts.
- `metis-plugin-sdk` owns the author-facing error class, the associated
  constructor, and the manifest generation the placeholder depends on.
- The `plugin-contract` amendments this effort owes: the new table entry and the
  bumped minor, the generated header, the slot constants, `layout.rs`,
  `c/layout_static_asserts.c`, the table rows in section 5, the authoring
  surface in section 12, the ownership split in section 13, section 14 item 7 and
  section 15's control-plane paragraph, and the documented hazards, which are a
  blocking entry with no deadline and the class out-param beside the frozen
  status space.
- Tickets must update the fixture, the C plugin and the ABI harness for the new
  entry, and add a runtime surface that lists capabilities with their enabled or
  disabled state so the daemon can filter.

**Remaining fog.**

- Whether the data directory the context names is created by the daemon or by
  the plugin. The recommendation is the daemon, which owns the configuration.
- Whether the error class should later widen from initialisation to call
  failures. The author-facing error type carries the class already, so widening
  is additive.
- The exact name of the initialisation entry and of the daemon-facing function.

**Completion evidence.** The twenty-three recorded decisions above, the
rejected alternatives, the constraints and rationale, the dependent-task
implications, and the remaining fog, confirmed by the operator.

## Later amendment

The start-up policy above is amended on one point, after the `plugin-signing`
grilling. A signature that is absent or does not verify is **fatal**: the daemon
refuses to start, because it is a sign the machine has been tampered with. The
operator changed their mind on this case after the start-up discussion. The
other four bundle-level failures still degrade: an extraction refusal, an ABI
mismatch, a bundle with no library for this host's target, and a manifest that
disagrees with the library. The entry recorded above as the one about the five
bundle-level failures is superseded on the signature case only.

A second note, from the `teardown-and-unload` grilling. The decision in this task
that a configuration change means constructing a new instance is refined: the
daemon reads its configuration at startup, plugins are loaded and initialised at
startup and released at shutdown, and there is no live teardown path. So a
configuration change is a daemon restart, and the "new instance" that decision
describes is the one the next startup creates. Nothing else in the decision
changes.

A third note, from the `compatibility-policy` grilling. The disabled placeholder
recorded above is built from the manifest, so it has a name and a capability
list. One bundle-level failure has no manifest to build it from: a container
whose format version this build does not know cannot be parsed, so the archive is
never opened. That case still degrades rather than aborting, and its placeholder
is identified by the bundle's file name, carries no capabilities, and records
that a newer daemon is needed.
