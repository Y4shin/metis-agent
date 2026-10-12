---
type: findings
title: Plugin ABI mechanics spike
status: stable
---

# Plugin ABI mechanics spike

Question: do the decisions in `../plugin-abi-layout/task.md` (D1 to D13)
survive contact with a compiler and a loader?

Answer: yes, with three exceptions that need the decision text updated (D7, D12,
D13) and one new constraint on the entry point signature (D8). The spike is
conclusive, not abandoned.

Source: `spikes/plugin-abi-mechanics/` (five crates: `spike-abi`,
`spike-payload`, `spike-plugin` (cdylib), `spike-host` (bin),
`spike-header-gen`). Throwaway prototype code, not product code, not a member of
the root workspace, not wired into CI.

## Environment

| Tool | Version |
| --- | --- |
| cargo | 1.101.0-nightly (29c5daa1a 2026-10-07) |
| rustc | 1.101.0-nightly (080cc2714 2026-10-11) |
| cbindgen | 0.29.4 (used as a library from `spike-header-gen`) |
| libloading | 0.9.0 |
| C compiler for `cc` in build.rs | clang 21.1.8 (nix wrapper), GCC 16.2.0 is what `cc` on PATH reports |
| Host | Linux x86_64 |

All commands were run through `devenv shell`. crates.io was reachable.

## What compiled

- `cargo build` builds all five crates. The cdylib exports exactly one dynamic
  symbol, verified with `nm -D --defined-only`:
  `T metis_plugin_entry_v1`, and it is the only dynamic symbol the library
  defines at all. No other `metis_*` or `spike_*` symbol is exported.
- The host resolves that symbol with `libloading`, reads the table, creates an
  instance, lists capabilities, invokes inline and pending, cancels twice,
  triggers a panic, releases every buffer, shuts down, then unloads the library.
  39 runtime checks, all PASS, exit code 0.
- `cargo test` passes 2 tests that compile a C translation unit through the `cc`
  crate and cross-check 16 measured sizes, alignments and offsets, plus drive a
  C-authored version 1 function table from Rust.
- The committed header passes `--check` with no diff.

## What did not compile (and what it cost)

The first generated header did not compile as C at all. cbindgen 0.29.4 renders
`Option<NamedFnAlias>` as an *opaque struct* value, not as a nullable function
pointer:

```
metis_plugin_abi.h:166:33: error: field 'invoke' has incomplete type
  166 |     struct Option_MetisInvokeFn invoke;
```

A second defect appeared once the fields were plain function pointers: because
the `create_instance` typedef mentioned `MetisPluginTableV1` and that struct in
turn depends on the typedef, cbindgen emitted the typedef first and C scoped the
struct tag to the prototype:

```
metis_plugin_abi.h:169:64: warning: 'struct MetisPluginTableV1' declared inside
parameter list will not be visible outside of this definition or declaration
```

Both were fixed *in the Rust source*, never in the header:

1. Table fields are plain (non-nullable) function pointers, so presence is
   expressed by `size` and not by a null pointer. A nullable entry would have to
   be written inline as `Option<unsafe extern "C" fn(...)>`, which cbindgen does
   render correctly (measured: `enum MetisStatus (*inline_option)(int32_t level);`)
   but without the named typedef.
2. `create_instance` no longer takes the plugin table pointer. No entry point
   mentions `MetisPluginTableV1` in its parameters, which removed the dependency
   cycle.

A third, quieter trap: with no `[export] include` list, cbindgen emitted *only
the constants* and none of the types, because no exported function seeded the
dependency graph and the crate exports no `extern "C" fn`. Every ABI type must be
listed in `[export] include`.

After those three fixes the header compiles clean under `cc -std=c11 -Wall
-Wextra` with no diagnostics of its own. The only build warning left comes from
the environment (`_FORTIFY_SOURCE requires compiling with optimization (-O)` in
the nix clang wrapper at debug opt level).

## Results by required check

