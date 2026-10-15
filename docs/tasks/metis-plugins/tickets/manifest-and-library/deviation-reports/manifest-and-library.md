---
type: deviation report
title: Deviation report for manifest-and-library
status: stable
---

## Deviation report: manifest-and-library

Compared: `docs/tasks/metis-plugins/arch-spec.md` section
`manifest-and-library` (its Exports, Existing abstractions, Do not reimplement,
Seams and Interface contract, plus the cross-ticket contract block at line 110)
and the effort's level-2 carried-risk note about the ABI minor at line 1019,
against the ticket doc and its six acceptance criteria, and the implementation
on `ticket/manifest-and-library`
(`git diff task/manifest-and-library..ticket/manifest-and-library`): ten files,
six `wip:` commits, tip `e269bda`.

### API surface changes

**1. `Manifest::parse` takes one argument, and the identity check moved into a
new `Manifest::read`.**

- **Planned:** the arch spec contradicts itself. Its cross-ticket contract block
  (line 110) says `Manifest::parse(text, &CapabilityList)`, while its level-2
  Exports say `Manifest::parse(text: &str)`, and the do-not list says the
  capability cross-check "needs a live instance and belongs to the load entry".
- **Actual:** `Manifest::parse(text: &str) -> Result<Manifest, ManifestError>`,
  following the level-2 section and the do-not list. The identity check the
  criterion-1 test needs and the cross-check does not is a separate entry,
  `Manifest::read(staged: &StagedBundle, common_name: &str) -> Result<Manifest,
  ManifestError>`, which tests the path, reads, parses, and compares the
  declared `name` against the chain's `VerifiedBundle::leaf_common_name()`,
  answering `ManifestError::NameMismatch { declared, chain }`.
- **Impact:** dependents call `parse` for text and `read` for a staged bundle,
  and pass the leaf common name from the verification step, because extraction's
  `StagedBundle` does not carry it. `async-initialisation` still owns the
  capability cross-check, exactly as planned.

**2. `Loaded::open` is `unsafe`.**

- **Planned:** `Loaded::open(lib_path, &Manifest)`; the spec fixes the signature
  and is silent on safety.
- **Actual:** `pub unsafe fn open(path: impl AsRef<Path>, manifest: &Manifest)
  -> Result<Loaded, CallError>`, carrying the same obligation as the landed
  `Loaded::load`, because it maps arbitrary native code. It decides the
  manifest's demand first (`manifest.abi_check()?`) and then delegates to
  `Loaded::load`, so the table's major, minor and reported size stay the landed
  `metis_read_table` rules.
- **Impact:** `async-initialisation` needs one `unsafe` block whose stated
  justification is the trust the bundle's signature verification established,
  carried forward by the extraction that staged the file.

**3. The public surface is wider than the named seam, additively.**

- **Planned:** `Manifest` (identity, version, `abi_major`, `min_abi_minor`,
  per-target libraries, capabilities, configuration schema),
  `ManifestError`, plus target selection and the mapping step.
