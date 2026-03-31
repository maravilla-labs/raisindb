#!/bin/bash
set -e

# Build WASM package using wasm-pack
# Target: nodejs (for use in Node.js CLI)

echo "Building raisin-schema-wasm..."

# Check if wasm-pack is installed
if ! command -v wasm-pack &> /dev/null; then
    echo "wasm-pack not found. Installing..."
    cargo install wasm-pack
fi

# Build for nodejs target (for CLI usage)
wasm-pack build --target nodejs --out-dir pkg --release

# Create package.json for npm linking
cat > pkg/package.json << 'EOF'
{
  "name": "@raisindb/schema-wasm",
  "version": "0.1.0",
  "description": "WASM bindings for RaisinDB schema validation",
  "main": "raisin_schema_wasm.js",
  "types": "raisin_schema_wasm.d.ts",
  "files": [
    "raisin_schema_wasm_bg.wasm",
    "raisin_schema_wasm.js",
    "raisin_schema_wasm.d.ts"
  ],
  "sideEffects": false
}
EOF

echo "Build complete! Output in ./pkg/"
echo ""
echo "To use in raisindb-cli, run:"
echo "  cd ../../../packages/raisindb-cli"
echo "  npm install ../../tooling/packages/raisin-schema-wasm/pkg"
