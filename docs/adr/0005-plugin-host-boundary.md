# 0005. The plugin host boundary

- Status: accepted
- Date: 2026-10-15

## Context

The `metis-plugins` effort built the host side of the plugin system: the bundle
a plugin ships as, the verifier and extractor that stage it, the manifest reader,
the one asynchronous load entry, cancellation, the callback sink, the start-up
policy, teardown, and the end-to-end gate that loads the C fixture as a signed
bundle in CI. Each of those was decided in the effort's map and written into
`docs/plugin-contract.md`, and the effort's tickets recorded the detail.

Four of those decisions are cross-cutting: every later consumer inherits them,
and none of them is a detail of one ticket. This record collects them with
pointers, because a settled decision belongs in an ADR rather than only in a
contract section or a ticket report.

## Decision

**One signed bundle is the artifact a plugin ships as.** A `.metis` file is a
fixed-size container header, an X.509 chain over Ed25519 keys, a payload
signature, and a ZIP archive holding the manifest and one library per target
(`docs/plugin-contract.md` section 17). The host verifies the chain against a
trust policy the daemon supplies before it extracts or loads anything, refuses a
bundle it cannot verify, and treats a signature that is absent or does not verify
as a reason not to start. Signing covers everything a user installs, so there is
no second artifact to keep in step.

**Start-up has three outcomes, and the split is one predicate.** A trust failure,
a bundle file that cannot be read, a manifest that is absent, unreadable,
invalid, or not the name the chain carries, and a plugin that could not be
unloaded stop start-up. A container this build cannot read, an extraction
refusal, a missing or ambiguous target library, an ABI mismatch, a manifest the
library disagrees with, and a registration the host refuses disable that one
plugin while the daemon runs. A failed initialisation is unloaded and replaced by
a disabled placeholder (`docs/plugin-contract.md` section 19). The stop-versus-
disable split is the predicate `BundleError::is_trust_failure`, so a new refusal
is classified by what it means rather than by where it was detected.

**Cancellation belongs to the caller.** The caller owns the deadline and the
drop; the runtime holds no timer and no default deadline, cancels a snapshot of
an instance's in-flight calls, and records an unacknowledged cancel on the
ordering ledger while the instance keeps serving. An id the instance does not
hold answers the frozen `NotFound` without the plugin being entered, because the
plugin's own call registry is process-wide. The same rule reaches teardown: an
instance is released with the caller's deadline, and an instance that cannot be
released stays mapped and is reported rather than forced.

**One sink carries every callback.** Progress, events, logs and service requests
arrive at one destination set when the host is built, every delivery names the
instance, a payload arrives as an owned copy taken before the plugin's buffer is
released, a sink cannot fail and must not block, and a panic inside one is caught
and recorded rather than propagated. A host with no sink is legal. The host
exposes this one boundary instead of one per callback kind
(`docs/plugin-contract.md` sections 8 and 12).

## Consequences

- The bundle format, the container version, the ABI major and minor, and the
  plugin's own version are four distinct things. The container version and the
  ABI major must match this build, the ABI minor is a minimum, and the plugin's
  version gates nothing.
- The runtime starts no runtime and no thread. It hands back a future and the
  daemon polls it, which is what makes unloading safe: the ordering ledger is the
  evidence that nothing is inside plugin code when a library is unmapped.
- The daemon resolves configuration and secrets and supplies the trust policy;
  the runtime never reads the daemon's configuration files.
- A plugin is native code in the daemon's process. The ABI is not a sandbox, and
  a plugin that blocks inside initialisation hangs the daemon, which is accepted
  and documented rather than mitigated.
- A bundle's identity before its manifest is read is its file's stem: the daemon
  resolves configuration by that name, and a placeholder whose manifest could not
  be read is named by it. The manifest's name is the registration and routing
  identity once the manifest has been read.
- The authoring `Plugin` trait is part of this boundary, so a later change to it
  is a change the plugin authoring SDK inherits wholesale
  (`metis-plugin-sdk`).
- A bundle's own root certificate travels as the chain's last link, and every key
  id is recomputed from the public key, so a carried identifier is never trusted.

## Alternatives considered

- **A shared library as the shipped artifact.** That is what the effort replaced:
  nothing about such a file tells an operator who signed it, whether it fits this
  daemon, or how to release it.
- **Hot unloading.** Replacing a mapped library while the daemon runs needs
  lifetime guarantees that do not exist, so a configuration change is a restart
  and disable is registry state only.
- **A second loader in the start-up policy.** Driving verify, extract, manifest
  and load inside the policy would duplicate the one load path and make two
  classifications, so the policy classifies the load entry's outcome instead.
- **One sink per callback kind, or a queue inside the runtime.** Several
  destinations split the boundary a daemon must implement, and a queue would make
  the runtime own backpressure it does not own; a daemon that wants either puts
  it behind the sink.
- **Releasing on start-up failure inside the start-up pass.** That would use a
  deadline the runtime does not own, so a stopped pass hands its started
  instances back to the caller instead.