- **Actual:** as planned, with named accessors (`name`, `version`, `abi_major`,
  `min_abi_minor`, `libraries`, `capabilities`, `config_schema`) and three
  entry points beyond the seam's `parse`: `read`, `select_library(&StagedBundle,
  target) -> Result<PathBuf, ManifestError>` and `abi_check() ->
  Result<(), MetisAbiError>`. `ManifestError` carries six variants: `Absent`,
  `Unreadable`, `Invalid`, `NoTargetLibrary`, `LibraryNotStaged`,
  `NameMismatch`.
- **Impact:** the ticket's "no library for this target" refusal is now two
  manifest-side refusals (`NoTargetLibrary`, `LibraryNotStaged`) beside
  extraction's two (`StageError::MissingTargetLibrary`,
  `StageError::AmbiguousTargetLibrary`), so `startup-policy` must classify
  rather than match exhaustively, the same rule that already applies to
  `BundleError` and `StageError`.

**4. The manifest's ABI demand direction follows the parent decision, not the
arch spec's carried-risk sentence.**

- **Planned:** the arch spec's level-2 section (line 661) says the check is the
  manifest's `min_abi_minor` against the landed `METIS_ABI_MINOR` 2, and its
  carried-risk note (line 1019) says "nothing in the landed code refuses a
  bundle at load time for a manifest whose `min_abi_minor` is older than
  `METIS_ABI_MINOR` 2".
- **Actual:** implemented the other way, per parent decision
  `compatibility-policy` C4 and the effort spec's compatibility rule: a manifest
  that requires a minor newer than the build's is refused
  (`MetisAbiError::MinorMismatch { known, found }`), and a manifest that requires
  an older one is served. Both directions are pinned by a test. The carried-risk
  sentence describes a case that cannot refuse anything, since an older
  requirement is legal.
- **Impact:** the carried residual is closed rather than filled as written: the
  manifest's demand is the bundle-load gate on the ABI minor bump, and an older
  *table* is still refused by the landed gate before any entry is read.
  `async-initialisation` and `sample-bundle-ci` write `min_abi_minor <= 2`.

**5. The manifest's field names and types are fixed by this ticket, as the spec
says they are.**

- **Planned:** the spec assigns the exact field names and types to this ticket
  and requires the contract to document them as the format an author writes.
- **Actual:** contract section 17.9 carries the field table, a complete
  hand-written example, the check order, the refusal list and the measured
  dependency count: `name`, `version`, `abi_major`, `min_abi_minor`,
  `[libraries]`, `[[capabilities]]` with `request_schema` and `response_schema`,
  and `[config_schema]`. A test reads the example out of the contract and parses
  it, so the documented format and the read format cannot drift.
- **Impact:** `sample-bundle-ci` writes against 17.9, and the SDK's generator
  must emit exactly these names. 17.9 also states two limits of expressing JSON
  schemas as TOML: a `null` default cannot be written at all, and a TOML
  date-time deserialises into an object carrying `$__toml_private_datetime`.

**6. The mapping step does not call `metis_plugin_table_has_init_plugin`
itself.**

- **Planned:** the arch spec says the mapping step "uses
  `METIS_PLUGIN_TABLE_V1_INIT_PLUGIN_SLOT` and
  `metis_plugin_table_has_init_plugin(size)` in place of the create-instance
  names".
- **Actual:** `Loaded::open` names no slot and no removed entry; the presence
  rule is enforced one layer down by the landed `metis_read_table`, which names
  `init_plugin` and answers `MetisAbiError::EntryMissing { entry: "init_plugin"
  }`. The predicate is exercised at the seam by the criterion-4 test on the
  mapped table.
- **Impact:** none. The check happens, with the landed typed error, and no
  create-instance name survives in the diff.

**7. The dependency figures contract 17.9 records do not reproduce.**

- **Planned:** 17.9 records the pipeline and its result: 84 crates before, 90
  after, six added.
- **Actual, measured here** with the exact pipeline 17.9 names
  (`cargo tree -p plugin-runtime --edges normal --prefix none | grep -v '(*)' |
  sort -u | wc -l`) on the two refs: 91 crates at `task/manifest-and-library`
  (the pre-ticket landing branch, `Cargo.lock` carrying no `toml`) and 97 at the
  ticket tip. The delta of six holds and matches the named crates (`toml`,
  `toml_parser`, `toml_datetime`, `serde_spanned`, `winnow` twice); the absolute
  numbers 17.9 records are seven lower than measured at both ends, and they
  contradict section 17.8's own "91 after" for the extraction ticket, which my
  pre-ticket measurement reproduces.
- **Impact:** documentation only, and it is the second time this effort has
  recorded a dependency number that does not reproduce (17.7's pipeline defect
  was the first). A later reader must not quote 84/90; the correct pair for this
  ticket is 91/97.

### Abstraction usage

- Used/was specified: **yes**. `toml` 0.9 was added once to the workspace table
  with `default-features = false` and only `parse`, `serde` and `std`, and
  referenced as `.workspace = true`. The capability list reuses
  `plugin_contract::capability::CapabilityDescriptor`, so no second descriptor
  type exists. `Loaded::load` and `MetisAbiError` are unchanged and reused for
  the mapping and the version gate. `ManifestError` follows the landed error
  style (`CallError`'s shape: a plain enum of struct variants, `Display`,
  `std::error::Error`), and the tests sit at the named seam,
  `crates/plugin-runtime/tests/manifest.rs` (11 tests), against a staged,
  signed bundle whose manifest is a test literal.
- The do-not list held in full: no second capability descriptor, no schema
  inference, no new ABI rules, and no capability cross-check here.
- The existing raw ABI checks and the ABI harness stay green on a library mapped
  from a bundle, which is criterion 4, and the contract's own tests are
  untouched.

### Out-of-scope changes

- `crates/plugin-runtime/tests/fixtures/patched_table.rs` is a new test-only
  mechanism the plan did not name: a cdylib source carrying `SIZE`, `MAJOR`,
  `MINOR` and `SYMBOL` placeholders that the test substitutes into and compiles
  with `rustc` at test time, because no library in the tree reports a minor, a
  major or a size this build did not write. It makes the four ABI refusals
  reachable at `Loaded::open`, but it adds a `rustc`-on-`PATH` dependency to the
  runtime suite and a source file that deliberately does not compile as it
  stands.
- `crates/plugin-runtime/tests/support/mod.rs` split `fixture_path()` out of
  `fixture()` so a test can read the fixture's bytes and file name. Additive.
- `docs/plugin-contract.md` (section 17.9) and `docs/testing.md` (one test-file
  bullet) are the planned doc edits, and `Cargo.toml`, `Cargo.lock` and the
  crate manifest carry the one new dependency. No other file changed, and
  nothing outside `plugin-runtime`'s source, its tests and the two documents was
  touched.

### Ticket doc update needed?

Yes. The landing step creates `## Implementation notes` and should record: the
seven divergences above; the mechanics (six `wip:` commits, tip `e269bda`,
`docs/tasks/state.yaml` left uncommitted as earlier landings did); the gate
results the tdd result reports; and the residuals, namely that reusing
`MetisAbiError` makes a manifest-side refusal's message say "table",
`select_library` resolves a declared path instead of shape-checking it against
contract 17.8's form (a signed plugin is trusted, so this is a footgun, not an
escalation), the TOML-for-JSON-schema limits, the `rustc`-on-`PATH` test
dependency, and that the patched-table fixture asserts acceptance of a
newer-minor table rather than the fault-level "stops at this build's layout",
which stays at the landed seams.

### User attention needed?

Yes, three sentences.

1. The manifest's ABI demand direction was implemented against the arch spec's
   carried-risk wording, following parent decision `compatibility-policy` C4 and
   the effort spec; the arch spec's sentence describes a case that cannot refuse
   anything, so a reviewer should confirm C4 is the intent before the
   reconciler rewrites that note.
2. The manifest expresses JSON schemas as TOML, so a `null` default is not
   expressible and a TOML date-time arrives as an object with
   `$__toml_private_datetime`; the SDK's generator and the C fixture's
   hand-written manifest must respect both, and moving the schema fields to JSON
   text is the deferred alternative.
3. Contract 17.9's dependency pair (84/90) does not reproduce and contradicts
   17.8; the measured pair is 91/97.
