# Raisin Orchestration Model (ROM) Spec

This document defines the Raisin Orchestration Model (ROM): a RaisinDB-native
workflow specification that is not compatible with XState. ROM is the product
spec. The `raisin-flow` runtime and designer execute and author ROM.

ROM goals:
- Simple to complex: one-step flows up to long-running, multi-actor systems.
- Durable by default: all execution state is persisted as nodes.
- Human-first: messaging is the default human I/O channel.
- Agent-first: streaming, validation, retries, and tool policy are native.
- Visual-first: the designer can render and validate every construct.

## Terminology

- Flow Definition: a ROM workflow stored in `raisin:Flow.workflow_data`.
- Flow Instance: a running instance stored in `raisin:FlowInstance`.
- Step: a node in the flow graph.
- Event: a runtime occurrence emitted by the flow executor.
- Message: a `raisin:Message` node for human I/O, approvals, and notifications.

## Node Types (ROM uses existing RaisinDB node types)

ROM is expressed through the existing node type system:

- `raisin:Flow` - Flow definition container with ROM `workflow_data`.
- `raisin:FlowInstance` - Runtime state and persistence for a flow instance.
- `raisin:FlowStepExecution` - Step-level execution record.
- `raisin:Message` and `raisin:MessageFolder` - Human I/O and notifications.
- `raisin:AIAgent` - Agent configuration (tools, rules, model, prompts).
- `raisin:Actor`, `raisin:AIActor`, `raisin:HumanActor` - Actor registry and
  capabilities (optional but recommended for handoffs and task routing).
- `raisin:ActorTask` - Structured task records (optional; messages are primary).

## ROM Workflow Data Schema (stored in raisin:Flow.workflow_data)

ROM is represented as a directed graph with optional nesting.

```json
{
  "rom_version": "1.0.0",
  "name": "example-flow",
  "entrypoint": "start",
  "metadata": { "owner": "team-a" },
  "nodes": [
    {
      "id": "start",
      "step_type": "start",
      "name": "Start",
      "properties": {},
      "next_node": "agent"
    },
    {
      "id": "agent",
      "step_type": "agent",
      "name": "Agent Step",
      "properties": {
        "agent_ref": "/agents/support-bot",
        "tool_mode": "auto",
        "output_validation": { "mode": "schema", "schema_ref": "/schemas/ticket" }
      },
      "next_node": "notify"
    },
    {
      "id": "notify",
      "step_type": "message",
      "name": "Notify User",
      "properties": {
        "message_type": "system_notification",
        "recipient_id": "user-123",
        "body_template": { "text": "Your ticket is ready." },
        "wait_for_status": ["processed"]
      }
    }
  ],
  "edges": []
}
```

### Common Node Fields

- `id`: unique step ID.
- `step_type`: ROM step type (see below).
- `name`: display name for designer.
- `properties`: step-specific configuration.
- `children`: nested nodes for container steps (optional).
- `next_node`: default next node (sequential flows).
- `on_error`: error handling policy (stop, skip, continue, route).
- `retry`: retry policy (count, backoff, retry_on).
- `timeout_ms`: per-step timeout.

### Edges (optional)

Edges are used for visual routing and branching:
- `from`, `to`
- `label` (e.g., "yes", "no")
- `condition` (REL expression)

When edges are not provided, the runtime may generate edges from `next_node`.

## ROM Step Types

ROM step types are native and mapped to `raisin-flow` runtime handlers.

### start / end
No properties. `start` must be reachable from `entrypoint`.

### function
Executes a `raisin:Function` with optional mapping and compensation.
Properties:
- `function_ref` (Reference)
- `input_mapping`, `output_mapping`
- `timeout_ms`, `max_retries`
- `compensation_ref`
- `guardrails` (optional)

