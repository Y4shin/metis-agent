---
type: deviation report
title: Deviation report for async-initialisation
status: stable
---

## Deviation report: async-initialisation

Compared: the arch spec's `async-initialisation` section, the ticket doc and its
six acceptance criteria, and the implementation on `ticket/async-initialisation`
(`git diff task/async-initialisation..ticket/async-initialisation`, 1140
insertions over nine files, tip `4cf353f`; the landing branch is `28d22ae`).

### API surface changes

**1. `load_bundle` takes the host as its first parameter.**

- **Planned:** `load_bundle(bundle_path, config, host_context, &TrustStore) ->
  impl Future<Output = Result<Serving, LoadError>>`.
- **Actual:** `plugin_runtime::load::load_bundle(host: &Host, bundle_path: impl
  AsRef<Path>, config: &serde_json::Value, host_context: &serde_json::Value,
  trust: &TrustStore) -> Result<Serving, LoadError>`, an `async fn`.
- **Impact:** every dependent call site in levels 4 to 6 passes a host. That is
  what lets `callback-sink` set the sink before a load and `sample-bundle-ci`
  observe an event through it after one, and it is the only way the
  drop-then-destroy ordering on a failed initialisation can be read, because a
  failed load returns no `Serving` to take a host from. `startup-policy` owns
  building the host it passes. The arch spec's signature line is wrong until the
  reconciler corrects it.

**2. `Serving` carries the registry the entry registered into.**

- **Planned:** the instance name, the `Arc<Instance>` and the descriptor list.
- **Actual:** those three plus `registry() -> &Registry`. The entry builds a
  fresh `Registry::new()` per load and registers the instance into it.
- **Impact:** `teardown-and-release` reaches `Registry::unregister` and
  `enable` through `Serving::registry()`, and a daemon that wants one registry
  for several plugins must register each `Serving` into its own aggregate
  through the same `Registry::register`, which is the one registration path.
  `startup-policy` must decide whether it keeps one registry per plugin or
  aggregates them, and its disabled placeholders need a registry of their own.

**3. `LoadError` is wider than the plan named.**

- **Planned:** a `LoadError` that composes `BundleError`, `StageError` and
  `ManifestError` rather than restating them, and the two setup classes the
  caller can tell apart.
- **Actual:** nine variants: `Unreadable { path, detail }`, `Trust(BundleError)`,
  `Stage(StageError)`, `Manifest(ManifestError)`, `Call(CallError)`,
  `ConfigurationRefused { plugin, status }`, `PluginDefect { plugin, status }`,
  `CapabilityMismatch { capability, detail }` and `Registration(RegistryError)`,
  with `Display`, `Error` and `From` for the three composed types. The two setup
  classes are separate variants rather than one status.
- **Impact:** `startup-policy` classifies by variant: `Trust` and `Unreadable`
  and `Manifest` stop start-up, `Stage`, `Call` (the ABI gate),
  `CapabilityMismatch` and `Registration` disable that plugin, and the two setup
  classes produce a placeholder. `LoadError::Call` carries the landed
  `CallError`, so the ABI refusal keeps its own typed error rather than being
  restated. Anything that matched variants exhaustively against the planned name
  list would not compile.

**4. The configuration and the host context arrive as `&serde_json::Value`, and
the initialisation body is written by hand.**

- **Planned:** the runtime does not own the two value types, and building the
  `{"config","host_context"}` body is this ticket's choice to make.
- **Actual:** both are `&Value`; `initialisation_body` builds the object with
  `json!` from two private constants, `CONFIG_FIELD` and `HOST_CONTEXT_FIELD`,
  which are the only place the two names appear. `plugin-contract`'s
  `authoring` feature stays off, so `Initialisation` and `HostContext` are not
  reachable from this crate at all.
- **Impact:** the daemon, `startup-policy` and `sample-bundle-ci` build the same
  two names and their own value shapes, and `HostContext`'s field set is kept in
  step with contract 18 by hand rather than by the type. That duplication is the
  price of the unchanged dependency posture, and it is now the recorded
  decision rather than an open one.

**5. The capability cross-check compares names only, and the contract says so.**

- **Planned:** "the manifest's capability list against what the library
  reports", refusing with the name of the capability that differed.
- **Actual:** the two declarations are compared as name sets, walking the
  manifest's list first and then the library's, so the first difference in that
  order is the one refused. Kind, version and schemas are not compared, and
  contract 17.9 gained a paragraph stating that a capability's kind and version
  are gated where a call routes instead.
- **Impact:** `sample-bundle-ci`'s hand-written manifest does not have to
  reproduce the library's derived schemas, which is what makes that gate
  reachable at all. A schema drift between the two sides is not caught at load.
  `startup-policy` gains `CapabilityMismatch` in its disable set, and a
  declaration with several differences reports one of them rather than all.

**6. The host-target derivation is private.**

- **Planned:** this ticket owns the derivation, and nothing in the plan asked for
  an exported helper.
