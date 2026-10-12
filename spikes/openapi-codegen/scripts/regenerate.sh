#!/usr/bin/env bash
# Regenerate the spike's OpenAPI output deterministically.
#
# Everything is pinned: the generator version, the spec, the generator options and
# a small post-generation normalization that the generators cannot express.
#
# Java is not in devenv.nix, so run this through a JVM explicitly:
#   nix shell nixpkgs#jdk21 --command spikes/openapi-codegen/scripts/regenerate.sh
#
# With `--check` it generates into a temporary directory and exits non-zero if the
# normalized output differs from the checked-in `generated/` tree. That is the
# drift check.
set -euo pipefail

GENERATOR_VERSION="7.10.0"
GENERATOR_JAR_SHA256="615e014705af34861e789e0b2a11075d3c80db134f881e937265479a4a83996e"

spike_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tools_dir="$spike_dir/.tools"
jar="$tools_dir/openapi-generator-cli-$GENERATOR_VERSION.jar"
spec="$spike_dir/spec/openapi.yaml"

mode="write"
if [[ "${1:-}" == "--check" ]]; then
  mode="check"
fi

if ! command -v java >/dev/null 2>&1; then
  echo "error: java not found on PATH (see the task's JVM note)" >&2
  exit 2
fi

mkdir -p "$tools_dir"
if [[ ! -f "$jar" ]]; then
  curl -fsSL -o "$jar" \
    "https://repo1.maven.org/maven2/org/openapitools/openapi-generator-cli/$GENERATOR_VERSION/openapi-generator-cli-$GENERATOR_VERSION.jar"
fi
echo "$GENERATOR_JAR_SHA256  $jar" | sha256sum --check --status || {
  echo "error: generator jar checksum mismatch" >&2
  exit 2
}

out="$spike_dir/generated"
if [[ "$mode" == "check" ]]; then
  out="$(mktemp -d)"
  trap 'rm -rf "$out"' EXIT
fi

rm -rf "$out/server" "$out/client"
mkdir -p "$out"

generate() { # generator, package name, destination
  java -jar "$jar" generate \
    -i "$spec" \
    -g "$1" \
    -o "$out/$3" \
    --additional-properties="packageName=$2" >/dev/null
}

generate rust-axum metis-spike-server server
generate rust metis-spike-client client

# Normalization 1: the generated client pulls reqwest's default native-tls, which
# needs OpenSSL. The repository devenv has no OpenSSL, so pin reqwest to rustls and
# enable streaming.
client_manifest="$out/client/Cargo.toml"
sed -i 's|^reqwest = { version = "\^0.12", features = \["json", "multipart"\] }$|reqwest = { version = "^0.12", default-features = false, features = ["json", "multipart", "rustls-tls", "stream"] }|' "$client_manifest"
grep -q 'rustls-tls' "$client_manifest" || {
  echo "error: reqwest normalization did not apply (generator output changed upstream)" >&2
  exit 2
}

# Normalization 2: generated crates must opt into the workspace lint posture.
for manifest in "$out/server/Cargo.toml" "$out/client/Cargo.toml"; do
  printf '\n[lints]\nworkspace = true\n' >> "$manifest"
done

# Normalization 3: raw output trips four lints under `-D warnings`. Two allows per
# crate are the smallest fix short of patching the templates.
sed -i '1i #![allow(unreachable_code)]\n#![allow(mismatched_lifetime_syntaxes)]' "$out/server/src/lib.rs"
sed -i '1i #![allow(clippy::empty_docs)]\n#![allow(clippy::derivable_impls)]' "$out/client/src/lib.rs"

# Normalization 4: the rust-axum README embeds the generation time, which would
# make every regeneration a drift. Strip it so the output is byte-stable.
sed -i '/^- Build date: /d' "$out/server/README.md" "$out/client/README.md"

if [[ "$mode" == "check" ]]; then
  if diff -ru "$spike_dir/generated" "$out"; then
    echo "drift check: no diff"
  else
    echo "drift check: generated output differs from generated/ (run regenerate.sh)" >&2
    exit 1
  fi
fi
