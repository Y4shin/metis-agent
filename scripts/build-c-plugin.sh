#!/usr/bin/env bash
# Compile the C implementers of the plugin table with both C compilers.
#
# The C plugin is the proof that the committed header is enough for a non-Rust
# author, so it is held to the same bar a C project would hold itself to:
# `-std=c11 -Wall -Wextra -Werror`, under gcc and under clang. Both compilers
# must accept it, and the library the harness loads is the gcc one, so the
# header is exercised by a compiler and then by a real load.
#
# The second artifact is the other-major plugin: a library whose only exported
# symbol is `metis_plugin_entry_v2`, which is what the harness resolves the v1
# entry symbol against to require a typed refusal. It is compiled once, by gcc,
# because the header itself has already been through both compilers.
#
# Usage, from the repository root:
#   bash scripts/build-c-plugin.sh
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_file="$repo_root/crates/plugin-fixture/c/c-plugin.c"
other_major_file="$repo_root/crates/plugin-fixture/c/other-major-plugin.c"
include_dir="$repo_root/crates/plugin-contract/include"
out_dir="$repo_root/target/c-plugin"

mkdir -p "$out_dir"

for compiler in gcc clang; do
  if ! command -v "$compiler" >/dev/null 2>&1; then
    echo "build-c-plugin: $compiler is not on PATH" >&2
    exit 1
  fi
  "$compiler" -std=c11 -Wall -Wextra -Werror -fPIC -shared \
    -I"$include_dir" \
    -o "$out_dir/libcplugin-$compiler.so" \
    "$source_file"
  echo "build-c-plugin: $compiler accepted the committed header"
done

# The harness loads this one, so the gcc build is the driven artifact and the
# clang build is the second opinion.
cp "$out_dir/libcplugin-gcc.so" "$out_dir/libcplugin.so"
echo "build-c-plugin: wrote $out_dir/libcplugin.so"

# The other-major plugin: one symbol, `metis_plugin_entry_v2`, and no table a
# host of this major can reach.
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
  -I"$include_dir" \
  -o "$out_dir/libother-major.so" \
  "$other_major_file"
echo "build-c-plugin: wrote $out_dir/libother-major.so"
