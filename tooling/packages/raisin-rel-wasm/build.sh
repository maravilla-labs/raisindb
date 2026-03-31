#!/bin/bash
set -e

echo "Building raisin-rel-wasm..."
wasm-pack build --target web --release

echo "Build complete! Output in ./pkg/"
