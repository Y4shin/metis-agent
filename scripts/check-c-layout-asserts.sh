#!/usr/bin/env bash
# Check the contract's C layout assertions in the C modes the targets reach.
#
# `crates/plugin-contract/c/layout_static_asserts.c` is compiled by that crate's
# build script into `plugin_c_layout` on every build, in whatever mode the
# host's compiler defaults to. That covers this host and no other. This script
# pins the modes a host build cannot, because `cl.exe` compiles `.c` files in
# its legacy C mode unless a `/std:` flag asks for more, and the recorded MSVC
# invocation carries no such flag
# (`docs/bugs/archive/plugin-contract-c-static-assert-msvc.md`).
#
# For each of gcc and clang, and in two modes, this script:
#
#   1. compiles the translation unit at the bar the rest of this repository's C
#      is held to, `-std=c11 -Wall -Wextra -Werror`;
#   2. compiles it in strict ISO C89 with the pedantic diagnostics promoted to
#      errors, which is the strongest bar available here for `cl.exe`'s legacy
#      mode, and which refuses anything newer than C89 wherever it appears;
#   3. compiles a copy with one assertion deliberately broken, which must fail
#      in both modes under both compilers. A check that cannot fail is not a
#      check, and this is what proves that making the file compile traded no
#      layout assertion for compilation.
#
# The generated header is included as a system header, so the strict mode
# judges this translation unit's own constructs: the header carries an
# enumerator trailing comma, which is a C99 feature `cl.exe` accepts and which
# the header generator owns, so it is not this file's to fix.
#
# Usage, from the repository root:
#   bash scripts/check-c-layout-asserts.sh
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_file="$repo_root/crates/plugin-contract/c/layout_static_asserts.c"
include_dir="$repo_root/crates/plugin-contract/include"

# The strict mode is the oldest C the compilers offer, with every pedantic
# diagnostic an error. The current mode is the one the build script pins for
# gcc and clang. Both deny warnings.
strict_flags=(-std=c89 -pedantic-errors -Wall -Wextra -Werror)
current_flags=(-std=c11 -Wall -Wextra -Werror)

# The one assertion this script breaks. It must appear exactly once, and the
# copy is refused below when the substitution changed nothing, so the negative
# half cannot pass by compiling an unchanged file.
intact_condition='sizeof(MetisStatus) == sizeof(uint32_t)'
broken_condition='sizeof(MetisStatus) == sizeof(uint32_t) + 1'

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT
broken_file="$work_dir/layout_static_asserts_broken.c"

sed "s/$intact_condition/$broken_condition/" "$source_file" > "$broken_file"
if cmp -s "$source_file" "$broken_file"; then
  echo "check-c-layout-asserts: the deliberate mismatch did not apply" >&2
  echo "check-c-layout-asserts: expected '$intact_condition' to appear once in $source_file" >&2
  exit 1
fi

for compiler in gcc clang; do
  if ! command -v "$compiler" >/dev/null 2>&1; then
    echo "check-c-layout-asserts: $compiler is not on PATH" >&2
    exit 1
  fi
done

for compiler in gcc clang; do
  for mode in strict current; do
    case "$mode" in
      strict) flags=("${strict_flags[@]}") ;;
      current) flags=("${current_flags[@]}") ;;
    esac

    # The intact translation unit must compile, and the broken copy must not.
    for expectation in accept reject; do
      if [ "$expectation" = accept ]; then
        candidate="$source_file"
      else
        candidate="$broken_file"
      fi

      status=0
      output=$("$compiler" "${flags[@]}" -isystem "$include_dir" -c "$candidate" -o /dev/null 2>&1) || status=$?

      if [ "$expectation" = accept ]; then
        if [ "$status" -ne 0 ]; then
          echo "check-c-layout-asserts: $compiler rejected the layout assertions ($mode mode)" >&2
          echo "$output" >&2
          exit 1
        fi
        echo "check-c-layout-asserts: $compiler accepted the layout assertions ($mode mode)"
      else
        if [ "$status" -eq 0 ]; then
          echo "check-c-layout-asserts: $compiler accepted a deliberately broken layout assertion ($mode mode)" >&2
          echo "check-c-layout-asserts: the check no longer fails the build, so it no longer holds" >&2
          exit 1
        fi
        first_error="$(printf '%s\n' "$output" | grep -m 1 'error' || true)"
        echo "check-c-layout-asserts: $compiler rejected the deliberate mismatch ($mode mode): ${first_error##*/}"
      fi
    done
  done
done

echo "check-c-layout-asserts: the layout assertions hold and still fail the build"
