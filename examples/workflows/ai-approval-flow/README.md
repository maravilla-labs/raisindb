# AI Approval Flow — agents as steps AND as approvers

Demonstrates that functions, AI agents, and humans are uniform step
concepts in RaisinDB workflows:

```
start ──▶ agent_step (summarize) ──▶ human_task assigned to /agents/refund-approver ──▶ end
                                            │
                                            │ low confidence / AI error
                                            ▼
                                   escalates to /users/admin
                                   (same inbox, same completion API)
```

- The `agent_step` makes a one-shot AI call with a template-resolved prompt
  (`{{ input.amount }}`, `{{ steps.summarize.response }}`).
- The approval `human_task`'s **assignee is an AI agent**. The agent gets
  the task content plus flow context and must answer a structured decision
  `{ decision, reasoning, confidence }`. A decision at or above
  `min_confidence` completes the task (recorded with
  `completed_by: /agents/refund-approver`) and the flow continues without
  any human involvement.
- On AI errors, unparseable output, or low confidence, the task is
  **escalated**: reassigned to `escalation_assignee`, with
  `escalated_from` / `escalation_reason` recorded. A human completes it
  through exactly the same inbox API (or the admin console Inbox page).

## Run it

```bash
cd examples/workflows/ai-approval-flow
npm install
RAISIN_AGENT_PROVIDER=openai RAISIN_AGENT_MODEL=gpt-4o-mini npm start
```

Requires an AI provider configured on the server for the agent decision
path. **Without one the example still completes** — through the escalation
path, which is itself half of what this example demonstrates.

Environment overrides: `RAISIN_URL`, `RAISIN_REPO`, `RAISIN_USER`,
`RAISIN_PASSWORD`, `RAISIN_AGENT_PROVIDER`, `RAISIN_AGENT_MODEL`.
