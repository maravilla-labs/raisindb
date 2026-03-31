#!/bin/bash
set -e

# Build WASM package using wasm-pack
# Target: web (for use with bundlers like Vite)

echo "Building raisin-sql-wasm..."

# Check if wasm-pack is installed
if ! command -v wasm-pack &> /dev/null; then
    echo "wasm-pack not found. Installing..."
    cargo install wasm-pack
fi

# Build for web target (compatible with Vite/ESM)
wasm-pack build --target web --out-dir pkg --release

# Create package.json for npm linking
cat > pkg/package.json << 'EOF'
{
  "name": "@raisindb/sql-wasm",
  "version": "0.1.0",
  "description": "WASM bindings for RaisinDB SQL parser validation",
  "main": "raisin_sql_wasm.js",
  "types": "raisin_sql_wasm.d.ts",
  "files": [
    "raisin_sql_wasm_bg.wasm",
    "raisin_sql_wasm.js",
    "raisin_sql_wasm.d.ts"
  ],
  "sideEffects": false
}
EOF

echo "Build complete! Output in ./pkg/"
echo ""
echo "To use in admin-console, run:"
echo "  cd ../../../packages/admin-console"
echo "  npm install ../../tooling/packages/raisin-sql-wasm/pkg"
