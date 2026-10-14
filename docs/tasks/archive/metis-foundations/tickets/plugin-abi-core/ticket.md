---
type: ticket
subtype: feature
title: Plugin ABI primitives proven by a fixture plugin
status: deprecated
workflow_state: done
blocked_by: []
size: m
---

## What to build

The plugin boundary itself: a small primitive ABI, a generated header for other
languages, and a fixture plugin that a host loads and drives, so that the plugin
host and SDK efforts have a real boundary to build on.

## Acceptance criteria

- [ ] A plugin shared library exports exactly one versioned entry symbol
      returning an append-only function table whose first field is its size, and
      the table covers instance lifecycle, capability listing, one generic
      invoke, cancel, buffer release, and shutdown.
- [ ] The host table carries a user data pointer and the complete, progress,
      emit-event, and log callbacks.
- [ ] The status space includes a pending member, and the payload envelope
      implements the one-byte format header: a codec this build does not know
      returns unsupported, while a bad format version or non-zero reserved bits
      return invalid-argument.
- [ ] Each side releases only what it allocated, including a payload the receiver
      decides to drop, and the host never frees plugin memory.
- [ ] A panic in an exported function and a panic in a host callback each become
      an internal error rather than unwinding across the boundary.
- [ ] A generated C header is committed, Rust asserts sizes, alignment, and
      offsets in pointer-size terms, and a C translation unit compiled in CI
      re-asserts the same numbers.
- [ ] A fixture plugin is loaded by a harness that prints a pass or fail line per
      check and exits non-zero on failure, and the fixture is built before the
      harness runs so a stale library is never loaded.

## Blocked by

- None (can start immediately).

## Implementation notes

Landed on `task/plugin-abi-core` as the merge commit `8ff64fa`
("ticket(plugin-abi-core): Plugin ABI primitives proven by a fixture plugin"),
parents `ce07875` (the landing branch, carrying the landed `contract-pipeline`)
and `5e122d2` (working branch `ticket/plugin-abi-core`). The merge was clean with
no conflicts, the landing tree is byte-identical to `5e122d2`, and
`ticket/plugin-abi-core` was deleted after the merge.

At landing, on the landing branch: `cargo test -p plugin-contract` passes (7
payload tests, 2 C layout cross-checks), the ABI harness prints
`[result] PASS (41 checks)` with exit 0, and
`scripts/regenerate-abi-header.sh --check` reports that the header matches the
crate. The deviations below come from the review artifact
`deviation-reports/plugin-abi-core.md` (committed with this note); the
per-criterion evidence and residual risks live in the implementation and verify
reports.

1. `devenv.nix` uses `pkgs.rust-cbindgen`, not `pkgs.cbindgen`. The pinned
   nixpkgs has no `cbindgen` attribute; the package was renamed, and it installs
   the `cbindgen` binary at 0.29.4, the version the mechanics spike measured.
   The arch spec still names the nonexistent attribute and should be corrected.
2. `cbindgen.toml` excludes the six `crate::payload` constants by name, because
   cbindgen emits every public constant of the crate regardless of
   `[export] include`. Any new public constant in `plugin-contract` must be
   excluded too, or it lands in the header that is meant to be `abi` only.
3. The fixture crate is `cdylib` plus `rlib`; the `rlib` exists only so the
   sibling harness binary can share the payload shapes instead of restating the
   JSON. The loaded `cdylib` is unchanged and still exports exactly one
   `metis_*` symbol.
4. The fixture advertises four capabilities (`tool.echo`, `tool.pending`,
   `tool.cancellable`, `tool.panic`). The late-completion-after-cancel path is
   `tool.pending` itself, which never honours a cancel, so no separate
   `tool.late` name exists.
5. The panic-in-a-host-callback probe is harness-owned, because unwinding out of
   a Rust `extern "C"` callback frame aborts. Every host must wrap its callbacks
   the same way; this is a real obligation on `plugin-runtime`, not a fixture
   detail.
6. A per-capability version gate check was written and then removed before
   landing, because the fixture advertises no version a host would refuse, so
   the check could not fail. Per-capability gating belongs to
   `plugin-compatibility`.
7. At landing the harness reported 41 checks, `ownership.plugin_ledger_balanced`
   `allocs=10 frees=10 live=0`, `ownership.ledger_agrees_with_harness` 10 plugin
   allocations against 10 harness releases, and
   `ownership.no_foreign_or_rejected_releases`.
8. Residual risks: the C side asserts that function pointers and object pointers
   share size and alignment on the supported targets, so an exotic target fails
   loudly rather than silently; Windows `cdylib` loading, CRT and allocator
   boundaries are untouched; the harness needs `nm` from the devenv shell for
   the symbol check; the pending paths rely on 120 ms / 300 ms worker delays
   against 5 s waits.