- **Actual:** `fn host_target()` is private to `load.rs`, and the test support
  module derives the same string independently.
- **Impact:** level 6's packaging step must re-derive `<os>-<arch>` by hand,
  exactly as the test support does, and a drift between two derivations refuses
  a bundle that carries this host's library. A public helper would be the
  smaller seam, and it is a one-function change whenever someone wants it.

**7. The fixture gained a public constant, `DEFECTIVE_INITIALISATION`.**

- **Planned:** not named. The ticket requires the "plugin is defective" class to
  be reachable, and the landed fixture only ever failed with `invalid_argument`.
- **Actual:** one reserved instance name,
  `plugin_fixture::DEFECTIVE_INITIALISATION`, makes the fixture's own `init`
  fail with `Internal`, so a real bundle can provoke that class. No struct
  literal changes elsewhere, and the ABI harness stays at its 82 checks.
- **Impact:** none outside tests. A later ticket that resolved a configuration
  for an instance with that name gets a failing plugin, which is the point.

**8. The seam has two test binaries and a new shared support module.**

- **Planned:** `crates/plugin-runtime/tests/load.rs`.
- **Actual:** `tests/load.rs` (11 tests), `tests/load_threads.rs` (1 test, its
  own binary so the count is not the harness's own threads), and
  `tests/support/bundle.rs` (218 lines), which builds a signed bundle on disk
  for each case and reuses the landed `support/signer.rs` rather than
  reimplementing the chain. It derives the host target a second time, as a test
  must.
- **Impact:** the no-executor property is measured in the file that can measure
  it, and `sample-bundle-ci` can reuse the builder. Test-only, so no product
  surface moves.

**9. `pub mod load` is the public path.**

- **Planned:** "one asynchronous load entry on `plugin-runtime`", with no module
  named.
- **Actual:** `plugin_runtime::load::{load_bundle, Serving, LoadError}`, declared
  by one added line in `src/lib.rs`.
- **Impact:** dependents import from `plugin_runtime::load`, which is where the
  arch spec's dependents should now be pointed.

### Abstraction usage

- Used/was specified: **yes**. The entry composes the landed surfaces in the
  contract's order and restates none of them: `bundle::verify_bundle`,
  `bundle::extract`, `Manifest::read` and `select_library`, the `unsafe
  Loaded::open` ABI gate, `Loaded::init_plugin` awaited to completion,
  `Instance::capabilities`, and `Registry::register`.
- The one `unsafe` block is exactly where the plan put it, at the mapping step,
  with the justification the plan named: the signature and chain verified the
  bytes and extraction staged them, so the library is code this host decided to
  trust.
- The do-not list held: no runtime and no thread started by the crate (the
  landed `tests/load_threads.rs` measures the thread count across a whole load),
  no second registration path, no restated error types, no schema validation
  changes, and no deadline or cancellation policy, which stays level 4's.
- No dependency changed: `Cargo.toml` and `Cargo.lock` are untouched. The test
  builder uses `zip`, `serde_json` and the fixture, all already in the crate's
  and the dev-dependency set.

### Out-of-scope changes

- `crates/plugin-fixture/src/lib.rs` gained `DEFECTIVE_INITIALISATION` and the
  branch that honours it, described above.
- `docs/plugin-contract.md` section 17.9 gained one paragraph fixing what the
  cross-check compares and what it does not. Contract-visible to a producer, so
  it is the right place, but the spec's level-3 section listed no doc edit.
- `docs/testing.md` gained two command bullets, one per new test file, because
  the repo's protocol rule gives a new test file one.
- `crates/plugin-runtime/src/lib.rs` gained `pub mod load;` and
  `tests/support/mod.rs` gained `pub mod bundle;`.

### Ticket doc update needed?

Yes. The landing note should record: the `load_bundle` signature with the host
parameter and why it is there; `Serving::registry()`; the nine `LoadError`
variants and which set `startup-policy` must map; the name-only cross-check and
contract 17.9's paragraph; the hand-built initialisation body and the two
private field constants; the fixture's `DEFECTIVE_INITIALISATION`; the second
test binary and the shared bundle builder; the private `host_target`; and the
three residuals (a dotted manifest name does not route, `LoadError::Registration`
is unreachable with the in-tree fixture, and criteria 4 and 6 needed no product
code so their RED phase is a compile-time proof and a measurement).

### User attention needed?

Yes, one item, because it outlives this ticket. `Registry::route` recovers the
tool half of a qualified name with a first-dot split, so an instance whose
manifest name carries a dot does not route: `dev.metis.test-plugin.tool.echo`
answers `RouteError::Unknown`. This ticket worked around it by naming its test
chain's instances without a dot, and the workaround is recorded rather than
hidden. It will bite `startup-policy`, whose placeholders are named from the
manifest, and `sample-bundle-ci` if its manifest name carries a dot. The landed
registry already knows the owner from its routes map, so the fix is one line,
and the user's call is whether one of the remaining tickets carries it or it is
filed. Everything else in this report is additive and breaks no dependent.
