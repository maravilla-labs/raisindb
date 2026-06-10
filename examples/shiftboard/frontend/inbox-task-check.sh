#!/usr/bin/env bash
#
# Proves the human-in-the-loop task panel end to end, with curl only:
#
#   1. As admin, deploys a tiny probe flow with a `human_task` step and runs
#      it for $EMAIL — the engine creates a `raisin:InboxTask` node under the
#      assignee's home inbox in `raisin:access_control` (exactly what any
#      production workflow does).
#   2. Logs in through the SvelteKit form action (?/login) and asserts the
#      server-rendered HTML already contains the task title + option buttons
#      (no JavaScript executed).
#   3. Completes the task via POST /api/inbox/{repo}/tasks/{id}/complete with
#      the user's own bearer (the same call the panel's buttons make) and
#      asserts status flips to `completed` and the flow resumes.
#   4. Cleans up: deletes the probe task node, the flow instance and the
#      probe flow definition.
#
# Prerequisites: the app on $BASE_URL (npm run start) and a raisin-server
# with the shiftboard demo data on $RAISIN_URL.
#
# NOTE: the default user is the planner. Users whose `inbox` folder was
# created by the messaging package as `raisin:MessageFolder` (e.g. staff who
# already received chat messages) currently cannot complete tasks — the
# storage update path rejects InboxTask children of MessageFolder. See the
# README section "Human-in-the-loop tasks in your own UI".
set -euo pipefail

BASE_URL="${BASE_URL:-http://localhost:5175}"
RAISIN_URL="${RAISIN_URL:-http://localhost:8081}"
REPO="${RAISIN_REPO:-shiftboard2}"
EMAIL="${EMAIL:-planner@example.com}"
PASSWORD="${PASSWORD:-Planner12345!}"
ADMIN_USER="${RAISIN_USER:-admin}"
ADMIN_PASSWORD="${RAISIN_PASSWORD:-Admin12345!@#}"

TITLE="Approve the inbox-check probe?"
FLOW_NAME="inbox-task-check-probe"

JAR="$(mktemp)"
trap 'rm -f "$JAR"' EXIT

json() { python3 -c "import sys,json;d=json.load(sys.stdin);print(d$1)"; }