| # | Check | Result | Evidence |
| --- | --- | --- | --- |
| 1 | Symbol resolution | PASS | `metis_plugin_entry_v1` resolved from the host through `libloading`; `metis_plugin_entry_v2` lookup fails with `dlsym failed`, which is the D8 "differing major fails at resolution" path |
| 2 | Append-only table, host reads its prefix | PASS | plugin reports `size=72`, rustc `size_of` is 72; a 40-byte "older host" prefix agrees with the full table including function addresses; a table claiming `size=8` is refused instead of read; a "newer host" asking for an appended entry (`size=80`) gets `None` from a 72-byte plugin, and gets the entry (and calls it) from an in-process 80-byte table |
| 3 | One-byte payload header | PASS | round trip preserves `0x01` (version nibble 1, codec bit 0, reserved bits 0); bit layout asserted (`0x0f` + codec `0x10` = `0x1f`); reserved bit set (`0x81`) reaches the plugin and returns `invalid_argument`; version 2 returns `invalid_argument`; codec 1 (MessagePack) returns `unsupported` |
| 4 | Synchronous invoke returning inline | PASS | `tool.echo` returns `ok` plus a plugin-allocated buffer, header valid, body `{"echo":"hihi","bytes":4}` |
| 5 | Pending invoke completed through the host callback | PASS | `tool.slow` returns `pending` with a null out buffer; the result arrives on a plugin worker thread through `complete`, the host copies it and releases the buffer; `progress` fired once before the completion |
| 6 | Cancel | PASS | `cancel` of a live request returns `ok`; the plugin suppresses its completion and logs the drop; no completion reaches the host; `cancel` of an unknown id returns `not_found`; a plugin that completes anyway (`tool.cancelrace`) has its completion dropped by the host, buffer still released |
| 7 | Each buffer released by its allocator | PASS | plugin accounting at shutdown: `allocs=308 frees=308 live=0 violations=0`; host: `allocs=260 frees=260`; zero foreign-free attempts and zero rejections. `live=0` also proves no plugin buffer was freed behind the plugin's bookkeeping |
| 8 | Panic converted to an internal-error status | PASS | `tool.panic` panics inside the exported adapter, `catch_unwind` returns `internal`, the panic hook logs through the host `log` callback, and the very next invoke on the same instance succeeds |
| 9 | Generated header layout agreement | PASS | Rust `const` assertions in `crates/abi/src/layout.rs` (sizes, alignments, every offset of four types) compile on every build; the C TU included from `build.rs` static-asserts the same literals and exports measured values that `tests/c_layout.rs` compares with `size_of`/`offset_of`; a C-authored table is then read and driven from Rust |

Extra, not requested but cheap: capability version gating (`tool.future` at
version 99 is refused while 5 capabilities are accepted and the plugin stays
usable), unknown capability returns `not_found`, a double destroy of the instance
handle returns `not_found` instead of a double free, and every host callback is
wrapped in `catch_unwind` (0 host callback panics).

## Inline versus callback-only completion

Measured in the same run, same trivial payload:

- inline: 200 calls in 1.07 ms total, about 5.3 microseconds per call
- pending return: 50 calls in 1.65 ms total, about 32.9 microseconds per call

The pending number is dominated by spawning a thread in the spike plugin, not by
the ABI shape, so treat it as an upper bound on a toy executor rather than a real
cost. What the comparison does show is where the complexity sits: the inline path
costs one null check on the out parameter, while the pending path costs a pending
registry, a condvar wait, a thread hop, and a callback that has to release the
buffer. Keeping both shapes is cheap, but the host's pending registry is the part
that needs real care (see D12 below).

## What went wrong during the spike (harness, not design)

- The first host run appeared to hang and was killed by a 900 s timeout. A trace
  print per check located it immediately: a check passed
  `state.pending.lock().unwrap().is_empty()` and also called
  `state.pending.lock()` inside the same `format!` argument list. `std::Mutex` is
  not reentrant, so the host deadlocked in its own reporting code, after all
  design relevant work had already finished. The checks now print as they run and
  the tail of a run is traceable.
- `cargo run -p spike-host` does not rebuild the cdylib member, so the host
  loaded a stale `.so` and reported results that did not match the plugin source.
  `cargo build` of the whole spike workspace must precede the host run. This is
  exactly the class of problem a real host must avoid, since the plugin it loads
  is a separate artifact.
