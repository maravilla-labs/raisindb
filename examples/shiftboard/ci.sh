#!/usr/bin/env bash
# Shiftboard package CI flow - proves the CLI/package install path end to end.
#
# Operator steps that packages cannot express (repo creation, identity users,
# AI provider check, CORS) now use first-class `raisindb` commands - the same
# flow works against local and remote servers (pure API + token).
#
# Env (all optional, defaults target the local dev server):
#   RAISINDB_SERVER  server URL                  (default http://localhost:8081)
#   RAISINDB_TOKEN   admin/system token; if unset it is fetched with
#                    RAISIN_USER / RAISIN_PASSWORD via system auth
#   RAISIN_USER      system username             (default admin)
#   RAISIN_PASSWORD  system password             (default Admin12345!@#)
#   RAISIN_TENANT    tenant for system auth      (default default)
#   REPO             target repository           (default shiftboard-pkg)
#   APP_ORIGIN       SPA origin to allow (CORS)  (default http://localhost:5173)
#   RAISINDB_BIN     raisindb CLI binary         (default: raisindb on PATH)
#   RUN_SMOKE        run smoke.mjs at the end    (default 1; costs Groq tokens)
#
# Usage:
#   ./ci.sh
#   RAISINDB_SERVER=https://my-instance RAISINDB_TOKEN=... REPO=shiftboard-pkg ./ci.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

export RAISINDB_SERVER="${RAISINDB_SERVER:-http://localhost:8081}"
RAISIN_USER="${RAISIN_USER:-admin}"
RAISIN_PASSWORD="${RAISIN_PASSWORD:-Admin12345!@#}"
RAISIN_TENANT="${RAISIN_TENANT:-default}"
REPO="${REPO:-shiftboard-pkg}"
APP_ORIGIN="${APP_ORIGIN:-http://localhost:5173}"
RAISINDB_BIN="${RAISINDB_BIN:-raisindb}"
RUN_SMOKE="${RUN_SMOKE:-1}"

log() { printf '\n=== %s\n' "$*"; }

