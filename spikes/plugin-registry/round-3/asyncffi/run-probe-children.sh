#!/usr/bin/env bash
# PROTOTYPE (throwaway). Lane async-ffi: run each probe child directly, on the
# WITH-async-ffi cdylib, and record its exit status, stdout and stderr verbatim.
# The harness runs these itself; this script exists so the raw evidence is on
# disk next to the findings.
# Run from this copy's root, inside the devenv shell:
#   cd /home/patric/Projects/metis-agent
#   devenv shell -- bash /tmp/metis-plugin-registry-round-3/asyncffi/run-probe-children.sh
set -uo pipefail
HOST="$(cd "$(dirname "$0")" && pwd)/target/release/proto-host"
cd "$(dirname "$0")" || exit 1
mkdir -p artifacts
for kind in ffi-drop-panic ffi-drop-panic-ok ffi-drop-panic-thread hand-drop-panic async-drop-panic async-drop-panic-ok; do
  out="artifacts/probe-child-${kind}.txt"
  {
    echo "# command: $HOST --probe-child ${kind} <cdylib>"
    "$HOST" --probe-child "$kind" > "/tmp/${kind}.out" 2> "/tmp/${kind}.err"
    code=$?
    echo "# exit status: ${code} (128 + signal means killed by that signal; 134 = SIGABRT)"
    echo "# stdout:"
    sed 's/^/    /' "/tmp/${kind}.out"
    echo "# stderr:"
    sed 's/^/    /' "/tmp/${kind}.err"
  } > "$out"
  echo "wrote $out: $(head -2 "$out" | tail -1)"
done
