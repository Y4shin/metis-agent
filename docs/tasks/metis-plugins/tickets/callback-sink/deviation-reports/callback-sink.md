---
type: deviation report
title: Deviation report for callback-sink
status: stable
---

## Deviation report: callback-sink

Compared: the arch spec's `callback-sink` section, the ticket doc and its seven
acceptance criteria, the tdd result, and the implementation on
`ticket/callback-sink` (`git diff task/callback-sink..ticket/callback-sink`: nine
files, 1612 insertions, 118 deletions, over landing branch `66a4ad3`, tip
`c611deb`, nine `wip:` commits).

The seven criteria hold against the code: the `Sink` trait carries one method per
callback kind and every one names the instance; the payload the sink is handed is
copied before the plugin's buffer is released; a sink's panic is caught in the
callback's frame and recorded on the ordering ledger while the plugin's call
finishes; delivery is ordered by a per-instance lock; a service request is
answered synchronously inside the callback and completed later through
`Instance::complete_service`; a host with no sink records into the existing
accessors and no existing test or the ABI harness changed; and the `no-panic`
check runs in a CI step of its own with its scope documented. What follows is
where the built surface differs from the plan.

### API surface changes

**1. `Host::for_instance(name)` is new public surface.**

- **Planned:** the arch spec's Exports for this ticket are the `Sink` trait,
  `Host::with_sink`, and a recording default; "every callback names the
  instance" is a property of the planned surface, not an item on it.
- **Actual:** the ABI carries one opaque pointer per host table, so the name has
  to travel in the table an instance was built with. `Host::for_instance(impl
  Into<String>)` builds a host value whose table names that instance and shares
  the sink, the records and the ordering ledger with the host it came from. The
  load entry applies it with the manifest's name
  (`crates/plugin-runtime/src/load.rs`), and any clone of a `Host` keeps the
  table's target alive, so the pointer stays valid as long as an instance does.
- **Impact:** one added public item, and the only way to name an instance on the
  raw path. `async-initialisation`'s `load_bundle` does it for every bundle, so
  `startup-policy`, `teardown-and-release` and `sample-bundle-ci` need none of
  it. A caller that drives `Loaded::init_plugin` by hand without it delivers the
  empty string as the instance name.

**2. The payload a sink receives is the decoded body, not the ABI bytes.**

- **Planned:** "Payloads arrive as owned copies taken before the plugin's buffer
  is released." The plan fixed the copying and not the framing.
- **Actual:** progress, event and service-request bodies are enveloped on the
  wire while a log message is not, so the runtime strips the one-byte envelope
  and hands the sink the decoded body, as it already does for call responses and
  capability lists. The log message is the topic/message string itself.
- **Impact:** the outward-facing shape of a sink depends on it.
  `sample-bundle-ci` observes a decoded JSON body for an event, not
  envelope-plus-JSON, and any daemon forwarding a payload forwards the body.
  This is a contract decision a dependent cannot infer from the trait alone.

**3. A host built with a sink answers nothing through the recording accessors.**

- **Planned:** "plus a recording default that keeps the existing accessors and
  tests working", and "a host with no sink stays legal".
- **Actual:** `Host::new()` installs the private `RecordingSink` over a private
  `Records`, and the accessors read that storage. `Host::with_sink(...)` builds a
  host whose `Records` is empty, so `logs()`, `events()`, `pending_services()`
  and `take_service()` answer nothing once a sink is installed. `Records` and
  `RecordingSink` are crate-private, so a caller that wants both a destination
  and the accessors implements its own recording sink.
- **Impact:** a dependent reading the accessors after installing a sink sees
  nothing rather than a copy. The existing tests and the ABI harness build hosts
  with `Host::new()` and are unchanged; a new consumer must pick one or the
  other.

**4. Two refusals the plan did not name, answered on the ledger rather than by
inventing a value.**

- **Planned:** the plan fixes the four levels and the payload copying, and says
  nothing about a callback whose bytes are unreadable.
- **Actual:** a buffer that is not one of this build's payloads, and a log level
  outside the four the contract documents, are recorded on the ordering ledger
  and nothing is delivered. `LogLevel::from_abi` returns `Option`, so no fifth
  level exists to deliver.
- **Impact:** a hostile or out-of-date plugin cannot make the host invent a level
  or hand a sink bytes it cannot interpret; a daemon sees the refusal in the
  ledger it already reads.

**5. The panic-free assertion covers the four hand-off frames, not the delivery
frames or the other frames the effort spec listed.**

- **Planned:** the arch spec's Seams say "the `no_panic` assertion over the
  host-facing frames"; the effort spec lists the four callbacks, the waker's wake
  and clone, the poll and drop wrappers, the buffer release path and the sink
  delivery path.
- **Actual:** measured over eight experiments, `no_panic` passes a frame only
  when every unwinding path in it is one a caught closure holds, and it fails on
  a frame that locks a `std` mutex, drops an allocation or drops a caught panic
  payload. So the four ABI frames became thin hand-offs to `deliver_*` frames of
  the same shape that carry the copy, the plugin's release entry, the ledger and
  the sink call, and the four hand-off frames carry the attribute. The waker
  frames, the poll and drop wrappers and the buffer release path are not
  annotated.