# ---------------------------------------------------------------------------
log "1/8 Auth (env-driven, no interactive login needed)"
# ---------------------------------------------------------------------------
if [ -z "${RAISINDB_TOKEN:-}" ]; then
  RAISINDB_TOKEN=$(curl -fsS -X POST "$RAISINDB_SERVER/api/raisindb/sys/$RAISIN_TENANT/auth" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$RAISIN_USER\",\"password\":\"$RAISIN_PASSWORD\"}" \
    | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
  echo "Obtained system token for '$RAISIN_USER' (tenant $RAISIN_TENANT)"
else
  echo "Using RAISINDB_TOKEN from environment"
fi
export RAISINDB_TOKEN
export RAISINDB_REPO="$REPO"
# The CLI honors RAISINDB_SERVER / RAISINDB_TOKEN / RAISINDB_REPO (env wins
# over .raisinrc).

auth_curl() { curl -fsS -H "Authorization: Bearer $RAISINDB_TOKEN" "$@"; }

# ---------------------------------------------------------------------------
log "2/8 Create repository '$REPO' if missing"
# ---------------------------------------------------------------------------
$RAISINDB_BIN repo create "$REPO" --exists-ok

echo "Waiting for builtin packages (messaging, ai-tools) to auto-install..."
for i in $(seq 1 60); do
  if auth_curl -o /dev/null "$RAISINDB_SERVER/api/repository/$REPO/main/head/functions/lib/raisin/ai/agent-handler" 2>/dev/null; then
    echo "Builtin packages installed (agent-handler present)"
    break
  fi
  [ "$i" = 60 ] && { echo "ERROR: builtin packages did not install within 60s" >&2; exit 1; }
  sleep 1
done

# ---------------------------------------------------------------------------
log "3/8 Validate + build + upload + install the shiftboard package via CLI"
# ---------------------------------------------------------------------------
# RAISINDB_BIN may be a multi-word command (e.g. "node /path/to/dist/index.js")
$RAISINDB_BIN package deploy "$SCRIPT_DIR/package" --install

# ---------------------------------------------------------------------------
log "4/8 Poll until the package reports installed"
# ---------------------------------------------------------------------------
for i in $(seq 1 60); do
  installed=$(auth_curl "$RAISINDB_SERVER/api/repos/$REPO/packages" \
    | python3 -c 'import sys,json;pkgs=json.load(sys.stdin);print(any(p.get("name")=="shiftboard" and (p.get("properties",{}).get("installed") or p.get("installed")) for p in pkgs))')
  if [ "$installed" = "True" ]; then
    echo "Package 'shiftboard' installed"
    break
  fi
  [ "$i" = 60 ] && { echo "ERROR: package did not reach installed state within 60s" >&2; exit 1; }
  sleep 1
done

echo "Waiting for installed content (assign-shift function + staffing seeds + agent)..."
for i in $(seq 1 60); do
  if auth_curl -o /dev/null "$RAISINDB_SERVER/api/repository/$REPO/main/head/functions/lib/shiftboard/assign-shift" 2>/dev/null \
     && auth_curl -o /dev/null "$RAISINDB_SERVER/api/repository/$REPO/main/head/staffing/shifts/sat-evening" 2>/dev/null \
     && auth_curl -o /dev/null "$RAISINDB_SERVER/api/repository/$REPO/main/head/functions/agents/shift-planner" 2>/dev/null; then
    echo "Functions, seed nodes, and agent are present"
    break
  fi
  [ "$i" = 60 ] && { echo "ERROR: installed content not visible within 60s" >&2; exit 1; }
  sleep 1
done

# ---------------------------------------------------------------------------
log "5/8 Register demo identity users (manager + staff chat accounts)"
# ---------------------------------------------------------------------------
$RAISINDB_BIN user register planner@example.com \
  --password 'Planner12345!' --display-name Planner \
  --repo "$REPO" --tenant "$RAISIN_TENANT" --exists-ok
$RAISINDB_BIN user register anna@example.com \
  --password 'Staff12345!' --display-name Anna \
  --repo "$REPO" --tenant "$RAISIN_TENANT" --exists-ok
$RAISINDB_BIN user register cara@example.com \
  --password 'Staff12345!' --display-name Cara \
  --repo "$REPO" --tenant "$RAISIN_TENANT" --exists-ok

# ---------------------------------------------------------------------------
log "6/8 Check tenant AI provider config (secret stays out of packages + logs)"
# ---------------------------------------------------------------------------
$RAISINDB_BIN ai provider list --tenant "$RAISIN_TENANT" --json | python3 -c '
import sys, json
providers = json.load(sys.stdin)
groq = [p for p in providers if p.get("provider") == "groq"]
ok = groq and groq[0].get("has_api_key") and groq[0].get("enabled")
print("Groq provider configured for tenant:", "yes" if ok else "NO")
sys.exit(0 if ok else 1)
' || { echo "ERROR: configure a Groq API key, e.g.: $RAISINDB_BIN ai provider set groq --api-key-env GROQ_API_KEY --model llama-3.3-70b-versatile" >&2; exit 1; }

# ---------------------------------------------------------------------------
log "7/8 Allow the SPA origin for this repo (CORS)"
# ---------------------------------------------------------------------------
$RAISINDB_BIN cors add "$APP_ORIGIN" --repo "$REPO"
$RAISINDB_BIN cors list --repo "$REPO"

# ---------------------------------------------------------------------------
log "8/8 Smoke test against the package-installed repo (ONE run - costs Groq tokens)"
# ---------------------------------------------------------------------------
if [ "$RUN_SMOKE" = "1" ]; then
  ws_url="${RAISINDB_SERVER/http:\/\//ws://}"
  ws_url="${ws_url/https:\/\//wss://}"
  RAISIN_WS_URL="$ws_url/sys/$RAISIN_TENANT/$REPO" RAISIN_REPO="$REPO" \
    RAISIN_USER="$RAISIN_USER" RAISIN_PASSWORD="$RAISIN_PASSWORD" \
    node "$SCRIPT_DIR/smoke.mjs"
else
  echo "Skipped (RUN_SMOKE=0)"
fi

log "DONE - shiftboard package flow verified on $RAISINDB_SERVER (repo: $REPO)"
