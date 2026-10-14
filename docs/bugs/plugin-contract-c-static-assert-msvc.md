---
type: bug
title: plugin-contract's C layout checks do not compile under MSVC
status: open
---

# `plugin-contract`'s C layout checks do not compile under MSVC C mode

## Symptom

Any Windows build of a crate that depends on `plugin-contract` fails in its build
script, before any Rust code is type-checked. Found by the `windows` CI job that
`windows-named-pipe` added: the first run of that job failed in `-p daemon`, not in
the Windows transport, with `error C2059`, `error C2143` and finally
`fatal error C1003: error count exceeds 100` from `c/layout_static_asserts.c`.

The exact command cc-rs ran (from the CI log, MSVC 14.51):

```
cl.exe -nologo -MD -Z7 -Brepro -I include -W4 -Wall -Fo<out>\a1edd97dd51cd48d-layout_static_asserts.o -c c/layout_static_asserts.c
```

## Root cause (measured, not yet confirmed against the compiler docs)

`cl.exe` compiles `.c` files as C89 unless `/std:c11` or later is requested, and
the file uses `_Static_assert(expr, "message")`, whose message form is a C11
feature. MSVC then parses `_Static_assert` as a function declaration and reports
`C2059: syntax error: ')'` plus the `C4255` prototype warning on every assertion,
which is what the CI log shows line by line.

Two candidate fixes, neither applied here because the effort it came from is
archived and this file is the plugin line's:

1. Pass a standard flag from `plugin-contract`'s build script, for example
   `.flag_if_supported("/std:c11")` on MSVC, so the same source compiles
   everywhere.
2. Rewrite the assertions to the MSVC-agnostic form (`static_assert` via
   `<assert.h>` in C11 mode, or a negative-array-size macro), which avoids
   depending on the compiler's default C standard.

The header and the Rust side are unaffected: `scripts/regenerate-abi-header.sh
--check` and the ABI harness stay green on Linux, so the layout claims still hold
where they are checked today.

## Impact

- `daemon` cannot be compiled for Windows, so its Windows readiness is unproven.
  That is why the `windows` CI job checks only `api-contracts`.
- `plugin-runtime`, and anything else that reaches `plugin-contract`, has the same
  block on Windows.
- Nothing on Linux, macOS or in the ABI gates is affected.

## Closing check

Run the `windows` CI job with `-p daemon --all-targets` restored, or locally on a
Windows host (or on Linux with the rustup nightly and the Windows target, where
the nix-wrapped clang fails for the same file for a different reason):

```
cargo +nightly check -p daemon --all-targets
```

It passes when the C file compiles under MSVC's default C mode; then the `windows`
job's comment and `docs/testing.md`'s CI bullet drop the exclusion.
