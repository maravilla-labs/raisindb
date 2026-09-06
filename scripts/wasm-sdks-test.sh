#!/usr/bin/env bash
#
# Run the three WebAssembly guest SDK test suites NATIVELY — no wasm toolchain,
# no server, no componentization. Each SDK compiles for the host with a mock
# host implementation, which is what makes these suites cheap enough to run in
# CI on every push.
#
#   sdks/rust/raisin-sdk   cargo test      (its own workspace)
#   sdks/go/raisin         go test ./...
#   sdks/ts/function-wasm  vitest run
#
# A missing toolchain is reported as SKIPPED rather than failing, so a
# contributor with only one of the three can still run the target. CI sets
# WASM_SDKS_STRICT=1, which turns every skip into a failure — a suite that
# quietly stops running there is the exact rot this script exists to prevent.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STRICT="${WASM_SDKS_STRICT:-0}"

status=0
ran=0
skipped=0

# run_suite <label> <tool> <dir> <command...>
run_suite() {
  local label="$1" tool="$2" dir="$3"
  shift 3

  echo
  echo "── $label  ($* in $dir)"
  if ! command -v "$tool" >/dev/null 2>&1; then
    if [ "$STRICT" = "1" ]; then
      echo "   FAILED: '$tool' is not installed (WASM_SDKS_STRICT=1)"
      status=1
    else
      echo "   SKIPPED: '$tool' is not installed"
      skipped=$((skipped + 1))
    fi
    return
  fi

  if (cd "$ROOT/$dir" && "$@"); then
    ran=$((ran + 1))
  else
    echo "   FAILED: $label"
    status=1
  fi
}

run_suite "Rust SDK"       cargo sdks/rust/raisin-sdk  cargo test
run_suite "Go SDK"         go    sdks/go/raisin        go test ./...
# `--config.verify-deps-before-run=false`: pnpm >= 10 runs a workspace-wide
# dependency check before `run`, and on a mismatch it wants to PURGE and
# reinstall the root node_modules — which is both slow and destructive to a
# tree other builds (cargo build -p raisin-server) are using. Installing deps
# is the caller's job (CI does it explicitly), not a side effect of running a
# test suite.
run_suite "TypeScript SDK" pnpm  sdks/ts/function-wasm \
  pnpm --config.verify-deps-before-run=false run test

echo
echo "wasm SDK suites: $ran ran, $skipped skipped"
if [ "$ran" -eq 0 ]; then
  echo "no suite ran at all — install cargo, go or pnpm (or fix the paths above)" >&2
  exit 1
fi
exit $status
