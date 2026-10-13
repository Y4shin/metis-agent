#!/usr/bin/env bash
# Acceptance probes for ticket `contract-gate-hardening`.
#
# The ticket's seams are command-level, not Rust-level (arch spec: "Test types:
# the two buf commands and the deliberate-edit probe for seams 1 and 2; the
# script for seam 3. No new Rust test."). This script encodes those probes, so
# the red -> green loop has an executable test:
#
#   seam 1 (configuration): `buf lint` and `buf breaking`, run from the
#           repository root, read the root `buf.yaml`; a deliberate edit to the
#           root `breaking` block changes the gate's verdict.
#   seam 2 (token): the plugin-resolving step receives `BUF_TOKEN` from
#           `secrets.BUF_TOKEN`, and an absent or empty secret leaves the gate
#           working without authentication. Which form buf needs is measured
#           here rather than assumed.
#   seam 3 (reproduction): `scripts/regenerate-contracts.sh --check` exits 0 on
#           a clean tree from the root layout, and non-zero after a hand edit.
#
# Every probe restores what it touched, so the tree is unchanged on exit.
#
# Network budget: one codegen run resolves two BSR remote plugins, and anonymous
# resolution is rate limited, so the probes spend as few generations as the
# acceptance criteria allow. The empty-secret half of seam 2 is measured on
# cheap BSR reads instead of on a second generation, and a rate-limited run is
# reported as skipped rather than as a failing probe.
#
# Run from inside the devenv shell, from anywhere in the repository:
#   devenv shell -- bash docs/tasks/metis-foundations/tickets/contract-gate-hardening/.work/probe.sh
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../../../../" && pwd)"
cd "$repo_root"

work="$(mktemp -d)"
root_config="$repo_root/buf.yaml"
gen_config="$repo_root/buf.gen.yaml"
proto="$repo_root/contracts/proto/metis/v1/daemon.proto"
generated="$repo_root/crates/api-contracts/generated/buffa/metis.v1.rs"
ci="$repo_root/.github/workflows/ci.yml"

backup() { [ -f "$1" ] && cp "$1" "$work/$(basename "$1").bak"; return 0; }
restore() {
  for pair in "$root_config:buf.yaml" "$gen_config:buf.gen.yaml" "$proto:daemon.proto" "$generated:metis.v1.rs"; do
    target="${pair%%:*}"; name="${pair##*:}"
    [ -f "$work/$name.bak" ] && cp "$work/$name.bak" "$target"
  done
  return 0
}
cleanup() { restore; rm -rf "$work"; }
trap cleanup EXIT

failures=0
skipped=0
ok() { printf '[ok]   %s\n' "$1"; }
bad() { printf '[fail] %s\n' "$1"; failures=$((failures + 1)); }
skip() { printf '[skip] %s\n' "$1"; skipped=$((skipped + 1)); }
expect_exit() { # expect_exit <expected> <actual> <label>
  if [ "$2" -eq "$1" ]; then ok "$3 (exit $2)"; else bad "$3 (exit $2, expected $1)"; fi
}
expect_contains() { # expect_contains <haystack> <needle> <label>
  case "$1" in *"$2"*) ok "$3" ;; *) bad "$3" ;; esac
}
expect_lacks() {
  case "$1" in *"$2"*) bad "$3" ;; *) ok "$3" ;; esac
}

backup "$root_config"; backup "$gen_config"; backup "$proto"; backup "$generated"

breaking() { buf breaking --against '.git#branch=main' --against-config buf.yaml >"$work/breaking.log" 2>&1; }

echo "== seam 1: one root configuration, read by every buf command"
if [ -f "$root_config" ]; then ok "root buf.yaml exists"; else bad "root buf.yaml exists"; fi
if [ ! -e "$repo_root/contracts/buf.yaml" ]; then ok "contracts/buf.yaml is gone"; else bad "contracts/buf.yaml is gone"; fi
modules="$(buf config ls-modules 2>/dev/null)"
expect_contains "$modules" "contracts/proto" "root configuration declares the module at contracts/proto (ls-modules: ${modules//$'\n'/ })"
config_text="$(cat "$root_config" 2>/dev/null || true)"
expect_contains "$config_text" "- STANDARD" "root configuration lints STANDARD"
expect_contains "$config_text" "- FILE" "root configuration breaks FILE"

buf lint >"$work/lint.log" 2>&1
expect_exit 0 "$?" "buf lint from the repository root passes"

breaking
expect_exit 0 "$?" "buf breaking from the repository root passes on the clean tree"

# Deliberate breaking edit to the contract: rename a field, keeping its number.
# FILE mode forbids it; WIRE mode allows it. That difference is what proves the
# root `breaking` block, and only it, governs the gate's verdict.
sed -i 's/^  string message = 1;$/  string text = 1;/' "$proto"
breaking
expect_exit 100 "$?" "a field rename in the working tree fails buf breaking"
if [ -f "$root_config" ]; then
  sed -i -E 's/^([[:space:]]*)- FILE$/\1- WIRE/' "$root_config"
  expect_contains "$(cat "$root_config")" "- WIRE" "deliberate edit: root breaking block now declares WIRE"
  breaking
  expect_exit 0 "$?" "the same input passes once the root breaking block declares WIRE"
  cp "$work/buf.yaml.bak" "$root_config"
  breaking
  expect_exit 100 "$?" "restoring the root breaking block restores the failing verdict"
