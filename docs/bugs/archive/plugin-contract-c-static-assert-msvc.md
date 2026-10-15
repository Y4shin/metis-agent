---
type: bug
title: plugin-contract's C layout checks do not compile under MSVC
status: fixed
fix_commit: e1099f5
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

## Root cause

Confirmed by measurement, and narrower than "MSVC cannot compile C11":

`cl.exe` compiles `.c` files in its legacy C mode unless a `/std:` flag asks for
more, and that invocation carries no `/std:` flag. The file used the C11
`_Static_assert(expr, "message")` message form and the C11 `_Alignof` operator,
which that mode does not have, so MSVC parsed each assertion as a declaration and
reported `C2059: syntax error: ')'` with a `C4255` prototype warning on every
assertion, which is what the CI log shows line by line. The build script did ask
for C11, but through `cc`'s `flag_if_supported("-std=c11")`, whose dash spelling
`cl.exe` does not take, so the probe dropped it and MSVC stayed in its legacy
mode. The same invocation's `-Wall` is a gcc spelling that `cl.exe` reads as its
all-warnings level, which is not what gcc's `-Wall` means and is not a level to
deny warnings at.

The two candidate fixes the report named were both real. The flag one cannot be
checked from this repository's Linux host at all, so the form fix was taken, and
the flag spelling was removed rather than repaired.

## Fix summary

`c-static-assert-msvc` landed the fix in `plugin-contract`. No ABI type, header
byte, slot or version changed, so nothing downstream of the layout moved:

- Every layout assertion is now written with `METIS_C_LAYOUT_ASSERT(condition,
  name)`, a typedef of a `char` array whose length is `condition ? 1 : -1`. A
  false condition is a negative array length, which gcc, clang and `cl.exe` all
  reject as an error rather than as a warning, and the array's name carries the
  check into the compiler's diagnostic, which is what the old string message did.
  The form is valid C89 onwards, so the file no longer depends on any compiler's
  default C standard, and the check cannot be skipped by asking for an older one.
- `_Alignof` became `__alignof`, the spelling gcc, clang and `cl.exe` all accept
  in every mode. The exported `metis_c_align_*` measurements use it too, and
  `tests/c_layout.rs` still compares them with Rust's `align_of`.
- `crates/plugin-contract/build.rs` pins `-std=c11` for gcc and clang as before,
  denies warnings on the MSVC invocation (`-WX` on the `-W4` that `cc` asks for by
  default), and no longer passes `-Wall` and `-Wextra` through
  `flag_if_supported`, which reached `cl.exe` as its all-warnings level. The
  denial is scoped rather than blanket because at `-O0` this host's glibc
  promotes its own fortify warning to an error under `-Werror`, which would fail
  every debug build here; gcc and clang are held to a warnings-denied compile by
  the new script instead.
- `scripts/check-c-layout-asserts.sh` is new, and is command 8 of
  `docs/testing.md`. For each of gcc and clang it compiles the translation unit
  with warnings denied in `-std=c11` and in strict ISO C89 (`-std=c89
  -pedantic-errors`, the strongest bar available here for `cl.exe`'s legacy
  mode), and it compiles a copy with one assertion deliberately broken, which
  must fail in all four compiles. It refuses to run when the deliberate
  substitution does not apply, so the negative half cannot pass by compiling an
  unchanged file.
- The `windows` CI job now runs `cargo +nightly check -p api-contracts -p daemon
  --all-targets`, so MSVC compiles the daemon and the whole plugin line for the
  first time.
- `docs/testing.md` carries the new command, the widened CI job, the narrowed
  gap wording, and the re-measured local reproduction.

Measured on this repository's Linux host when the record was written:
`bash scripts/check-c-layout-asserts.sh` accepts the unit under gcc and clang in
both modes and rejects the deliberate mismatch under both (`size of array
'metis_c_layout_assert_MetisStatus_size' is negative`), `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings` and `cargo test` are clean,
`bash scripts/build-c-plugin.sh` and `bash
scripts/regenerate-abi-header.sh --check` are clean, and `cargo run -p
plugin-fixture --bin abi-harness` prints `[result] PASS (82 checks)`.

The fix landed on the working branch `ticket/c-static-assert-msvc`. The
frontmatter's `fix_commit` names that branch's tip when this record was written;
`finalize-task` records the landing branch's own SHA (`git rev-parse
task/c-static-assert-msvc`) in its place, per its bug-closure step.

## Impact

Before the fix, on Windows: any build of a crate that depends on
`plugin-contract` failed in its build script, before any Rust code was
type-checked. `daemon` and `plugin-runtime` therefore could not be compiled for
the Windows target, which is why the `windows` CI job checked `api-contracts`
alone. Nothing on Linux, macOS or in the ABI gates was affected: the header and
the Rust side were never in question.

After the fix, on Windows: the translation unit compiles in the legacy C mode
`cl.exe` defaults to, with warnings denied, and a layout mismatch still fails the
build there because a negative array length is an error. Under gcc and clang the
same file compiles in strict ISO C89 and in C11, and the deliberately broken copy
fails under both, so the layout check was not traded for compilation. The
`windows` job compiles the daemon.

## Closing check

Run the pinned check and the C gates this fix added or touched:

```
devenv shell -- bash scripts/check-c-layout-asserts.sh
devenv shell -- bash scripts/build-c-plugin.sh
devenv shell -- bash scripts/regenerate-abi-header.sh --check
devenv shell -- cargo test -p plugin-contract --test c_layout
```

And the check that only a Windows host can carry, because it is the one place
MSVC compiles this file. It is also the `windows` CI job's own command:

```
devenv shell -- cargo +nightly check -p api-contracts -p daemon --all-targets
```

On this repository's Linux host that command does not reach the file: the
nix-wrapped clang refuses the Windows target itself (`clang: error: unsupported
option '-fPIC' for target 'x86_64-pc-windows-msvc'`), which it reports for
`crates/plugin-contract`'s C file and for `libsqlite3-sys`'s vendored
`sqlite3.c` alike. That is a property of this host, recorded in
`docs/testing.md`, and not of the repository.