- The first spike iteration had four real gaps of its own (progress and
  emit_event never implemented, one wrong argument order in a header assertion,
  and an intentional double destroy counted as a violation). All were fixed; none
  of them was a design problem.

## Decisions D1 to D14

- **D1, primitives: no revision.** The set is sufficient. Two additions to
  record: the host table needs a `user_data` pointer (the spike's callbacks
  cannot work without it), and `invoke` taking the host table per call while the
  instance also stores it works and costs nothing.
- **D2, source of truth: no revision, one new CI obligation.** `repr(C)` Rust
  plus a committed generated header works, and the no-diff check
  (`spike-header-gen --check`) is real and cheap. cbindgen needs `[export]
  include` to list every ABI type, because a crate whose ABI surface is a table
  of function pointers exports no functions to seed the dependency graph. Also
  cbindgen 0.29.4 does not translate `Option<NamedFnAlias>`, which is a direct
  constraint on how the structs are authored.
- **D3, trait and translation placement: not exercised.** This spike has no
  authoring trait and no feature gate. Nothing contradicted it.
- **D4, payload envelope: no revision.** Round trip, reserved-bit rule and codec
  bit all behave. One clarification worth adding to the spec: the spike returns
  `unsupported` for codec 1 and `invalid_argument` for a bad version or non-zero
  reserved bits, so the two failure modes are distinguishable. MessagePack was
  never exercised beyond the refusal.
- **D5, compatibility across compiled versions: no revision.** Per-capability
  version gating worked; the host refused one capability and continued.
- **D6, ownership and release: no revision, but one case needs stating.** A
  completion that the receiver discards still carries a plugin-allocated buffer
  that must go back through `release_buffer`. The spike measured exactly that: a
  late completion dropped because of a successful cancel still produced one
  release. "Each side releases what it allocated" should say "including when the
  payload is ignored".
- **D7, errors and panics: needs revision.** Two points.
  1. The frozen status list has no member for D11's pending outcome, so `invoke`
     cannot report "the result will arrive through `complete`". The spike added
     `Pending = 7`. The alternatives are a separate out parameter carrying a
     completion shape (`inline` or `pending`), which keeps the seven member
     status space intact, or overloading `ok` with a null out buffer, which makes
     "ok with no result" ambiguous and should be rejected.
  2. `catch_unwind` is required on both sides of the boundary, not only in
     exported adapters. A host callback is itself an `extern "C"` frame reached
     from plugin code, so a panic inside it unwinds into plugin frames. The spike
     wraps every host callback as well, and counts zero callback panics.
- **D8, versioning mechanism: no revision, plus one new constraint.** Both
  directions were proven: a newer host reads only the 40-byte prefix it knows and
  does not touch an absent appended entry, while an 80-byte table lets it find and
  call the appended entry. Constraints that fall out of the layout: `size` must be
  the first field so it is readable before anything else; the fields a host knows
  must form a prefix of the layout, so an entry can only ever be appended; and
  the entry point signature itself must be frozen inside a major version, because
  the symbol name carries only the major. The spike used an argument-free entry
  point for exactly that reason.
- **D9, capability model: no revision.** Descriptors with name, kind, version and
  schema reference were enough, and `invoke` by name worked.
- **D10, payload version versus capability version: no revision.** Both versions
  were exercised independently.
- **D11, async shape: no revision.** Both completion shapes work. Pending state
  must be registered before the plugin is called (the host did, and the plugin's
  completion for `tool.slow` found it), and the host needs a wait with a timeout:
  if a completion never arrives, an unbounded wait hangs, which is precisely what
  the harness bug looked like from the outside.
- **D12, callback rules: needs revision.** Two gaps.
  1. After a successful `cancel`, the host must also retire its own pending
     entry. No completion is guaranteed, so if the host only ignores late
     callbacks it leaks a pending record per cancelled request. The spike's first
     full run had exactly one leaked entry, from the cancelled request whose
     completion the plugin suppressed.
  2. The rules should state both halves of the drop path: the plugin may suppress
     its completion after a successful cancel (measured, logged), and the host
     must drop a completion that arrives anyway (measured with a capability that
     ignores cancellation). Only the second half is safe on its own, because the
     cancel and the completion race.