else
  bad "deliberate edit to the root breaking block (no root buf.yaml to edit)"
fi
cp "$work/daemon.proto.bak" "$proto"

# The lint half of the same seam: a lint violation in the contract is caught
# from the root, and a root `lint` block edit clears it. Without a root
# configuration this lint run would not even cover the contract.
sed -i 's/^  string message = 1;$/  string Message = 1;/' "$proto"
buf lint >"$work/lint-violation.log" 2>&1
expect_exit 100 "$?" "a lint violation fails buf lint from the repository root"
if [ -f "$root_config" ]; then
  awk '{print} /^lint:/{print "  except:"; print "    - FIELD_LOWER_SNAKE_CASE"}' \
    "$work/buf.yaml.bak" >"$work/buf-lint-except.yaml"
  cp "$work/buf-lint-except.yaml" "$root_config"
  buf lint >"$work/lint-except.log" 2>&1
  expect_exit 0 "$?" "the same contract passes once the root lint block excepts the rule"
  cp "$work/buf.yaml.bak" "$root_config"
fi
cp "$work/daemon.proto.bak" "$proto"
buf lint >"$work/lint-restored.log" 2>&1
expect_exit 0 "$?" "restoring both leaves buf lint from the root passing"

echo "== seam 2: BUF_TOKEN on the plugin-resolving step, and without one"
# Which form buf needs is the measurement this criterion asks for. The two
# contract gates that run before generation never touch the BSR, and the codegen
# step does, so the probes below separate the two.
auth_failure='the BUF_TOKEN environment variable is not valid for buf.build'
plugin_read() { # plugin_read <label> <env-command...>
  local label="$1"; shift
  timeout 120 "$@" buf registry plugin info buf.build/anthropics/buffa >"$work/read-$label.log" 2>&1
  echo $?
}
# A non-empty, invalid token: buf refuses it before it reaches the BSR.
code="$(plugin_read bogus env BUF_TOKEN=not-a-real-token)"
if [ "$code" -eq 0 ] || ! grep -q "$auth_failure" "$work/read-bogus.log"; then
  bad "a non-empty invalid BUF_TOKEN is refused before the BSR (exit $code)"
else
  ok "a non-empty invalid BUF_TOKEN is refused before the BSR (exit $code)"
fi
# Unset and empty: the read reaches the BSR anonymously. BSR answers with its
# own precondition for this plugin, which is what proves the request was served
# rather than refused for authentication.
for form in unset empty; do
  case "$form" in
    unset) code="$(plugin_read "$form" env -u BUF_TOKEN)";;
    empty) code="$(plugin_read "$form" env BUF_TOKEN=)";;
  esac
  if grep -q "$auth_failure" "$work/read-$form.log"; then
    bad "BUF_TOKEN $form reaches the BSR anonymously (buf reported: $auth_failure)"
  else
    ok "BUF_TOKEN $form reaches the BSR anonymously (exit $code, no auth failure)"
  fi
done

# The two gates that run from the repository root resolve no plugins, so they
# pass even with a token buf would refuse: they need no token at all.
BUF_TOKEN=not-a-real-token buf lint >"$work/lint-bogus-token.log" 2>&1
expect_exit 0 "$?" "buf lint passes with a token buf refuses (it resolves no plugins)"
BUF_TOKEN=not-a-real-token breaking
expect_exit 0 "$?" "buf breaking passes with a token buf refuses (it resolves no plugins)"

# The codegen step is the one that resolves plugins. It is rate limited when it
# runs anonymously, so a rate-limited attempt is retried a bounded number of
# times and then reported as skipped, never as a false failure.
drift_check() { # drift_check <log-name> <env-command...>
  local log="$work/$1.log"; shift
  local attempt code
  for attempt in 1 2; do
    "$@" bash scripts/regenerate-contracts.sh --check >"$log" 2>&1
    code=$?
    grep -q 'resource_exhausted' "$log" || break
    printf '  [info] BSR rate limit on attempt %s of 2, waiting 30s\n' "$attempt" >&2
    sleep 30
  done
  echo "$code"
}
code="$(drift_check unset env -u BUF_TOKEN)"
if grep -q 'resource_exhausted' "$work/unset.log"; then
  skip "the drift check passes with BUF_TOKEN unset (BSR rate limited the probe)"
else
  expect_exit 0 "$code" "the drift check passes with BUF_TOKEN unset"
fi

