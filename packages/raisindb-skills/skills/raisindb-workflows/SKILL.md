---
name: raisindb-workflows
description: "Build durable RaisinDB workflows (raisin:Flow): designer format, step types, and/or/parallel/loop containers, human approval tasks in the inbox, AI agent steps, saga compensation, retries and timeouts. Use this whenever the user wants a workflow, flow, approval process, multi-step automation, human-in-the-loop process, 'ask people one by one', scheduled/durable orchestration, or mentions raisin:Flow, flow designer, inbox tasks, or fill-shift-style coordination — even if they don't say the word 'workflow'."
---

# RaisinDB Workflows

A workflow is a `raisin:Flow` node whose `workflow_data` property holds the
flow definition in **designer format** — the same format the admin console's
visual designer reads and writes. The engine lowers it to runtime steps;
functions, AI agents, and humans are uniform step concepts; the inbox is the
human-in-the-loop primitive.

**Working reference apps** (read these before inventing anything):
- `examples/shiftboard/package/content/functions/flows/fill-shift/.node.yaml`
  — candidates → loop → human approval task → assign → notify (the canonical
  "ask people one by one until someone accepts" pattern)
- `examples/workflows/event-ticketing/run.mjs` — saga compensation, OR
  routing, approval gate, full SDK run+inbox client code

**MANDATORY after editing any flow YAML**: run the flow doctor before
deploying — it catches the template/REL traps listed below:

    npx raisindb flow doctor <path-to-package-or-flow>

Package validation runs these same checks automatically: `raisindb package
validate ./package` is the final gate before a `.rap` is built (and
`package create` / `deploy` refuse to build when a flow has doctor errors).
Use `flow doctor` as the fast, focused loop while editing a single flow.

## Anatomy (designer format)

No start/end nodes — array order is execution order; the engine injects them.

```yaml
node_type: "raisin:Flow"
properties:
  name: "fill-shift"
  title: "Fill Shift"
  enabled: true
  workflow_data:
    version: 1
    error_strategy: fail_fast        # or continue
    nodes:
      - id: pick_candidates          # snake_case! (see REL pitfalls)
        node_type: raisin:FlowStep
        properties:
          action: "Pick candidates for {{ input.shift_path }}"
          function_ref: /lib/myapp/pick-candidates
          arguments:
            shift_path: "${input.shift_path}"
      - id: gate
        node_type: raisin:FlowContainer
        container_type: or
        rules:
          - { condition: "steps.pick_candidates.count > 0", next_step: ask }
        children: [ ... ]
```

## Step types

| step_type | Purpose |
|---|---|
| *(default)* + `function_ref` | Run a server function; output under `steps.<id>.*` |
| `human_task` | Create a `raisin:InboxTask` in the assignee's inbox; flow WAITS |
| `ai_agent` | One LLM answer; the agent's own tools run in a bounded internal loop |
| `chat` | Long multi-turn conversation step (experimental) |

### Human tasks (the inbox primitive)

```yaml
- id: ask_candidate
  node_type: raisin:FlowStep
  properties:
    step_type: human_task
    task_type: approval                       # approval | input | review | action,
                                              # or your OWN slug ([a-z][a-z0-9_-]*)
    assignee: "${candidate.user_path}"        # user home path, e.g. /users/internal/anna-at-example-com
    action: "Can you take {{ steps.pick_candidates.shift_title }}?"
    task_description: "Details: {{ steps.pick_candidates.day }} {{ steps.pick_candidates.start }}"
    options:
      - { value: accept,  label: "I'll take it", style: success }
      - { value: decline, label: "Can't make it", style: danger }
    due_in_seconds: 86400
    timeout_edge: next_step_id                # where to go if nobody answers
```

- The task node lands at `{assignee}/inbox/task-{step}-{instance}-it{N}` —
  per-instance, per-loop-iteration. Clients complete it via
  `POST /api/inbox/{repo}/tasks/{task_id}/complete` with `{response:{value}}`
  (or the SDK `InboxApi`). The step output is the completion response merged
  with `completed_by` — reference it as `steps.ask_candidate.action` etc.
- Assignee may be an AI AGENT path — the agent answers the task with a
  structured decision and a confidence; `min_confidence` +
  `escalation_assignee` route low-confidence answers to a human.
- Task UIs are just node UIs: subscribe to `{home}/inbox/**` and render the
  task node's own `options` as buttons (see `examples/shiftboard/frontend/
  src/lib/components/TaskPanel.svelte`).

## Containers

| container_type | Semantics |
|---|---|
| `and` | Children run as a sequence |
| `or` | REL `rules` evaluated in order; first match routes to its child; no match → container skipped. Add a `router: {agent_ref...}` and an AI agent picks the branch when no rule matched |
| `parallel` | Children as parallel branch flows; add `fan_out` for one branch PER COLLECTION ITEM (below) |
| `loop` | Iterate a collection (below) |
| `ai_sequence` | Agentic tool loop (agent + tools until done) |
| `competition` | Several agents answer; a referee judges/refines |

### Loop (ask-one-by-one, batch processing)

```yaml
- id: ask_each
  node_type: raisin:FlowContainer
  container_type: loop
  loop:
    over: "${steps.pick_candidates.candidates}"   # required, collection expr
    item: candidate                               # exposed as a flow variable
    index: candidate_index                        # optional
    max_iterations: 10                            # optional cap
    until: 'steps.ask_candidate.action == "accept"'  # optional early exit,
                                                  # evaluated AFTER each iteration
  children: [ ...body steps reference ${candidate.*}... ]