- **D13, header and layout verification: no revision, with a clarification.** A
  committed generated header plus Rust `const` assertions plus a C TU that
  static-asserts the same numbers all work, and the C TU also proves the header is
  usable by a C implementer (it builds a version 1 table with designated
  initialisers and Rust then reads and calls it). Two notes: the literal numbers
  are 64-bit specific, so either the assertions must be written in terms of
  `size_of::<usize>()` or the check must run per target; and compiling the C side
  needs at least `-std=c11` for `_Static_assert`. The spike goes one step further
  than the decision by exporting the C-side measurements (`SPIKE_*` symbols) and
  comparing them with `size_of`/`offset_of` in Rust, so a stale literal is caught
  as well as a layout drift.
- **D14, validation by prototype: no revision.** This spike is conclusive; the
  strategy does not need to be abandoned.

## Reproduction

From the repository root, with `devenv` on PATH:

```sh
# 1. build everything (MUST precede the host run: `cargo run -p` does not
#    rebuild the cdylib member, and the host would load a stale .so)
devenv shell -- cargo build --manifest-path spikes/plugin-abi-mechanics/Cargo.toml

# 2. runtime checks: prints a PASS/FAIL line per check, exits non-zero on failure
devenv shell -- cargo run -q --manifest-path spikes/plugin-abi-mechanics/Cargo.toml -p spike-host

# 3. layout agreement: Rust const assertions, the C TU through `cc`, and the
#    Rust test that compares both sides
devenv shell -- cargo test --manifest-path spikes/plugin-abi-mechanics/Cargo.toml

# 4. regenerate the committed header, then check that it matches
devenv shell -- cargo run -q --manifest-path spikes/plugin-abi-mechanics/Cargo.toml -p spike-header-gen
devenv shell -- cargo run -q --manifest-path spikes/plugin-abi-mechanics/Cargo.toml -p spike-header-gen -- --check

# 5. what the cdylib actually exports
devenv shell -- nm -D --defined-only spikes/plugin-abi-mechanics/target/debug/libspike_plugin.so
```

Useful switches: `SPIKE_PLUGIN_PATH` points the host at a different `.so`;
`SPIKE_SHOW_PANIC_BACKTRACE=1` lets the default panic hook print during the panic
check (the plugin otherwise forwards the panic through the host `log` callback).

## Remaining uncertainty

- Everything here is Linux x86_64. The 32-bit sizes differ (tables shrink to 40
  and 32 bytes), so the literal assertions would fail there, and none of the
  Windows questions in D6 (CRT, allocator boundaries, `cdylib` loading) were
  touched at all.
- Library unloading is safe only after `shutdown` has joined every plugin worker
  thread. The spike does join before `drop(library)` and the unload succeeds, but
  the general rule for a plugin that leaks a thread is untested, and a real host
  has to treat "unload while callbacks may still arrive" as a hazard.
- One instance, one plugin, and a single-threaded host apart from plugin workers.
  Concurrent invokes, multiple instances, and reentrancy were not exercised.
- Callback ordering (progress before complete) was observed but is not specified
  anywhere; the spike relies on the plugin's own thread, not on a contract.
- The timing comparison is not a real executor measurement, see above.
- The C side of the layout check is a minimal implementer (no payload handling,
  no release path), so the header is proven usable, not sufficient.
- The spike did not test what happens if a plugin returns a buffer whose `owner`
  tag lies (host would call `release_buffer`, plugin would reject it, and the
  leak would be detected by the `live` counter only at shutdown).
- The handwritten-header alternative was not built as a separate artifact. The
  evidence gathered is indirect but relevant: the generated header needed two
  structural fixes, and both were made in Rust, which is an argument for the
  generated approach; a handwritten header would have to be kept in step with
  these same three authoring rules by hand.

## Verdict

Conclusive. The ABI mechanics work as decided, with the D7, D12 and D13 wording
changes above and the entry point signature constraint added to D8. Nothing was
found that argues against `repr(C)` Rust as the source of truth with a generated
header.
