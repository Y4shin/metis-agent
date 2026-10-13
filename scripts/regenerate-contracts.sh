#!/usr/bin/env bash
# Regenerate the committed Rust for the wire contract, or check that the
# committed trees still match the contract.
#
# Both codegen plugins are BSR remote plugins pinned by tag in the root
# `buf.gen.yaml`, so generation needs `buf` and nothing else: no local plugin
# binary, no `protoc`, no JVM. The committed trees are the plugin output after
# the rustfmt post-step below. That is why the everyday build needs neither
# `buf` nor a generator: the generated Rust is already in the tree.
#
# The module and the template both live at the repository root, so every buf
# command runs from there and reads one configuration.
#
# Usage, from the repository root:
#   bash scripts/regenerate-contracts.sh           # regenerate in place
#   bash scripts/regenerate-contracts.sh --check   # fail when the trees drift
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
generated_dir="$repo_root/crates/api-contracts/generated"
cd "$repo_root"

# Format one generated tree in place.
#
# The plugin output is not rustfmt-shaped, so the post-step runs rustfmt with
# the repository config (`max_width = 100`, edition 2024); `cargo fmt` never
# sees these files, because they are `include!`d rather than declared as
# modules. `normalize_comments` is added on top of the repository config: buffa
# emits multi-line `/** */` doc comments, and rustfmt's re-indenting of their
# prose turns it into an indented Markdown code block, which rustdoc then
# collects as a doctest that cannot compile. Normalizing those block comments to
# `///` lines keeps the generated tree inside `cargo test`.
format_tree() {
  local tree="$1"
  local file
  for file in "$tree"/*.rs; do
    rustfmt --edition 2024 \
      --config-path "$repo_root/rustfmt.toml" \
      --config normalize_comments=true \
      "$file"
  done
}

if [ "${1:-}" = "--check" ]; then
  scratch="$(mktemp -d)"
  trap 'rm -rf "$scratch"' EXIT

  # The same template, pointed at the scratch tree, so a drift check never
  # writes to the committed files.
  sed "s#crates/api-contracts/generated#$scratch#" \
    "$repo_root/buf.gen.yaml" > "$scratch/buf.gen.yaml"

  buf generate --template "$scratch/buf.gen.yaml"
  format_tree "$scratch/buffa"
  format_tree "$scratch/connect"

  status=0
  for tree in buffa connect; do
    if ! diff -ru "$generated_dir/$tree" "$scratch/$tree"; then
      echo "drift check: generated/$tree does not match the contract" >&2
      status=1
    fi
  done
  if [ "$status" -ne 0 ]; then
    echo "drift check: run 'bash scripts/regenerate-contracts.sh' and commit the result" >&2
    exit 1
  fi
  echo "drift check: generated trees match the contract"
  exit 0
fi

buf generate
format_tree "$generated_dir/buffa"
format_tree "$generated_dir/connect"
echo "regenerated crates/api-contracts/generated"
