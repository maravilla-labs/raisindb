# Workflow examples

End-to-end examples for RaisinDB workflows: regular flows, AI flows, and
the human-in-the-loop inbox primitive.

| Example | Shows |
|---------|-------|
| [approval-flow](approval-flow/) | Run a flow, stream events, complete a human approval task via the inbox API |
| [ai-approval-flow](ai-approval-flow/) | Agent step (one-shot AI call) + a human task **assigned to an AI agent** with confidence-based escalation to a human |
| [event-ticketing](event-ticketing/) | Functions deployed as nodes, designer-format flow, saga compensation, REL-routed human approval |
| [ecommerce-order](ecommerce-order/) | SAGA with **two compensations (LIFO rollback, verified live)**, fraud gate with human review, cancel path that voids the charge |

## The model in one paragraph

A workflow (`raisin:Flow` node) is a list of steps: functions, AI agents,
and humans are uniform step concepts. Function steps queue real function
executions; agent steps call an AI agent; `human_task` steps create a
`raisin:InboxTask` in the assignee's inbox and pause the flow. The assignee
can be a user (`/users/alice`) or an AI agent (`/agents/support`) — agents
decide tasks automatically with structured output and escalate to humans
when not confident. Step arguments, titles, prompts, and conditions support
template expressions (`{{ input.x }}`, `${steps.prev.y}`) and REL
conditions over the same namespaces. Errors are handled per step with
retries, `error_edge` routing, `continue_on_fail`, or saga compensation
(`compensation_ref`), and waits/timeouts (`due_in_seconds`,
`timeout_edge`) are enforced by the engine.

## SDK surface (\`@raisindb/client\`)

- `FlowClient` — `run`, `runAndWait`, `streamEvents` / `createEventStream`,
  `getInstanceStatus`, `resume`, `respondToHumanTask`
- `InboxApi` — `listTasks`, `getTask`, `completeTask` (also available as
  `db.inbox` on a database handle)
- Workflow definition types — `FlowDefinition`, `FlowStepProperties`,
  `InboxTask`, ... (shared with the visual designer)
