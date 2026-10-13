#!/usr/bin/env bash
# PROTOTYPE (throwaway). Builds the lane's C plugin and runs the host-side probe.
#
# Run it from the repository root INSIDE the devenv shell, because `gcc`, `clang`
# and `cargo` are only on that shell's path:
#
#   cd /home/patric/Projects/metis-agent
#   devenv shell -- bash /tmp/metis-plugin-registry-round-3/ownership/c-ownership/build.sh
#
# Every command below is the exact one recorded in FINDINGS-ownership.md.
set -euo pipefail

COPY=/tmp/metis-plugin-registry-round-3/ownership
CC=${CC:-gcc}

$CC -std=c11 -Wall -Wextra -Werror -O2 -fPIC -shared -pthread \
    -I "$COPY/crates/proto-abi/include" \
    -o "$COPY/c-ownership/libmetis_c_plugin.so" \
    "$COPY/c-ownership/c_plugin.c"

clang -std=c11 -Wall -Wextra -Werror -O2 -fPIC -shared -pthread \
    -I "$COPY/crates/proto-abi/include" \
    -o "$COPY/c-ownership/libmetis_c_plugin_clang.so" \
    "$COPY/c-ownership/c_plugin.c"

cargo build --release --manifest-path "$COPY/Cargo.toml"

C_PLUGIN_LIB="$COPY/c-ownership/libmetis_c_plugin.so" \
    cargo run -q --release --manifest-path "$COPY/Cargo.toml" -p proto-host --bin c-ownership-probe
