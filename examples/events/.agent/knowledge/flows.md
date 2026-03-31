# RaisinDB Flows Reference

## Overview

Flows are multi-step workflows defined in YAML with `node_type: raisin:Flow`. They chain together function calls, AI interactions, decisions, and human tasks.

## Flow Structure

```yaml
node_type: raisin:Flow
properties:
  title: My Workflow
  name: my-workflow
  description: Processes incoming requests
  enabled: true
  workflow_data:
    version: 1
    error_strategy: fail_fast    # or: continue, compensate
    nodes:
      - id: start
        node_type: "raisin:FlowStep"
        properties:
          step_type: start
        connections:
          - target: process
      - id: process
        node_type: "raisin:FlowStep"
        properties:
          step_type: function_step
          function_ref: /lib/myapp/process-data
        connections:
          - target: end
      - id: end
        node_type: "raisin:FlowStep"
        properties:
          step_type: end
```

## Step Types

| Type | Description |
|------|-------------|
| start | Entry point of the flow |
| end | Terminal node |
| function_step | Execute a server-side function |
| decision | Branch based on a condition |
| chat | Interactive multi-turn AI chat session |
| ai_container | AI agent with tool loop (agentic) |
| agent_step | Single-shot AI call (no tool loop) |
| human_task | Wait for human approval or input |
| wait | Pause for a duration or external event |
| parallel | Fork into concurrent branches |
| join | Synchronize parallel branches |
| sub_flow | Execute another flow as a step |
| loop | Iterate over items |

## Function Step

```yaml
- id: run-fn
  node_type: "raisin:FlowStep"
  properties:
    step_type: function_step
    function_ref: /lib/myapp/my-function
    input_mapping:
      user_id: "$.trigger.properties.user_id"
    output_mapping:
      result: "$.steps.run-fn.output"
  connections:
    - target: next-step
```

## Decision Step

```yaml
- id: check-status
  node_type: "raisin:FlowStep"
  properties:
    step_type: decision
    condition: "$.steps.run-fn.output.approved == true"
  connections:
    - target: approve-step
      label: "yes"
    - target: reject-step
      label: "no"
```

## Chat Step (Multi-turn AI Conversation)

```yaml
- id: chat-session
  node_type: "raisin:FlowStep"
  properties:
    step_type: chat
    action: Support Chat
    agent_ref: /agents/support-agent
    system_prompt: |
      You are a helpful support agent. Assist the user with their request.
    max_turns: 10
    conversation_format: inbox   # or omit for default ai_chat
    termination:
      allow_user_end: true
      allow_ai_end: true
      end_keywords: []
  connections:
    - target: end
```

## AI Container (Agentic Tool Loop)

```yaml
- id: ai-agent
  node_type: "raisin:FlowContainer"
  container_type: ai_sequence
  ai_config:
    agent_ref: /agents/my-agent
    tool_mode: auto              # auto | explicit | hybrid
    max_iterations: 10
    thinking_enabled: false
    timeout_ms: 30000
    total_timeout_ms: 300000
  children: []
```

## Human Task Step

```yaml
- id: approval
  node_type: "raisin:FlowStep"
  properties:
    step_type: human_task
    task_type: approval          # approval | input | review | action
    title: Approve Publication
    description: Review and approve this article for publishing
    assignee: /users/editor
    priority: 3
    options:
      - value: approved
        label: Approve
      - value: rejected
        label: Reject
  connections:
    - target: next-step
```

## Wait Step

```yaml
- id: delay
  node_type: "raisin:FlowStep"
  properties:
    step_type: wait
    duration_ms: 60000           # wait 60 seconds
  connections:
    - target: next-step
```

## Connections

Each node uses `connections` to define outgoing edges:

```yaml
connections:
  - target: next-node-id              # simple flow
  - target: yes-node-id               # decision branch
    label: "yes"
  - target: no-node-id
    label: "no"
```

## Error Strategies

| Strategy | Behavior |
|----------|----------|
| fail_fast | Stop flow on first error |
| continue | Skip failed step, continue to next |
| compensate | Run compensation functions to rollback |

## File Location

Flows live in `package/content/functions/flows/<name>/.node.yaml` and can be triggered by a trigger's `flow_path` or started via the API.
