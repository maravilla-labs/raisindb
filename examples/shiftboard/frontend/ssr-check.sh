#!/usr/bin/env bash
#
# Proves the Shiftboard SvelteKit app does real server-side rendering:
#
#   1. Logs in through the SvelteKit form action (?/login), which stores the
#      RaisinDB identity tokens in httpOnly cookies (303 redirect on success).
#   2. Fetches / with that cookie jar using plain curl — no JavaScript is
#      ever executed.
#   3. Asserts the raw HTML contains a shift title loaded from the database.
#
# Prerequisites: the app running on $BASE_URL (npm run start, or npm run
# preview / npm run dev) and a raisin-server with the shiftboard demo data.
set -euo pipefail

BASE_URL="${BASE_URL:-http://localhost:5175}"
EMAIL="${EMAIL:-planner@example.com}"
PASSWORD="${PASSWORD:-Planner12345!}"
EXPECT="${EXPECT:-Saturday Evening}"

JAR="$(mktemp)"
trap 'rm -f "$JAR"' EXIT

# 1. Login via the form action, sending the headers a browser would:
#    - Origin: SvelteKit's CSRF protection rejects form posts whose Origin
#      doesn't match the host.
#    - Accept: text/html: with curl's default */* SvelteKit negotiates an
#      action JSON response instead of the 303 redirect.
status=$(curl -s -o /dev/null -w '%{http_code}' -c "$JAR" \
  -H "Origin: $BASE_URL" \
  -H "Accept: text/html" \
  --data-urlencode "email=$EMAIL" \
  --data-urlencode "password=$PASSWORD" \
  "$BASE_URL/?/login")

if [ "$status" != "303" ]; then
  echo "FAIL: login action returned HTTP $status (expected 303 redirect)"
  exit 1
fi

if ! grep -q "shiftboard_access" "$JAR"; then
  echo "FAIL: login did not set the shiftboard_access cookie"
  exit 1
fi
echo "ok: login via ?/login form action set httpOnly auth cookies"

# 2. Fetch the page HTML with the auth cookie — curl runs no JavaScript.
html=$(curl -s -b "$JAR" "$BASE_URL/")

# 3. The server-rendered HTML must already contain shift data.
if grep -qF "$EXPECT" <<<"$html"; then
  echo "ok: SSR HTML contains \"$EXPECT\" without JavaScript execution"
  echo "PASS"
else
  echo "FAIL: \"$EXPECT\" not found in server-rendered HTML"
  exit 1
fi
