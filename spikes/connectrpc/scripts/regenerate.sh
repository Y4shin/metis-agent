#!/usr/bin/env bash
# Regenerate the ConnectRPC spike's checked-in generated code, or verify it is
# up to date.
#
# The plugins are BSR remote plugins, so the only tool needed is buf itself,
# obtained unmodified through nixpkgs. There is no local plugin install.
set -euo pipefail
cd "$(dirname "$0")/.."

check=0
if [ "${1:-}" = "--check" ]; then
  check=1
fi

if [ "$check" = 1 ]; then
  baseline="$(mktemp -d)"
  trap 'rm -rf "$baseline"' EXIT
  cp -r generated "$baseline/generated"

  nix shell nixpkgs#buf --command buf generate

  if diff -ru "$baseline/generated" generated; then
    echo "drift check: no diff"
  else
    echo "drift check: generated/ is stale" >&2
    exit 1
  fi
else
  nix shell nixpkgs#buf --command buf generate
fi