### agent
Executes a `raisin:AIAgent` with streaming and validation.
Properties:
- `agent_ref` (Reference)
- `conversation_ref` (optional)
- `tool_mode`: auto | explicit | hybrid
- `toolsets` or `tool_preparer` (optional)
- `history_processors` (optional)
- `output_validation` (schema + validators)
- `retry_policy` (self-correction)
- `model_fallbacks` (optional)

### message
Creates a `raisin:Message` and waits for a status transition.
Properties:
- `message_type` (String)
- `recipient_id` or `recipient_ref`
- `sender_id` (optional; default: system)
- `body_template` (Object)
- `wait_for_status` (Array, default: ["processed", "completed"])
- `timeout_ms` (optional)
- `on_timeout` (route or escalation)

### decision
Routes based on REL expressions.
Properties:
- `condition` (String) or `branches` (Object of condition -> target)
- `default_target`

### parallel
Starts multiple child branches.
Properties:
- `join_policy`: all | quorum
- `quorum` (when join_policy is quorum)

### join
Synchronizes parallel branches.
Properties:
- `expected` (number of branches)
- `timeout_ms`

### wait
Pauses until a condition is met.
Properties:
- `wait_type`: time | event | message | external
- `duration_ms` (for time)
- `target_path` and `expected_event` (for event)
- `message_id` or `message_type` (for message)

### subflow
Invokes another `raisin:Flow`.
Properties:
- `flow_ref`
- `input_mapping`, `output_mapping`

### loop
Repeats child steps until a REL condition fails.
Properties:
- `condition`
- `max_iterations`

### container
Visual grouping for nodes. No execution semantics by itself.

### custom
Extension hook for plugin-defined step types.

## Execution Semantics (raisin-flow contract)

- Execute synchronously until an async boundary is reached.
- Async boundaries persist `raisin:FlowInstance` and return a wait state.
- Resume on message status change, tool result, timer, or event trigger.
- OCC and idempotency are required for reliability.
- Step results update `variables` and emit step-level events.

### Wait Types (FlowInstance.wait_info)

ROM standard wait types:
- `message` - waiting on a `raisin:Message` status change.
- `tool_call` - waiting on tool execution result.
- `function_call` - waiting on function job result.
- `event` - waiting on a matching event from triggers.
- `scheduled` - waiting on a timer.
- `join` - waiting on parallel branches.

## Messaging Integration

Messaging is the default human I/O channel:
- MessageStep creates a message in the sender's outbox.
- Router trigger delivers to inbox and emits notifications.
- Flow resumes when message status reaches `processed` or `completed`.
- Approvals and handoffs are message types, not special cases.

Standard message types for ROM:
- `task_assignment`
- `approval_request`
- `handoff_request`
- `system_notification`

## Agent Capabilities (ROM-native)

ROM requires these agent features, derived from `docs/AI_ENHANCEMENTS.md`:
- Streaming output events and partial node updates.
- Output validation strategies and custom validators.
- Self-correction (model retry) policies.
- Dynamic tool selection and tool prepare functions.
- History processors and context window control.
- Model fallback chains and error-based routing.
- Guardrails on input, output, and tools.

## Observability and Events

ROM defines canonical flow events:
- `raisin.flow.started`
- `raisin.flow.waiting`
- `raisin.flow.completed`
- `raisin.flow.failed`
- `raisin.flow.step.started`
- `raisin.flow.step.completed`
- `raisin.flow.step.failed`
- `raisin.flow.message.created`
- `raisin.flow.message.resumed`

Events should be emitted to WebSocket/SSE for live UI updates.

## Visual Designer Contract

The designer must:
- Author ROM `workflow_data` with validation.
- Provide palettes for all ROM step types.
- Display and edit step properties with schemas.
- Surface validation errors (missing refs, invalid REL, cycles).
- Support layout metadata without affecting execution semantics.

## Versioning

ROM is versioned via `workflow_data.rom_version`.
- Minor changes may be backwards compatible.
- Major changes require migration rules in `raisin-flow`.