```

Output: `steps.ask_each.results` (array, index-aligned with the collection)
plus `count`. To find *which* iteration succeeded, add a small function step
that zips `${steps.ask_each.results}` with the original collection — see
`resolve-accepter` in the shiftboard package.

### Fan-out (ask everyone AT ONCE, then join)

A loop asks one person at a time. To ask everyone concurrently and wait for
all of them, use a `parallel` container with `fan_out` — the children become
ONE branch subgraph run per item, and the container joins every run:

```yaml
- id: ask_all
  node_type: raisin:FlowContainer
  container_type: parallel
  fan_out:
    over: "${steps.pick_candidates.candidates}"   # required, collection expr
    max_branches: 100                             # optional cap (default 500)
  merge_strategy: all_success                     # or merge_all | first_success
  children: [ ...branch steps reference ${item.*} and ${index}... ]
```

- Each branch is its own child flow instance, so a branch may park on a
  human task — the join resumes once the LAST person has answered.
- `item` / `index` are bound per branch; the branch's flow input is
  `{ item, index }`.
- Iterate `steps.ask_all.branches` — each entry carries `branch_id`,
  `status`, `output`. Positional `steps.ask_all.branch_0` also works.

Loop vs fan-out: **loop** = sequential, can early-exit with `until` (ask
until someone accepts). **fan-out** = concurrent, always waits for all
(collect every answer).

## Templates and data flow

- `{{ expr }}` interpolates into strings; **whole-string** `${expr}` keeps
  the native JSON type (numbers, arrays, objects):
  `quantity: "${input.quantity}"` stays a number.
- Namespaces: `input.*` (flow input), `steps.<id>.*` (step outputs),
  `trigger.*`, loop item vars.

### REL pitfalls (these WILL bite you)

1. **snake_case step ids.** REL identifiers are `[A-Za-z0-9_]` only —
   `steps.create-accounts.x` parses as a SUBTRACTION. The doctor flags this.
2. Bracket access exists but is NOT null-safe. Prefer snake_case ids.
3. Conditions are full REL: `steps.reserve.total_price > 500`,
   `input.tier == "vip"`.

## Errors, retries, timeouts, compensation

- **Two independent retry layers**: the queued function-execution JOB retries
  ~3x on a fixed ~10s/30s backoff regardless of step config; then the FLOW
  retries the step (its own `retry_strategy`/`retry_base_delay_ms`).
  Consequence: a step's `timeout_ms` must OUTLIVE the job retry schedule
  (~40s+), or the wait expires first and compensation never runs.
- **Saga compensation**: give a step `compensation_ref` +
  `compensation_input_mapping` (`${output.reservation_id}` refers to that
  step's own output). On a later unrecoverable failure, compensations run
  LIFO. `failed` is transient (~1-2s) before `rolled_back`.
- `timeout_edge` on human tasks → "nobody answered" path instead of failure.
- `continue_on_fail: true` for best-effort steps (e.g. notifications).

## AI in flows (summary — details in the website workflows guide)

- `ai_agent` step: `agent_ref`, templated `prompt`, optional
  `include_context: "input" | "full"` (appends workflow state as JSON),
  `response_format` for structured output. The agent's own tools execute in a
  bounded internal loop.
- Agents can also START flows: give an agent a tool function that calls
  `raisin.flows.run('/flows/my-flow', input)` — chat becomes the interface,
  the workflow does the durable part (see `start-shift-fill` in shiftboard).

## Deploy, run, test

```bash
# Fast focused check while editing (templates, REL, container/loop config...)
npx raisindb flow doctor ./package

# Final gate: full package validation (WASM schema + flow doctor).
# `package create` and `deploy` run this automatically and abort on errors.
npx raisindb package validate ./package

# Deploy with the package (flows are content like everything else)
raisindb deploy ./package --repo myrepo --install

# Run + inspect
POST /api/flows/{repo}/run         {"flow_path":"/flows/fill-shift","input":{...}}
GET  /api/inbox/{repo}             # pending tasks for the caller
POST /api/inbox/{repo}/tasks/{id}/complete   {"response":{"value":"accept"}}
```

- SDK: `db.flow.run(...)` / `FlowClient`, `InboxApi`; stream execution events
  via SSE (`flow_waiting` tells you a human task is pending).
- Test runs support **mocked functions and agents** (`mock_functions`,
  `mock_agents` in test config) so flows are testable without side effects.
- The admin console has the visual designer (Functions → flows → open) and a
  live instance diagram (Flow Instances → row → Diagram) that highlights the
  current step of a run.

## Choosing chat-agent coordination vs a workflow

Free-form agent chat (see the `raisindb-messaging-agents` skill) is flexible
but its state lives in conversation history — no timeouts, no audit, and the
LLM can act on stale context. Re-model the same coordination as a flow when
you need durability, deadlines/escalation, an audit trail, or button-based
responses. They compose: a flow step can be an agent; an agent tool can start
a flow.