ci_text="$(cat "$ci" 2>/dev/null || true)"
expect_contains "$ci_text" 'BUF_TOKEN: ${{ secrets.BUF_TOKEN }}' "CI supplies BUF_TOKEN from secrets.BUF_TOKEN"
gen_step="$(awk '/- name: Contract regeneration check/{keep=1; print; next} /^      - name: /{keep=0} keep' "$ci")"
expect_contains "$gen_step" 'BUF_TOKEN: ${{ secrets.BUF_TOKEN }}' "the plugin-resolving CI step carries BUF_TOKEN"
# An absent secret arrives as the empty string. The step must not hand buf a
# present-but-empty variable, so it drops it before buf runs.
expect_contains "$gen_step" 'unset BUF_TOKEN' "the plugin-resolving CI step drops an empty BUF_TOKEN instead of passing it"
token_steps="$(awk '
  /^      - name: /{ if (name != "" && hit) print name; name=$0; sub(/^      - name: /, "", name); hit=0 }
  /BUF_TOKEN/{ hit=1 }
  END { if (name != "" && hit) print name }
' "$ci" | paste -sd, -)"
expect_exit 1 "$(printf '%s' "$token_steps" | tr ',' '\n' | grep -c .)" "BUF_TOKEN appears on exactly one CI step (${token_steps:-none})"

echo "== seam 3: the regeneration command reproduces the committed trees"
# Offline half of seam 3: the same check with the BSR out of the loop. A stub
# `buf` on PATH puts a pristine snapshot of the committed trees where generation
# would, so the diff half of the check is exercised without resolving a plugin.
# This keeps the negative probe measurable when the anonymous rate limit refuses
# a real generation; the generation half of the seam runs for real below.
stub_dir="$work/stub-bin"
snapshot="$work/snapshot"
mkdir -p "$stub_dir" "$snapshot"
cp -r "$repo_root/crates/api-contracts/generated" "$snapshot/generated"
cat >"$stub_dir/buf" <<'STUB'
#!/usr/bin/env bash
# Probe stub: stands in for `buf generate` by copying the snapshot of the
# committed trees to the output paths named in the scratch template, so the
# drift diff runs offline against the unedited trees.
set -euo pipefail
template=""
while [ "$#" -gt 0 ]; do
  case "$1" in --template) template="$2"; shift 2 ;; *) shift ;; esac
done
for out in $(awk '/^[[:space:]]*out:[[:space:]]/{print $2}' "$template"); do
  tree="$(basename "$out")"
  mkdir -p "$out"
  cp "$PROBE_SNAPSHOT/generated/$tree"/*.rs "$out/"
done
STUB
chmod +x "$stub_dir/buf"
export PROBE_SNAPSHOT="$snapshot"
PATH="$stub_dir:$PATH" bash scripts/regenerate-contracts.sh --check >"$work/stub-clean.log" 2>&1
expect_exit 0 "$?" "the drift check passes on a clean tree with generation stubbed (offline)"
printf '\n// probe: deliberate drift\n' >>"$generated"
PATH="$stub_dir:$PATH" bash scripts/regenerate-contracts.sh --check >"$work/stub-drift.log" 2>&1
expect_exit 1 "$?" "the drift check fails after a hand edit with generation stubbed (offline)"
cp "$work/metis.v1.rs.bak" "$generated"

if [ -f "$generated" ]; then
  printf '\n// probe: deliberate drift\n' >>"$generated"
  code="$(drift_check drift)"
  if grep -q 'resource_exhausted' "$work/drift.log"; then
    skip "regenerate-contracts.sh --check fails after a hand edit (BSR rate limited the probe)"
  elif [ "$code" -ne 0 ]; then
    ok "regenerate-contracts.sh --check fails after a hand edit (exit $code)"
  else
    bad "regenerate-contracts.sh --check fails after a hand edit (exit 0)"
  fi
  cp "$work/metis.v1.rs.bak" "$generated"
else
  bad "regenerate-contracts.sh --check fails after a hand edit (generated tree missing)"
fi

echo "== criterion 5: the docs describe the new locations"
readme="$(cat "$repo_root/contracts/README.md" 2>/dev/null || true)"
testing="$(cat "$repo_root/docs/testing.md" 2>/dev/null || true)"
expect_contains "$readme" "buf.yaml" "contracts/README.md still names the buf configuration"
expect_contains "$readme" "repository root" "contracts/README.md says the configuration lives at the repository root"
expect_lacks "$readme" "cd contracts && buf lint" "contracts/README.md no longer lints from contracts/"
expect_contains "$readme" "secrets.BUF_TOKEN" "contracts/README.md documents the token wiring"
expect_contains "$readme" "unset" "contracts/README.md documents that an absent secret is left unset"
expect_contains "$testing" "buf lint" "docs/testing.md documents buf lint"
expect_lacks "$testing" "cd contracts && buf lint" "docs/testing.md no longer lints from contracts/"
expect_contains "$readme" 'bash scripts/regenerate-contracts.sh' "the documented developer command is unchanged and still runs from the root"

echo
if [ "$failures" -eq 0 ] && [ "$skipped" -eq 0 ]; then
  echo "[result] PASS"
elif [ "$failures" -eq 0 ]; then
  echo "[result] PASS ($skipped probe(s) skipped: BSR rate limited)"
else
  echo "[result] FAIL ($failures probe(s) failed, $skipped skipped)"
  exit 1
fi
