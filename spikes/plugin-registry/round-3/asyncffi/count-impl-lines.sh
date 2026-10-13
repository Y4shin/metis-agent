#!/usr/bin/env bash
# PROTOTYPE (throwaway). Lane async-ffi: per-implementation line counts for the
# three future objects in the plugin cdylib's future.rs, by section banner.
# Run from this copy's root:
#   bash count-impl-lines.sh
set -uo pipefail
F=crates/proto-plugin-cdylib/src/future.rs

report() {
  local name="$1" start="$2" end="$3"
  awk -v name="$name" -v s="$start" -v e="$end" '
    /^/ {
      if (n == 0 && index($0, s) > 0) { on = 1 }
      if (on && e != "" && index($0, e) > 0) { on = 0 }
      if (on) {
        total++
        if ($0 ~ /unsafe/) unsafe++
        trimmed = $0
        sub(/^[ \t]+/, "", trimmed)
        if (trimmed != "" && substr(trimmed, 1, 2) != "//") code++
      }
    }
    END { printf "%-28s total %4d  code %4d  lines containing unsafe %4d\n", name, total, code, unsafe }
  ' "$F"
}

echo "future.rs: $(wc -l < "$F") lines total"
echo
report "async-ffi object"      "--- the async-ffi future"        "--- the hand-rolled async object"
report "hand-rolled object"    "--- the hand-rolled async object" "--- the hand-written future"
report "hand-written vtable"   "--- the hand-written future"      "--- the exported entries"
report "shared"                "--- instrumentation"              "--- the async-ffi future"
echo
echo "safety of the two object wrappers, plugin-side:"
grep -n "catch_unwind" "$F" | sed 's/^/  /'
