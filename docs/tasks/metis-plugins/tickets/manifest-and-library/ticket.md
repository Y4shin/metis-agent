---
type: ticket
subtype: feature
title: Validate the manifest and select this host's library
status: stable
workflow_state: ready
blocked_by: [abi-initialisation-entry, bundle-extraction]
size: m
---

## What to build

A verified, extracted bundle becomes a mapped library with a validated table. The
manifest is parsed and validated: identity, the plugin's own version, the ABI
major and the minimum minor it needs, the library path per target, the capability
list with its schemas, and the configuration schema. This host's target library is
selected and mapped, the entry symbol is resolved against the ABI major, and the
table is checked under the landed rules. Refusals are typed and distinguishable:
absent manifest, invalid manifest, no library for this target, missing entry
symbol, major mismatch, an older minor refused before any entry is read, and a
table too short for an entry this build needs.

## Acceptance criteria

- [ ] An absent manifest and an invalid manifest are different typed errors, and
      the invalid one names what is wrong.
- [ ] Target selection chooses this host's library and refuses a bundle that does
      not carry it, without touching another target's file.
- [ ] The ABI checks reuse the landed rules and their existing typed error, and a
      newer minor's table is read while stopping at this build's layout.
- [ ] A library mapped from a bundle yields a table the existing raw ABI checks
      accept.
- [ ] A hand-written manifest loads, so the format is not something only a
      generator can produce.
- [ ] The manifest's fields are documented in the contract as the format an author
      writes.

## Blocked by

- `abi-initialisation-entry`
- `bundle-extraction`

## Implementation notes

Landed on `task/manifest-and-library` as the merge commit `fa95fe9`
("ticket(manifest-and-library): Validate the manifest and select this host's
library"), parents `51879e8` (the landing branch tip, `docs(ticket): land
c-static-assert-msvc`) and `e269bda` (the working branch, `wip:
manifest-and-library the manifest's test fixture builds once and lands
atomically`). The merge was clean with no conflicts, and `git diff e269bda HEAD`
is empty, so the landed tree is byte-identical to the verified tip. The branch
`ticket/manifest-and-library` was deleted after the merge, and no source, test or
config file was changed by the landing step. `docs/tasks/state.yaml` is left
uncommitted, matching the earlier landings in this effort, because
`finalize-task` owns the workflow state.

The implementation is 10 files, 1286 insertions and 4 deletions against
`51879e8`, over six `wip:` commits. `plugin_runtime::manifest` parses the TOML an
author writes (`name`, `version`, `abi_major`, `min_abi_minor`, `[libraries]`,
`[[capabilities]]` with `request_schema` and `response_schema`, and
`[config_schema]`), reads it from a staged bundle and compares the declared name
against the chain's leaf common name, selects this host's library, and answers
the manifest's own ABI demand; `Loaded::open` decides that demand before it
opens the library and then delegates to the unchanged `Loaded::load`, so the
table's major, minor and reported size stay the landed `metis_read_table` rules
and the landed `MetisAbiError`. The public `Manifest` surface is proven by 11
tests in `crates/plugin-runtime/tests/manifest.rs`, one per acceptance criterion
plus the supporting cases, and the contract's section 17.9 documents the
manifest as the format an author writes, with a test that reads the documented
example back out of `docs/plugin-contract.md` and parses it, so the documented
and read formats cannot drift.

The verifier ran against `e269bda`, inside the devenv shell: `cargo fmt --check`
clean, `cargo clippy --all-targets -- -D warnings` clean (including a re-run
after touching the changed sources), `cargo test -p plugin-runtime --test
manifest` 11 passed with 0 failed, and the full `cargo test` green across 49 test
targets with 0 failures. The tdd worker's wider run was also green
(`cargo test -p plugin-contract --features authoring`,
`scripts/build-c-plugin.sh`, the ABI harness at `[result] PASS (82 checks)`,
`scripts/regenerate-abi-header.sh --check`, `scripts/check-c-layout-asserts.sh`,
`cargo check -p plugin-contract --no-default-features`). Because the landed tree
is byte-identical to the verified tip, those results carry to the landing branch
unchanged; the landing step did not re-run them and did not change any code.

The deviation report lives at
`docs/tasks/metis-plugins/tickets/manifest-and-library/deviation-reports/manifest-and-library.md`
and records seven divergences from the arch spec. The consequential ones, also
summarised in the tdd result: the manifest's ABI demand direction was implemented
per parent decision `compatibility-policy` C4 (a required minor newer than this
build's is refused) rather than the arch spec's carried-risk sentence, which
describes a case that cannot refuse anything; `Manifest::parse` takes only the
text, with the identity check in the new `Manifest::read` because the
capability cross-check belongs to `async-initialisation`; `Loaded::open` is
`unsafe` and carries the same obligation as `Loaded::load`; and the mapping step
relies on the landed `metis_read_table` for the `init_plugin` presence rule
instead of naming a slot itself.

Three items are handed to `finalize-task` and to later tickets, not done here.
First, the deviation report's user-attention section asks a reviewer to confirm
that `compatibility-policy` C4 is the intended direction before the reconciler
rewrites the arch spec's carried-risk note. Second, contract section 17.9's
dependency pair (84 before, 90 after) does not reproduce: the deviation report
measured 91 and 97 with the pipeline 17.9 names, and the delta of six matches the
named crates, so a later reader must not quote 84/90. Third, the manifest
expresses JSON schemas as TOML, so a `null` default is not expressible and a TOML
date-time arrives as an object carrying `$__toml_private_datetime`; both limits
are stated in 17.9, and the SDK's generator and the sample bundle's hand-written
manifest must respect them.

Residual risk carried forward, unchanged from the verified tip: reusing the
landed `MetisAbiError` for the manifest's demand makes a manifest-side refusal's
message say "table"; `Manifest::select_library` resolves the declared path
instead of shape-checking it against contract 17.8, so a signed manifest
carrying an absolute or `..` path can name a library outside the staging
directory (a footgun for a trusted author, not an escalation); the runtime test
suite now depends on a `rustc` on `PATH`, because
`tests/fixtures/patched_table.rs` is compiled at test time; and the
patched-table fixture asserts acceptance of a newer-minor table rather than the
fault-level "stops at this build's layout", which stays measured at the landed
seams (`plugin-contract/tests/abi.rs` and the ABI harness).
