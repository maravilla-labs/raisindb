#!/usr/bin/env bash
# Copy the built guest components into the runtime's fixture directory, where
# `runtime/wasm/tests.rs` picks them up with `include_bytes!`.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
src="$here/target/wasm32-wasip2/release"
dest="$here/../../crates/raisin-functions/src/runtime/wasm/fixtures"

mkdir -p "$dest"
for name in echo call_host log spin alloc wrong_world sockets bench; do
    cp "$src/$name.wasm" "$dest/$name.wasm"
    printf '%8d  %s\n' "$(wc -c < "$dest/$name.wasm")" "$name.wasm"
done
