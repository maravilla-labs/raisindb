# Approval Flow — human in the loop, end to end

The smallest complete example of RaisinDB's workflow + inbox primitives:

```
start ──▶ human approval task ──▶ decision ──▶ end
                  │                   │
                  ▼                   └─▶ rejected ──▶ end
        task in /users/admin inbox
```

1. The flow definition (a `raisin:Flow` node) declares a `human_task` step.
   Its title uses template expressions (`{{ input.order_id }}`) resolved
   against the flow input.
2. Running the flow pauses it at the approval and creates a
   `raisin:InboxTask` in the assignee's inbox.
3. Completing the task through the inbox API records who decided and
   resumes the flow; the decision is available to later steps as
   `__human_response.action`.

## Run it

```bash
# 1. Start a dev server (from the repo root)
cargo build --release --package raisin-server --features "storage-rocksdb,websocket,pgwire"
./target/release/raisin-server --config examples/cluster/node1.toml --dev-mode

# 2. Run the example
cd examples/workflows/approval-flow
npm install
npm start
```

Environment overrides: `RAISIN_URL`, `RAISIN_REPO`, `RAISIN_USER`,
`RAISIN_PASSWORD`.

## What to look at

- `run.mjs` — flow definition, `FlowClient` (run + SSE event stream), and
  `InboxApi` (list + complete tasks).
- The same task also appears in the admin console under
  **Repository → Inbox**, where it can be completed from the UI instead.
- Assignees can be AI agents too (`/agents/...`) — see the
  `../ai-approval-flow` example.
