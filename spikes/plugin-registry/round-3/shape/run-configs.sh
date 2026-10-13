#!/usr/bin/env bash
# Lane shape: run the four configurations and record counts and exit codes.
# Not product code; a throwaway harness log generator. Lives inside the copy.
set -u
cd /home/patric/Projects/metis-agent
COPY=/tmp/metis-plugin-registry-round-3/shape
OUT=$COPY/run-log.txt
: > "$OUT"

run() {
  local label="$1" profile="$2" features="$3"
  local cargo_args=(build --manifest-path "$COPY/Cargo.toml")
  [ "$profile" = release ] && cargo_args+=(--release)
  cargo_args+=(-p proto-plugin-cdylib -p proto-host)
  [ "$features" = off ] && cargo_args+=(--no-default-features)
  echo "### $label: cargo ${cargo_args[*]}" | tee -a "$OUT"
  devenv shell -- cargo "${cargo_args[@]}" >>"$OUT" 2>&1
  local build_rc=$?
  local bin="$COPY/target/$profile/proto-host"
  "$bin" >"$COPY/run-$label.txt" 2>&1
  local run_rc=$?
  local summary
  summary=$(tail -1 "$COPY/run-$label.txt")
  echo "$label build_rc=$build_rc run_rc=$run_rc :: $summary" | tee -a "$OUT"
}

run on-debug        debug   on
run on-release      release on
run off-debug       debug   off
run off-release     release off