- **Impact:** the assertion is narrower than the effort spec's sentence, and the
  per-frame `catch_unwind` remains the runtime guarantee it adds to rather than
  replaces. `docs/testing.md` records the measured scope, including the two
  profiles in which the check proves nothing (no link, or no unwinding). The
  arch spec should be reconciled to the measured list rather than the
  aspirational one.

**6. One lock discipline replaced `.lock().unwrap()` crate-wide.**

- **Planned:** not planned, and not part of the sink surface.
- **Actual:** `fn lock` in `crates/plugin-runtime/src/lib.rs` takes a poisoned
  mutex's contents, and every host-side lock in the crate now goes through it.
- **Impact:** after a contained panic the host keeps reading its records instead
  of panicking again on a poisoned lock, which is what makes criterion 3's record
  observable at all. It is a behaviour change for every locked path in the crate,
  not only the sink's, and it is deliberate and documented with a `rule:`
  comment.

**7. A new optional dependency, feature and build profile.**

- **Planned:** the level's dependency posture in the arch spec says the level
  adds no dependency, and the plan's seam list places the check in "a dedicated
  CI step with the feature enabled".
- **Actual:** `no-panic` 0.1 (resolved 0.1.37) as an optional, feature-gated
  build-time dependency; the crate feature `no-panic-check`; and the workspace
  profile `no-panic` (`inherits = "dev"`, `opt-level = 1`). With the feature off,
  which every shipped and default build uses, the resolved graph is unchanged.
- **Impact:** the default and shipped graphs are untouched, so no dependent
  changes. The cost is three new configuration surfaces a reader must know about,
  and a pinned proc-macro crate the CI step now depends on.

### Abstraction usage

- Used/was specified: **yes**. `Host`'s recorded callbacks became the private
  default sink's storage rather than a second path, so `logs()`, `events()`,
  `pending_services()` and `take_service()` keep their behaviour for a host with
  no sink; `catch_unwind` with `AssertUnwindSafe` guards each frame as the rest
  of the crate does; the log level is a plain enum with the integer mapping at
  the ABI edge, which is one of the two forms the plan allowed; and the ordering
  ledger carries the panic record rather than a new log surface.
- The do-not list held in full: no queue and no backpressure in the runtime, no
  second callback path for logs, and the sink is documented as unable to call
  back into the runtime or the plugin. No ABI type changed and no contract text
  changed.
- `fn lock` is an addition to the crate's own abstractions, not a
  reimplementation of anything the plan named.

### Out-of-scope changes

- `fn lock` applied beyond the sink path (deviation 6): `lib.rs` only, same file
  the ticket already touches.
- The load entry now builds the instance's host view (`load.rs`), which the
  interface contract requires but the plan's file list did not name.
- `Cargo.toml`, `Cargo.lock` and `crates/plugin-runtime/Cargo.toml` for the
  optional dependency, the feature and the profile (deviation 7).
- `.github/workflows/ci.yml` (+10) and `docs/testing.md` (+64): the dedicated CI
  step and command 11, both inside the plan's seam list.
- `crates/plugin-runtime/src/sink.rs` (new, 185 lines) and
  `crates/plugin-runtime/tests/sink.rs` (new, 937 lines, 10 cases).
- Nothing was removed, and no test file outside this ticket's named seam was
  touched: `tests/registry.rs`, `tests/concurrency.rs`, `tests/waker.rs`,
  `tests/bundle*.rs` and the ABI harness are untouched, which is what makes
  criterion 6's "unchanged" claim checkable.

### Ticket doc update needed?

Yes, and the landing step creates the section. It should record: the seven
deviations above, with the two that outlive the ticket (the decoded-body framing
and the empty accessors on a sink host) stated as the sink's contract; the
measured scope of the `no-panic` check and the eight measurements it took to find
it; the dependency measurement as the tdd result gives it (90 crates on normal
edges both before and after with the section 17.7 pipeline, because the added
crate is behind a feature and the filter drops its `(proc-macro)` line; 114 to
115 with the feature on and the filter absent); the gate results; and one
documentation defect to fix in the harvest rather than here, the dependency
pipeline itself.

That defect: `docs/plugin-contract.md` sections 17.7, 17.8 and 17.9 present
`grep -v '(*)'` as the filter that "drops the marker lines" `cargo tree` prints
for a repeated subtree. In basic regular expressions `(*` means zero or more
literal `(`, so the pattern matches any line containing `)`, and the pipeline
drops every line carrying a parenthesised suffix, including path dependencies
such as `name v0.1.0 (/path)`. Measured: the three-filter run keeps the path line
that `grep -vF '(*)'` keeps. The recorded counts stay comparable across
`bundle-verification`, `bundle-extraction` and `manifest-and-library` because all
three measured through the same filter, and the prosaic claim ("adds six") holds;
the prose should be corrected to `grep -vF '(*)'` or the pipeline restated.

### User attention needed?

Yes, two sentences. First, three sink decisions are daemon-facing and outlive
this ticket: a payload reaches a sink as the decoded body with the envelope
stripped, a host built with a sink answers nothing through the logging
accessors, and `Host::for_instance` is the only way to name an instance on the
raw path. Second, the `no_panic` assertion covers the four hand-off frames only,
narrower than the effort spec's frame list, so the arch spec should record the
measured scope and the per-frame catch as the runtime guarantee.