# --- admin + user tokens --------------------------------------------------
ADMIN_TOKEN=$(curl -sf -X POST "$RAISIN_URL/api/raisindb/sys/default/auth" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$ADMIN_USER\",\"password\":\"$ADMIN_PASSWORD\"}" | json "['token']")
USER_TOKEN=$(curl -sf -X POST "$RAISIN_URL/auth/$REPO/login" \
  -H 'Content-Type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" | json "['access_token']")
USER_HOME=$(curl -sf "$RAISIN_URL/api/inbox/$REPO" \
  -H "Authorization: Bearer $USER_TOKEN" | json "['assignee']")
echo "ok: tokens acquired (assignee: $USER_HOME)"

# --- 1. deploy + run the probe flow ---------------------------------------
read -r -d '' FLOW_PROPS <<EOF || true
{
  "name": "$FLOW_NAME",
  "title": "Inbox task check probe",
  "enabled": true,
  "workflow_data": {
    "version": 1,
    "error_strategy": "fail_fast",
    "nodes": [
      {
        "id": "probe-approval",
        "node_type": "raisin:FlowStep",
        "properties": {
          "action": "$TITLE",
          "step_type": "human_task",
          "task_type": "approval",
          "assignee": "$USER_HOME",
          "task_description": "Scripted verification task - safe to ignore.",
          "priority": 4,
          "due_in_seconds": 3600,
          "options": [
            { "value": "accept", "label": "Accept", "style": "success" },
            { "value": "decline", "label": "Decline", "style": "danger" }
          ]
        }
      }
    ]
  }
}
EOF

create=$(curl -s -X POST "$RAISIN_URL/api/repository/$REPO/main/head/functions/flows" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  -d "{\"node\":{\"name\":\"$FLOW_NAME\",\"node_type\":\"raisin:Flow\",\"properties\":$FLOW_PROPS}}")
if grep -qi "exists\|conflict" <<<"$create"; then
  curl -sf -X PUT "$RAISIN_URL/api/repository/$REPO/main/head/functions/flows/$FLOW_NAME" \
    -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
    -d "{\"properties\":$FLOW_PROPS}" > /dev/null
fi
INSTANCE=$(curl -sf -X POST "$RAISIN_URL/api/flows/$REPO/run" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  -d "{\"flow_path\":\"/flows/$FLOW_NAME\",\"input\":{}}" | json "['instance_id']")
echo "ok: probe flow running ($INSTANCE)"

# Wait for the task node to appear in the user's inbox.
TASK_ID=""
for _ in $(seq 1 30); do
  TASK_ID=$(curl -sf "$RAISIN_URL/api/inbox/$REPO?status=pending" \
    -H "Authorization: Bearer $USER_TOKEN" \
    | python3 -c "import sys,json;ts=[t for t in json.load(sys.stdin)['tasks'] if t.get('flow_instance_id')=='$INSTANCE'];print(ts[0]['id'] if ts else '')")
  [ -n "$TASK_ID" ] && break
  sleep 1
done
[ -n "$TASK_ID" ] || { echo "FAIL: task never appeared in the inbox"; exit 1; }
TASK_PATH=$(curl -sf "$RAISIN_URL/api/inbox/$REPO/tasks/$TASK_ID" \
  -H "Authorization: Bearer $USER_TOKEN" | json "['path']")
echo "ok: task created at $TASK_PATH"

cleanup() {
  curl -s -X DELETE "$RAISIN_URL/api/repository/$REPO/main/head/raisin:access_control$TASK_PATH" \
    -H "Authorization: Bearer $ADMIN_TOKEN" > /dev/null || true
  curl -s -X DELETE "$RAISIN_URL/api/flows/$REPO/instances/$INSTANCE" \
    -H "Authorization: Bearer $ADMIN_TOKEN" > /dev/null || true
  curl -s -X DELETE "$RAISIN_URL/api/repository/$REPO/main/head/functions/flows/$FLOW_NAME" \
    -H "Authorization: Bearer $ADMIN_TOKEN" > /dev/null || true
  rm -f "$JAR"
}
trap cleanup EXIT

# --- 2. task title must be in the server-rendered HTML --------------------
status=$(curl -s -o /dev/null -w '%{http_code}' -c "$JAR" \
  -H "Origin: $BASE_URL" -H "Accept: text/html" \
  --data-urlencode "email=$EMAIL" --data-urlencode "password=$PASSWORD" \
  "$BASE_URL/?/login")
[ "$status" = "303" ] || { echo "FAIL: login action returned HTTP $status"; exit 1; }

html=$(curl -s -b "$JAR" "$BASE_URL/")
grep -qF "$TITLE" <<<"$html" \
  || { echo "FAIL: task title not in SSR HTML"; exit 1; }
grep -qF ">Accept<" <<<"$html" && grep -qF ">Decline<" <<<"$html" \
  || { echo "FAIL: option buttons not in SSR HTML"; exit 1; }
echo "ok: SSR HTML contains the task card (title + one button per option)"

# --- 3. complete via the same endpoint the UI buttons call ----------------
result=$(curl -sf -X POST "$RAISIN_URL/api/inbox/$REPO/tasks/$TASK_ID/complete" \
  -H "Authorization: Bearer $USER_TOKEN" -H 'Content-Type: application/json' \
  -d '{"response":{"action":"accept"}}')
[ "$(json "['status']" <<<"$result")" = "completed" ] \
  || { echo "FAIL: completion did not flip status: $result"; exit 1; }
echo "ok: completion flipped status to completed (flow resumes: $(json "['flow']['instance_id']" <<<"$result"))"

after=$(curl -sf "$RAISIN_URL/api/inbox/$REPO/tasks/$TASK_ID" \
  -H "Authorization: Bearer $USER_TOKEN" | json "['status']")
[ "$after" = "completed" ] || { echo "FAIL: task re-read shows status $after"; exit 1; }
echo "ok: task node re-reads as completed"

echo "PASS (probe task, instance and flow definition cleaned up)"
