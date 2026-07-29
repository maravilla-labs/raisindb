# Authoring RaisinDB Workflows (`raisin:Flow`)

A practical guide for human developers and AI agents authoring workflows —
either as YAML content files (`.node.yaml` in packages) or via the visual
flow designer in the admin console. Everything documented here is verified
against the engine source:

- Designer format types: `crates/raisin-flow-runtime/src/types/designer_format/types.rs` and `config_types.rs`
- Lowering semantics: `crates/raisin-flow-runtime/src/types/designer_format/conversion.rs`
- Runtime behavior: `crates/raisin-flow-runtime/src/handlers/`
- TypeScript types: `packages/raisin-client-js/src/types/flow-definition.ts`
- Executable examples: `crates/raisin-flow-runtime/tests/e2e_flows.rs`, `examples/workflows/`

Functions, AI agents, and humans are **uniform step concepts**: a function
step queues a function, an agent step calls an AI agent, and a human task
pauses for a person — or for an AI agent acting as the assignee, with
automatic escalation back to a human.

---

## 1. The flow node

A workflow is a `raisin:Flow` node in the **functions** workspace. Its
`workflow_data` property holds the flow definition. As a package content
file (`.node.yaml`):

```yaml
node_type: raisin:Flow
properties:
  title: Order Approval            # display title
  name: order-approval             # node name (path segment)
  description: Approves incoming orders
  enabled: true
  workflow_data:
    version: 1                     # schema version (currently 1)
    error_strategy: fail_fast      # fail_fast (default) | continue
    nodes:
      - id: approve
        node_type: raisin:FlowStep
        properties:
          action: Approve order {{ input.order_id }}
          step_type: human_task
          task_type: approval
          assignee: /users/manager
          options:
            - { value: approve, label: Approve, style: success }
            - { value: reject,  label: Reject,  style: danger }
```

(A real example: `builtin-packages/ai-tools/content/functions/flows/ai-agent-handler/.node.yaml`.)

### The designer format is canonical

`workflow_data` in the **designer format** is the canonical authoring
format — the same format the visual designer reads and writes:

- Every node is `node_type: raisin:FlowStep` or `node_type: raisin:FlowContainer`.
- There are **no explicit `start` / `end` nodes** — the engine injects them.
- There is **no `next_node` chaining** — execution order is the **array
  order** of `nodes` (and of each container's `children`). The engine
  lowers the tree into a flat graph, chaining each node to the next sibling
  and the last node to the injected `end`.

The engine auto-detects the format of `workflow_data`: nodes with
`node_type` are designer format; nodes with `step_type` at the top level
are the lower-level *runtime format* (see Appendix A). Don't mix the two in
one definition.

Top-level `workflow_data` fields (designer format):

| Field | Type | Notes |
|---|---|---|
| `version` | number | Defaults to `1`. |
| `error_strategy` | `fail_fast` \| `continue` | Defaults to `fail_fast`. `continue` lowers to `continue_on_fail` on every work step (function/agent/AI container) that has no explicit error handling of its own; per-step settings (§5) always win. |
| `timeout_ms` | number, optional | Global timeout. Preserved in the lowered flow's metadata but not yet enforced as a total-instance deadline (per-step and wait timeouts ARE enforced). |
| `nodes` | array | Root steps/containers, executed in order. |

---

## 2. Steps (`raisin:FlowStep`)

Common shape:

```yaml
- id: my-step                # unique within the flow; referenced by rules/edges and steps.<id>.*
  node_type: raisin:FlowStep
  properties: { ... }        # see below per step kind
  on_error: stop             # optional: stop | skip | continue
  error_edge: error-handler  # optional: node id to jump to on failure (also allowed inside properties)
```

How the engine decides what a step is (`conversion.rs::determine_step_type`),
in priority order:

1. `step_type: human_task` (or any `task_type` present) → human task
2. `function_ref` present → function step
3. `step_type: ai_agent` → single-shot agent step
4. `agent_ref` present without `step_type: ai_agent` → full AI container (backward compat: tool loop + conversation persistence)
5. `step_type: chat` → chat step
6. `step_type: wait` | `sub_flow` | `decision` → that step type (§2.6)
7. `condition` present → decision step
8. otherwise → function step (which then fails for missing `function_ref`)

### 2.1 References (`function_ref`, `agent_ref`, `compensation_ref`)

A reference is either a **plain path string** (hand-authoring convenience;
workspace defaults to `functions`) or the **full reference object**:

```yaml
# Both are accepted and equivalent:
function_ref: /lib/charge-payment

function_ref:
  raisin:ref: /lib/charge-payment
  raisin:workspace: functions
  raisin:path: /lib/charge-payment    # optional resolved path; wins if present
```

### 2.2 Function step

Queues a RaisinDB function via the job system; the flow pauses
(`Wait`/`function_call`) and resumes when the function completes.

```yaml
- id: charge
  node_type: raisin:FlowStep
  properties:
    action: Charge the card           # display label
    function_ref: /lib/charge-payment
    arguments:                        # template expressions resolved against the flow context
      order_id: "{{ input.order_id }}"
      amount: "${input.amount}"       # whole-string expression keeps the native JSON type (number)
      note: "Order {{ input.order_id }} for {{ input.customer }}"   # interpolation -> string
    timeout_ms: 30000                 # optional wait deadline for the function execution
    retry:                            # optional explicit retry config
      max_retries: 2
      base_delay_ms: 1000
      max_delay_ms: 10000
    compensation_ref: /lib/refund-payment   # saga rollback (see §5.3)
    continue_on_fail: false
```

Step output = the function's `result` value, available downstream as
`steps.charge.*`. With no `arguments`, an empty object is sent.

Other supported function-step properties: `disabled` (bool, default false),
`isolated_branch` (bool — run in an isolated git-like branch),
`execution_identity` (`agent` | `caller` | `function`, default `agent` —
which identity the permission check uses), `payload_key`, `lua_script`
(reserved).

### 2.3 AI agent step (`step_type: ai_agent`)

A lightweight AI call: one agent, one answer, no conversation
persistence. **The agent's own tools work**: tools configured on the
agent node are executed in a bounded internal loop (default 5
iterations, `max_tool_iterations` to change). Use `ai_sequence` (§3.4)
when you need workflow-level tools, explicit tool steps, or
orchestration — the mental model: *an agent's tools travel with the
agent; an `ai_sequence`'s children are additional workflow tools
layered on top.*

```yaml
- id: summarize
  node_type: raisin:FlowStep
  properties:
    action: Summarize request
    step_type: ai_agent
    agent_ref: /agents/summarizer
    prompt: "Summarize this refund request: {{ input.reason }} ({{ input.amount }} CHF)"
```

- `prompt` is template-resolved. Without a `prompt`, the handler falls back
  to the triggering content (`input.event.node_data.properties.content`,
  then `input.message`, then `input.input`).
- **`include_context`** (`"input"` | `"full"` | `true`): appends the
  workflow context as a JSON block to the prompt — `"input"` = the flow
  input, `"full"` = input + all step outputs + trigger info + flow
  variables. The agent then sees the workflow state without you
  templating each field. Templates remain the *precise* mechanism (and
  the token-efficient one); `include_context` is the *broad* one. The
  third path is **pull**: give the agent node-read tools and let it
  fetch details itself.
- Output shape: `{ response, model, finish_reason, usage }` — reference the
  text downstream as `{{ steps.summarize.response }}`. When tools ran,
  `tools_used` (name, function_ref, error) and `tool_iterations` are
  included for auditing.
- `response_format` (designer + runtime format) requests structured
  output; parsed JSON lands in `structured_output`.

> **Note:** `ai_agent` steps and `ai_sequence` containers chain normally in
> designer format — the handlers read the node-level `next_node` link the
> converter emits (with the legacy `next_node` *property* still honored for
> runtime-format flows).

### 2.4 Human task (`step_type: human_task`)

Creates an inbox task and pauses the flow until it is completed (§6).

```yaml
- id: approve
  node_type: raisin:FlowStep
  properties:
    action: "Approve order {{ input.order_id }}"   # becomes the TASK TITLE
    step_type: human_task
    task_type: approval                 # approval | input | review | action, or
                                        # any application-defined slug (see below)
    assignee: /users/manager            # user path OR agent path; templates allowed
    task_description: "Please review order {{ input.order_id }}."  # becomes description
    priority: 4                         # 1-5 (5 = highest); default 3; templates allowed
    due_in_seconds: 86400               # due time AND wait deadline; templates allowed
    timeout_edge: escalate-step         # where to go if the deadline expires
    options:                            # for approval tasks
      - { value: approve, label: Approve, style: success }
      - { value: reject,  label: Reject,  style: danger }
    # input tasks instead use:
    # input_schema: { type: object, properties: { quantity: { type: number } } }
    # agent-assignee controls (see §6.3):
    # min_confidence: 0.75
    # escalation_assignee: /users/boss
```

Notes:

- **The designer's `action` label doubles as the task title** (the runtime
  requires `title`; the converter maps `action` → `title`, defaulting to
  `"Task"`). `task_description` maps to the task's `description`.
- **`task_type` is an OPEN set.** `approval | input | review | action` are
  the types the runtime understands semantically (approval options, input
  schemas), but any slug matching `[a-z][a-z0-9_-]{0,63}` is accepted and
  carried through to the task node verbatim, so an application can define
  its own task vocabulary without an engine change. Only the slug's SHAPE is
  validated — an invalid one is a step configuration error.
- **Every property is template-resolved, including the numeric ones.**
  Title, description, assignee, option labels, `data`, *and*
  `due_in_seconds` / `priority` (e.g. `due_in_seconds: "${input.sla_seconds}"`)
  resolve against the flow context before the task is created; a resolved
  numeric string is coerced to a number. A value that resolves to something
  non-numeric is a configuration error rather than a silently missing
  deadline; an *unresolvable* expression leaves the task with no deadline
  at all.
- `options[*].style` is free-form; the UI understands
  `default | success | danger | warning`.
- `due_in_seconds` materializes an absolute `due_at` on the task and is
  also the flow's wait deadline. On expiry the task is marked `expired`;
  with `timeout_edge` the flow continues at that node, **without** a
  `timeout_edge` the flow fails.
- The completed response is exposed to downstream steps as
  `__human_response` (see §6.2).

### 2.5 Chat step (`step_type: chat`)

A long-running, multi-turn conversation step. The flow waits
(`chat_session`) between turns; conversation history is persisted as nodes
(no in-memory window).

```yaml
- id: chat-session
  node_type: raisin:FlowStep
  properties:
    step_type: chat
    action: Chat Session
    chat_config:
      agent_ref: /agents/support        # primary agent (null = resolved from context)
      system_prompt: "You are a helpful support agent."   # optional
      max_turns: 50                     # default 50
      session_timeout_ms: 600000        # how long to wait for the user each turn
      handoff_targets:                  # optional sub-agent delegation
        - agent_ref: /agents/billing
          description: "Billing and invoice questions"
          condition: "input.topic == \"billing\""   # optional REL expression
      termination:
        allow_user_end: true            # user keyword can end the session (default true)
        allow_ai_end: true              # AI may declare the session complete (default true)
        end_keywords: ["goodbye", "exit"]
```

Behavior (from `handlers/chat_step/`):

- Each user message increments the turn counter; reaching `max_turns`
  completes the step (`completion_reason: max_turns_reached`).
- `end_keywords` are matched case-insensitively against the user message
  when `allow_user_end` is true.
- `allow_ai_end` lets the agent end the session (the AI turn signals
  `end_session`).
- See `builtin-packages/ai-tools/content/functions/flows/chat/.node.yaml`
  for the canonical chat flow shipped with the ai-tools package.

### 2.6 Wait, sub-flow, and decision steps

Three more step types, all authorable in the designer format.

**`step_type: wait`** — pause for time, an event, or a cron point:

```yaml
- id: cool-off
  node_type: raisin:FlowStep
  properties:
    action: Cool-off period
    step_type: wait
    wait_type: delay            # delay (default) | until | event | cron
    duration: "30m"             # for delay; templates allowed
    # until: "${input.publish_at}"      # for until
    # event_type: order.shipped         # for event, with optional `timeout`
    # cron: "0 9 * * 1"                 # for cron
```

**`step_type: sub_flow`** — run another deployed flow as one step. The
child's output becomes `steps.<id>.*` in the parent:

```yaml
- id: settle
  node_type: raisin:FlowStep
  properties:
    action: Settle the order
    step_type: sub_flow
    flow_ref: /flows/settle-order      # path or full reference object
    input_mapping:
      order_id: "${input.order_id}"
    async: true                        # optional
```

**`step_type: decision`** — a two-way branch on a REL condition, with both
arms named explicitly. Name only the arm that diverges; the other falls
through to the next sibling:

```yaml
- id: big-order
  node_type: raisin:FlowStep
  properties:
    action: Big order? skip the discount
    step_type: decision
    condition: "input.amount > 500"
    yes_branch: invoice          # jump PAST the discount step
    # no_branch defaults to the next sibling (discount)
- id: discount
  node_type: raisin:FlowStep
  properties: { action: Apply discount, function_ref: /lib/discount }
- id: invoice
  node_type: raisin:FlowStep
  properties: { action: Invoice, function_ref: /lib/invoice }
```

A step carrying a bare `condition` and no `step_type` is also a decision
(rule 7 above); it behaves the same way.

> **Mind the fallthrough.** Designer siblings chain in **array order**, so a
> free-standing decision is naturally a **guard** that skips forward over
> steps — as above, where `discount` is skipped and both paths land on
> `invoice`. It does NOT give you two mutually exclusive arms that rejoin
> afterwards: if you point `yes_branch` at a later sibling, the steps in
> between still fall through into it when the condition is false.
>
> For genuinely exclusive branches use an **`or` container** (§3.2). Its
> children each continue to the container's successor instead of into each
> other, which is exactly the "one of these, then carry on" shape. Don't
> emulate either one with two sibling containers carrying complementary
> conditions — that has no single point of decision, and both arms can
> mis-evaluate independently.

---

## 3. Containers (`raisin:FlowContainer`)

```yaml
- id: my-container
  node_type: raisin:FlowContainer
  container_type: and          # and | or | parallel | ai_sequence | competition | loop
  children: [ ...nodes... ]
  rules: [ ... ]               # 'or' containers only
  ai_config: { ... }           # 'ai_sequence' containers only
  loop: { ... }                # 'loop' containers only
  timeout_ms: 60000            # optional
```

### 3.1 `and` — all children, sequentially

Children run in array order; the flow then continues after the container.
Children can reference each other's outputs (`steps.<child-id>.*`).

```yaml
- id: book-everything
  node_type: raisin:FlowContainer
  container_type: and
  children:
    - id: book-flight
      node_type: raisin:FlowStep
      properties: { function_ref: /lib/book-flight }
    - id: book-hotel
      node_type: raisin:FlowStep
      properties:
        function_ref: /lib/book-hotel
        arguments: { near_flight: "${steps.book-flight.arrival_airport}" }
```

### 3.2 `or` — REL-routed, exactly one child

Rules are evaluated **in order**; the first matching rule routes to its
child, that child runs, then execution **exits the container** (other
children are skipped). If **no rule matches, the whole container is
skipped** and the flow continues after it.

```yaml
- id: route-by-tier
  node_type: raisin:FlowContainer
  container_type: or
  rules:
    - { condition: "input.tier == \"premium\"", next_step: vip }
    - { condition: "input.tier == \"basic\"",   next_step: standard }
  children:
    - id: vip
      node_type: raisin:FlowStep
      properties: { function_ref: /lib/vip-handling }
    - id: standard
      node_type: raisin:FlowStep
      properties: { function_ref: /lib/standard-handling }
```

Conditions are REL expressions over the same namespaces as templates (§4).
If `rules` is omitted, each child's own `condition` property is used as its
rule. With neither, the container passes through to its first child.

#### AI-routed `or`: the agent decides the branch

Add a `router` and an **agent picks the child** — deterministic REL
rules always run first; the agent decides only when none matched (or
when there are no rules at all). The agent receives your routing
instructions plus the list of branches (each child's id and `action`
text) and must answer with structured output
`{ branch, reasoning, confidence }` — `branch` is schema-constrained to
the declared child ids, so the model can never route to an invented
target.

```yaml
- id: route
  node_type: raisin:FlowContainer
  container_type: or
  rules:                                    # optional - deterministic first
    - { condition: "input.amount > 10000", next_step: escalate }
  router:
    agent_ref: /agents/dispatcher
    prompt: "Order from {{ input.customer }} for {{ input.amount }} CHF. Route it."
    min_confidence: 0.6                     # below -> default_branch / skip
    default_branch: standard                # omit to skip the container instead
    # include_context: full                 # append workflow state as JSON (§2.3)
  children:
    - id: escalate
      node_type: raisin:FlowStep
      properties: { action: Escalate to ops, function_ref: /lib/escalate }
    - id: vip
      node_type: raisin:FlowStep
      properties: { action: VIP handling, function_ref: /lib/vip }
    - id: standard
      node_type: raisin:FlowStep
      properties: { action: Standard handling, function_ref: /lib/standard }
```

The router's decision is recorded as a step output (id
`<container>__router`, or the container id when there are no rules):
`{ routed_to, routed_by_agent, reasoning, confidence }` — downstream
steps can reference and audit it. Typical use: a node-event trigger
starts the flow and the agent routes by looking at the changed node in
`{{ input }}`.

### 3.3 `parallel` — fork branches, join them

Each branch becomes its own child flow instance; the container waits until
**every** branch reaches a terminal state, then joins their outputs with the
configured `merge_strategy`:

| `merge_strategy` | Behaviour |
|---|---|
| `merge_all` (default) | Continue with every branch result, successful or not |
| `first_success` | Continue with the first successful branch; fail if all failed |
| `all_success` | Fail the container if any branch failed |

Because the join waits for terminal children rather than polling, a branch
that itself parks on a human task is joined correctly — the container
resumes once the last person has answered.

**Static branches** — the children ARE the branches, a fixed set:

```yaml
- id: par
  node_type: raisin:FlowContainer
  container_type: parallel
  merge_strategy: merge_all
  children:
    - id: left
      node_type: raisin:FlowStep
      properties: { function_ref: /lib/left }
    - id: right
      node_type: raisin:FlowStep
      properties: { function_ref: /lib/right }
```

**Dynamic fan-out** — `fan_out` turns the children into ONE branch subgraph
run once per item of a runtime collection, so the branch COUNT comes from
data rather than from the flow. This is how "one task per row, then join"
is expressed:

```yaml
- id: collect-approvals
  node_type: raisin:FlowContainer
  container_type: parallel
  fan_out:
    over: "${steps.plan.items}"   # same expression forms as a loop's `over`
    max_branches: 200             # safety cap; defaults to 500
  merge_strategy: all_success
  children:
    - id: approve-item
      node_type: raisin:FlowStep
      properties:
        step_type: human_task
        action: "Approve {{ input.item.name }}"
        task_type: approval
        assignee: "${input.item.owner}"
```

Inside a fan-out branch, `item` and `index` are bound to the current
element and its position; the child flow's input is `{ item, index }`.

The joined output lands in the container's step output two ways:

- positionally — `steps.par.branch_0.status`, `steps.par.branch_0.output`
- as an ordered array — `steps.collect-approvals.branches`, each entry tagged
  with its `branch_id` and `instance_id`. For a fan-out the branch id is the
  only handle on WHICH item produced a result, so this is the form to use.

Branch ids are deliberately not also top-level keys: they would share a
namespace with `branch_0` / `branches`, and REL can't reference an id
containing `-` without bracket access anyway.

A child may itself be a container.

**Runtime format.** The lowered step takes `for_each` (the collection
expression) plus a `branch` template (`{ id?, flow_path | flow_definition,
input_mapping? }`) instead of a `branches` array. A branch may reference a
DEPLOYED flow by `flow_path` — the fan-out then runs one instance of that
flow per item — or carry an inline `flow_definition`. Fan-out width is
bounded by `max_branches`; exceeding it truncates and logs a warning rather
than fanning out without limit.

### 3.4 `ai_sequence` — agentic tool loop

An AI agent runs in a loop: it is called, may request tool calls, the tools
execute, results are fed back, and the loop continues. **The loop ends when
the agent responds without tool calls, or when `max_iterations` is
reached** (the last response is then used as the final answer).

```yaml
- id: assistant
  node_type: raisin:FlowContainer
  container_type: ai_sequence
  ai_config:
    agent_ref: /agents/helper      # or "$auto" to derive from the conversation's agent_ref
    tool_mode: auto                # auto | explicit | hybrid (default auto)
    explicit_tools: []             # tool names exposed as explicit child steps (hybrid mode)
    max_iterations: 10             # default 10
    thinking_enabled: false
    on_error: stop                 # stop | continue | retry
    timeout_ms: 30000              # per-call timeout
    total_timeout_ms: 300000       # across all iterations
    # conversation_ref: <reference>   # continue an existing conversation
  children: []                     # explicit tool steps (explicit/hybrid modes)
```

Tool modes:

- `auto` — the agent's configured tools are executed internally by the loop.
- `explicit` — every tool call appears as an explicit child step.
- `hybrid` — tools listed in `explicit_tools` are explicit; the rest internal.

**Task & context:** the container's initial user message is, in order
of preference: a container-level `prompt` (template-resolved, sibling
of `ai_config`), the triggering node's `content` property, or — for
manual/API runs — the flow input as JSON (so the agent never starts
task-less). `ai_config.include_context` (`"input"` | `"full"` | `true`)
additionally appends the workflow context as a JSON block, exactly like
the agent-step option.

Output: `{ response, iterations, message_count }` under
`steps.<container-id>.*`. `ai_sequence` containers chain normally in
designer format; `ai_config` also accepts `response_format`,
`output_schema`, `max_retries`, and `retry_delay_ms`.

### 3.5 `competition` — competing agents judged by a referee

Every child agent (each potentially backed by a **different LLM**)
answers the same task; a **referee agent** judges the answers and either
accepts a winner or sends per-competitor feedback for another round
(bounded by `max_rounds`, default 1 refinement round). On the final
round the referee must accept. The referee's declared confidence travels
in the step output — gate on it downstream exactly like a human-task
confidence:

```yaml
- id: compete
  node_type: raisin:FlowContainer
  container_type: competition
  prompt: "Write a tagline for {{ input.product }}."   # shared task (templated)
  referee:
    agent_ref: /agents/referee
    min_confidence: 0.7          # below -> output.confident = false
    max_rounds: 2                # refinement rounds after the initial one
    # prompt: optional judging instructions
    # include_context: input     # append workflow state to the shared task (§2.3)
  children:
    - id: writer_claude
      node_type: raisin:FlowStep
      properties: { action: Claude writer, step_type: ai_agent, agent_ref: /agents/writer-claude }
    - id: writer_gpt
      node_type: raisin:FlowStep
      properties: { action: GPT writer, step_type: ai_agent, agent_ref: /agents/writer-gpt }
      # children may override the shared task with their own `prompt`
```

Refinement: the referee answers
`{ action: accept|refine, winner, confidence, feedback: {<competitor>: text} }`
(schema-enforced). On `refine`, only competitors **with feedback**
re-answer — they see their previous answer plus the referee's notes;
the others' answers stand.

Output under `steps.compete.*`:
`{ response, winner, confidence, confident, reasoning, rounds, answers, models }`
— `response` is the winning answer, `answers`/`models` keep every
competitor's final answer and model for auditing. The canonical
low-confidence pattern is a follow-up `or` gate into a human task:

```yaml
- id: confidence-gate
  node_type: raisin:FlowContainer
  container_type: or
  rules:
    - { condition: "steps.compete.confidence < 0.7", next_step: human_review }
  children:
    - id: human_review
      node_type: raisin:FlowStep
      properties:
        step_type: human_task
        task_type: review
        assignee: /users/editor
        action: "Review the AI tagline (referee confidence {{ steps.compete.confidence }})"
```

### 3.6 `loop` — iterate the children over a collection

The container's children form the **loop body** and run once per item of
the collection. The current item is exposed as a flow variable (default
`item`), referenced like any other value: `${candidate}` /
`{{ candidate }}` in templates, bare `candidate` in REL conditions.

```yaml
- id: ask_each_candidate
  node_type: raisin:FlowContainer
  container_type: loop
  loop:
    over: '${steps.pick_candidates.candidates}'   # required: must resolve to an array
    item: candidate                               # default: item (snake_case identifier)
    index: candidate_index                        # optional: 0-based iteration index
    max_iterations: 10                            # optional: cap on processed items
    until: 'steps.ask.response == "accept"'       # optional: early-exit REL condition
  children:
    - id: ask
      node_type: raisin:FlowStep
      properties:
        function_ref: /lib/ask-candidate
        arguments: { who: "${candidate}", position: "${candidate_index}" }
```

Semantics (lowered onto the runtime's `for_each` loop step):

- **`over`** is a template expression evaluated once when the loop starts.
  Arrays iterate item by item; objects iterate as `{key, value}` pairs.
  An **empty collection skips the loop** (output `{results: [], count: 0}`).
  A loop block without `over` is rejected when the definition is parsed.
- **Body** children chain like an `and` container, then loop back for the
  next item. Each iteration sees the fresh `item`/`index` variables; step
  outputs (`steps.ask.*`) hold the **latest** iteration's values.
- **`max_iterations`** caps how many items are processed (safety bound
  for unbounded collections).
- **`until`** is a REL condition evaluated **after each completed
  iteration** — the just-finished iteration's step outputs and loop
  variables are visible. When true, the loop stops early and keeps the
  results collected so far. Waiting steps inside the body (human tasks,
  chats) work: the loop resumes exactly where it left off.
- **Output** under `steps.<loop-id>.*`: `{ results, count }` — `results`
  is the array of per-iteration body outputs, in order. The aggregate is
  also stored as the `<loop-id>_results` flow variable.

The canonical "ask each candidate until one accepts" pattern above stops
asking as soon as `until` fires; downstream steps can branch on
`steps.ask_each_candidate.count` or inspect
`steps.ask_each_candidate.results`.

Loops nest (an inner `loop` container inside the body gets its own
`item` variable — give them distinct names). `raisin flow doctor` checks
loops for a missing `over`, non-identifier `item`/`index` names, an
`until` referencing unknown steps, and unknown roots in body templates.

---

## 4. Template expressions and REL conditions

Anywhere a value supports templates (function `arguments`, agent `prompt`,
human-task title/description/assignee/options, compensation mappings, wait
durations), two marker styles are accepted and equivalent:

- `{{ expr }}`
- `${expr}`

Both are evaluated with **REL** (`raisin-rel`) against the flow context.
Rules (from `runtime/data_mapper.rs`):

- **Whole-string expression keeps its native JSON type.**
  `amount: "${input.amount}"` → a number; `user: "${input.user}"` → the
  whole object. Surrounding whitespace still counts as whole-string.
- **Mixed strings interpolate.** `"Hello {{ input.user.name }}!"` → string;
  non-string values are inserted as compact JSON, `null` becomes empty.
- Objects and arrays are resolved recursively (keys are never resolved).
- Unterminated markers (`"price is ${100"`) are left as literal text.
- Expressions can compute: `"${input.user.age + 1}"`.

### Namespaces

Built by `FlowContext::to_json` — the **same context** for templates and
for REL conditions (decision steps, `or` rules):

| Namespace | Meaning |
|---|---|
| `input.*` | The flow input (run request body `input`, or the triggering node data). |
| `steps.<step_id>.*` | Output of a previously completed step (by node id). |
| `trigger.*` | Trigger info (event type, node path, actor, ...) when event-triggered. |
| `output.*` | The current step's fresh output — available in `compensation_input_mapping`. |
| `error.*` | Error info when running after an `error_edge` / `continue_on_fail` (§5.2). |
| *(bare names)* | Flow variables, including `__human_response` and the flat-merged keys of previous step outputs. |

> **Step id naming — use snake_case.** REL identifiers are
> `[A-Za-z0-9_]` only, so `steps.create-accounts.email` parses as
> **subtraction** (`steps.create - accounts.email`) and silently resolves
> to garbage. Bracket access (`steps['create-accounts'].email`) parses,
> but is **not null-safe** — it errors when the step was skipped (e.g. an
> OR-container branch that didn't run), whereas dot access on a missing
> step resolves to `null`. Prefer `create_accounts` over
> `create-accounts`; `raisindb flow doctor` flags hyphenated dot paths.

Condition examples (REL):

```text
steps.score.value > 5
input.tier == "premium"
(input.priority >= 5 || input.urgent == true) && input.enabled == true
__human_response.action == "approve"
```

Truthiness for conditions: `false`/`null`/`0`/`0.0`/`""`/`[]`/`{}` are
false; everything else is true.

> Note: object step outputs are *also* merged flat into the variables, so
> after a step returning `{score: 8}` both `steps.score-step.score` and the
> bare `score` resolve. Prefer the explicit `steps.<id>.*` form — flat keys
> can be clobbered by later steps.

---

## 5. Error handling per step

### 5.1 Retries

```yaml
properties:
  retry:
    max_retries: 2          # retry budget for this step
    base_delay_ms: 1000
    max_delay_ms: 10000
  # OR a preset name:
  retry_strategy: none      # disables retries (max_retries = 0)
```

Engine behavior (verified in `executor/result_handlers.rs` + `helpers.rs`):

- The retry budget comes from `max_retries`; **the default is 3** when
  nothing is configured.
- With a `retry` block, the backoff is exponential: `base_delay_ms`
  doubled per attempt, capped at `max_delay_ms`. Without one, a legacy
  fixed ladder applies: 10s, 30s, 60s, then 120s per attempt.
- Preset names other than `none` (`quick`, `standard`, `aggressive`, `llm`
  from the SDK's `RETRY_STRATEGIES`) are designer/SDK conventions: the
  designer UI expands them into a `retry` block. The engine itself only
  special-cases `retry_strategy: none`.

### 5.2 `error_edge`, `on_error`, `continue_on_fail`

Once retries are exhausted, the engine checks, in order:

1. **`error_edge`** (on the node or in `properties`) — jump to the named
   handler node. The error context is populated for the handler path:

   ```json
   "error": {
     "error_type": "step_error",
     "message": "...",
     "step_id": "charge",
     "timestamp": "..."
   }
   ```

   Reference it as `{{ error.message }}` / `error.step_id` in the handler
   steps.

2. **`continue_on_fail: true`** — continue to the next step; `error.*` is
   populated with `"continued": true`.

3. Otherwise the flow **fails** and saga compensation runs (§5.3).

`on_error: stop | skip | continue` (node level) is carried through to the
runtime as a property for UI/auditing; the enforced mechanics are the
`error_edge` / `continue_on_fail` paths above.

### 5.3 Saga compensation (`compensation_ref`)

```yaml
- id: book-flight
  node_type: raisin:FlowStep
  properties:
    function_ref: /lib/book-flight
    compensation_ref: /lib/cancel-flight
    # Optional input mapping; the forward step's output is available as output.*
    # (without a mapping, the forward arguments are reused)
    # NOTE: designer-format gap, see below.
```

- Compensation is registered **only after the forward function succeeded**.
- On a later unrecoverable failure, the flow status becomes `rolled_back`
  and compensations execute in **LIFO order** with their mapped inputs.
- `compensation_input_mapping` (e.g.
  `{ "booking_id": "${output.booking_id}" }`) resolves against the step's
  fresh output via `output.*`.

> **Designer-format gap:** `compensation_input_mapping` exists in the
> TypeScript types and is honored by the runtime, but it is **not** in the
> Rust designer schema — the converter drops it. In designer format the
> compensation receives the forward arguments. Use the runtime format if
> you need the mapping.

### 5.4 Timeouts

- `timeout_ms` (step property) — for function steps it becomes the wait
  deadline of the queued execution so a stuck function doesn't hang the
  flow; for containers it is carried on the lowered node.
- `timeout_edge` (step property) — where to route when a **wait deadline**
  expires (human task `due_in_seconds`, function `timeout_ms`). The waiting
  inbox task (if any) is marked `expired`. Without a `timeout_edge`, an
  expired wait **fails the flow** (then §5.2/§5.3 apply).

---

## 6. Human-in-the-loop lifecycle

### 6.1 Task creation

When a human task step executes, the engine creates a **`raisin:InboxTask`**
node in the **`raisin:access_control`** workspace at:

```
{assignee}/inbox/task-{step_id}-{timestamp}
```

e.g. `/users/manager/inbox/task-approve-1718012345678`. Task properties:
`task_type`, `title`, `description`, `assignee`, `priority`, `options` /
`input_schema`, `status: pending`, `flow_instance_id`, `step_id`,
`created_at`, `due_in_seconds` + `due_at`. The flow then waits
(`human_task`).

### 6.2 Completion and resume

Complete via HTTP (the caller must be the assignee or an admin):

```
POST /api/inbox/{repo}/tasks/{task_id}/complete
{ "response": { "action": "approve", "comment": "LGTM" } }
```

or via the SDK: `inbox.completeTask(taskId, { action: 'approve' })`.
Response payload conventions:

- approval: `{ action: "<option value>", comment?: string }`
- input: the value(s) matching the task's `input_schema`
- review/action: any acknowledgement payload

Completion marks the task `completed` (recording `completed_by`,
`responded_at`, `response`) and **resumes the flow**. Downstream steps see
the response as the human-task step's output (`steps.approve.*`) **and** as
the `__human_response` variable:

```yaml
# A later 'or' rule or decision condition:
condition: "__human_response.action == \"approve\""
```

> **Chained human tasks:** `__human_response` always holds the response of
> the **most recently completed** task — with several human tasks in one
> flow, gate on the specific step's output instead
> (`steps.quote_review.action`). The step output carries the response plus
> `completed_by` and `task_path`.

Task statuses: `pending | completed | expired | cancelled`.

> **`waiting` does not mean "parked for a human".** The runtime also reports
> a transient `waiting` status *between* steps, while a queued execution is
> in flight. Code that asks "is this instance blocked on a person?" must
> discriminate on the **wait reason** (`human_task`, recorded in the
> instance's `WaitInfo` alongside the task path) rather than on the bare
> status — or, from outside the engine, on an inbox task existing for the
> instance. Two other wait reasons park a flow the same way:
> `parallel_branches` (a fork awaiting its children) and `sub_flow`.
>
> **Completing a task the instant it appears can race the park.** The inbox
> task node is created — and becomes listable — slightly *before* the owning
> instance's own status record settles to `Waiting` in the same execution.
> Completing in that window returns
> `Invalid state transition from pending to resumed`. Retrying succeeds once
> the instance settles, so a caller that lists and immediately completes
> should poll the instance status to `waiting` first, or tolerate and retry
> that specific error rather than surfacing it as a hard failure.

### 6.3 AI agent as assignee

If `assignee` resolves to a `raisin:AIAgent` node (in the functions
workspace), the task is still created (full audit trail), then **evaluated
by the agent immediately**:

- The agent receives the task (title, description, options/input_schema)
  plus the workflow context and must answer with a structured decision
  `{ decision | value, reasoning, confidence }` (the engine builds the JSON
  schema automatically from `options` / `input_schema`).
- **Confident decision** (`confidence >= min_confidence`, default **0.7**):
  the task is completed with `completed_by` = the agent path, the response
  mirrors a human submission (`{ action, comment, confidence }` for
  approvals; `{ value, comment, confidence }` for input tasks), and the
  flow continues — `__human_response` works identically.
- **Low confidence, unparseable output, or AI error**: the task is
  **escalated** — reassigned to `escalation_assignee` (if configured) with
  audit fields `escalated_from`, `escalation_reason`, `escalated_at` — and
  the flow waits for the human like any other task. Without an
  `escalation_assignee`, the task stays assigned to the agent and must be
  completed via the inbox API.

```yaml
properties:
  step_type: human_task
  task_type: approval
  assignee: /agents/refund-approver
  min_confidence: 0.75
  escalation_assignee: /users/admin
  options:
    - { value: approve, label: Approve refund }
    - { value: reject,  label: Reject }
```

---

## 7. Running and observing flows

### 7.1 HTTP API

| Endpoint | Purpose |
|---|---|
| `POST /api/flows/{repo}/run` | Start a flow. Body: `{ "flow_path": "/flows/order-approval", "input": {...} }`. Returns `{ instance_id, job_id, status: "queued" }`. |
| `POST /api/flows/{repo}/test` | Test run. Body adds `test_config`: `{ is_test_run, mock_functions, isolated_branch, auto_discard }`. |
| `GET /api/flows/{repo}/instances/{id}` | Instance status: `{ id, status, variables, flow_path, started_at, error? }`. |
| `POST /api/flows/{repo}/instances/{id}/resume` | Resume a waiting instance with `{ "resume_data": {...} }`. |
| `POST /api/flows/{repo}/instances/{id}/cancel` | Cancel a running/waiting instance. |
| `DELETE /api/flows/{repo}/instances/{id}` | Delete an instance. |
| `GET /api/flows/{repo}/instances/{id}/events` | **SSE** stream of execution events. |
| `GET /api/inbox/{repo}` | List the caller's inbox tasks (`?status=pending&assignee=...`). |
| `GET /api/inbox/{repo}/tasks/{task_id}` | Get one task. |
| `POST /api/inbox/{repo}/tasks/{task_id}/complete` | Complete a task (resumes the owning flow). |

Test-run mocks (`test_config.mock_functions`, keyed by function path):

```json
{
  "is_test_run": true,
  "mock_functions": {
    "/lib/charge-payment": { "behavior": "mock_output", "mock_output": { "charge_id": "test" } },
    "/lib/audit-log":      { "behavior": "passthrough", "mock_delay_ms": 100 }
  },
  "isolated_branch": true,
  "auto_discard": true
}
```

Behaviors: `real` (default), `passthrough` (input echoed as output),
`mock_output`. AI agents always run real and cannot be mocked.

### 7.2 SSE events

Events are tagged `{"type": "...", ...}` (snake_case):

`step_started` (node_id, step_name, step_type), `step_completed` (node_id,
output, duration_ms), `step_failed` (node_id, error, duration_ms),
`flow_waiting` (node_id, wait_type, reason), `flow_resumed` (node_id,
wait_duration_ms), `flow_completed` (output, total_duration_ms),
`flow_failed` (error, failed_at_node, total_duration_ms), plus AI streaming
events: `text_chunk`, `tool_call_started`, `tool_call_completed`, and `log`.

### 7.3 Instance statuses

`pending → running → waiting ⇄ running → completed | failed | cancelled | rolled_back`

`waiting` covers human tasks, queued functions, chat turns, sub-flows,
scheduled waits, and retry backoff.

### 7.4 Admin console

- **Flows** (repository → Flows): list, create, and open flows in the
  **visual designer** (drag steps/containers, configure properties in the
  side panel — what it saves is exactly the designer format documented
  here). The **Run dialog** starts a flow with a JSON input and live event
  view; the test-run mode exposes the `mock_functions` editor.
- **Inbox** (repository → Inbox): the assignee-facing task list; approve /
  reject / fill input forms; completing a task resumes the flow.
- **Flow Instances** (management → Flow Execution Monitor): instance list
  with status, step timeline, variables, errors, and cancel/delete actions.

---

## 8. SDK quick reference (`@raisindb/client`)

```js
import { RaisinHttpClient, FlowClient, InboxApi } from '@raisindb/client';

const client = new RaisinHttpClient(BASE_URL, { tenantId: 'default' });
await client.authenticate({ username, password });

const flows = FlowClient.fromHttpClient(client, BASE_URL, repo);
const inbox = new InboxApi(BASE_URL, repo, client.getAuthManager());

// Start a flow
const { instance_id } = await flows.run('/flows/order-approval', { order_id: 'ORD-1' });

// Status & events
const status = await flows.getInstanceStatus(instance_id);
const stream = await flows.createEventStream(instance_id);   // { events, close() }
for await (const event of stream.events) { /* §7.2 event types */ }
// (streamEvents(id) is the lazy async-generator variant)

// Convenience runners
await flows.runAndWait('/flows/x', input);     // run + poll until terminal
await flows.runAndCollect('/flows/x', input);  // run + collect all events

// Resume / human tasks
await flows.resume(instance_id, resumeData);
await flows.respondToHumanTask(instance_id, { action: 'approve' });

// Inbox
const { tasks } = await inbox.listTasks({ status: 'pending', assignee: '/users/admin' });
const task = await inbox.getTask(taskId);
await inbox.completeTask(taskId, { action: 'approve', comment: 'LGTM' });
```

On a `Database` obtained with HTTP context, the same clients are available
as **`db.flow`** and **`db.inbox`**:

```js
const result = await db.flow.runAndWait('/flows/my-flow', { key: 'value' });
const { tasks } = await db.inbox.listTasks({ status: 'pending' });
```

Working end-to-end scripts: `examples/workflows/approval-flow/run.mjs`
(deploy + run + inbox completion + SSE) and
`examples/workflows/ai-approval-flow/run.mjs` (agent step + agent assignee
+ escalation).

---

## 9. Complete worked example

An order workflow combining REL routing (`or` container), function steps
with templated arguments and saga compensation, a human approval with an
**AI agent assignee**, and error handling:

```yaml
node_type: raisin:Flow
properties:
  title: Order Fulfillment
  name: order-fulfillment
  enabled: true
  workflow_data:
    version: 1
    error_strategy: fail_fast
    nodes:
      # 1. Validate the order (function step with retry + error edge)
      - id: validate
        node_type: raisin:FlowStep
        properties:
          action: Validate order
          function_ref: /lib/validate-order
          arguments:
            order_id: "{{ input.order_id }}"
            items: "${input.items}"           # keeps the array type
          retry: { max_retries: 2, base_delay_ms: 1000, max_delay_ms: 10000 }
          error_edge: record-failure          # invalid orders -> failure handler

      # 2. Route by order size (or-container, first matching rule wins;
      #    no match = container skipped entirely)
      - id: route
        node_type: raisin:FlowContainer
        container_type: or
        rules:
          - { condition: "input.amount >= 1000", next_step: approve-large }
          - { condition: "input.amount < 1000",  next_step: auto-approve }
        children:
          # 2a. Large orders: approval task assigned to an AI agent.
          #     Confident agent decisions complete it; otherwise it
          #     escalates to the human ops lead.
          - id: approve-large
            node_type: raisin:FlowStep
            properties:
              action: "Approve order {{ input.order_id }} ({{ input.amount }} CHF)"
              step_type: human_task
              task_type: approval
              assignee: /agents/order-approver
              min_confidence: 0.8
              escalation_assignee: /users/ops-lead
              task_description: "Validated: {{ steps.validate.summary }}"
              priority: 4
              due_in_seconds: 86400
              timeout_edge: record-failure
              options:
                - { value: approve, label: Approve, style: success }
                - { value: reject,  label: Reject,  style: danger }

          # 2b. Small orders: mark approved automatically
          - id: auto-approve
            node_type: raisin:FlowStep
            properties:
              action: Auto-approve small order
              function_ref: /lib/mark-approved
              arguments: { order_id: "{{ input.order_id }}" }

      # 3. Gate on the decision (or-container reading __human_response;
      #    auto-approved orders have no rejection, so the charge rule matches)
      - id: decision-gate
        node_type: raisin:FlowContainer
        container_type: or
        rules:
          - { condition: "__human_response.action == \"reject\"", next_step: record-rejection }
          - { condition: "true", next_step: charge }
        children:
          - id: record-rejection
            node_type: raisin:FlowStep
            properties:
              action: Record rejection
              function_ref: /lib/record-rejection
              arguments:
                order_id: "{{ input.order_id }}"
                reason: "{{ __human_response.comment }}"

          # Charge with saga compensation: if a LATER step fails
          # unrecoverably, /lib/refund-payment runs automatically (LIFO).
          - id: charge
            node_type: raisin:FlowStep
            properties:
              action: Charge payment
              function_ref: /lib/charge-payment
              arguments:
                order_id: "{{ input.order_id }}"
                amount: "${input.amount}"
              compensation_ref: /lib/refund-payment
              timeout_ms: 30000
              error_edge: record-failure

      # 4. Ship (best-effort notification afterwards)
      - id: ship
        node_type: raisin:FlowStep
        properties:
          action: Create shipment
          function_ref: /lib/create-shipment
          arguments: { order_id: "{{ input.order_id }}" }

      - id: notify
        node_type: raisin:FlowStep
        properties:
          action: Notify customer
          function_ref: /lib/send-notification
          arguments:
            order_id: "{{ input.order_id }}"
            tracking: "{{ steps.ship.tracking_number }}"
          continue_on_fail: true          # notification failure must not fail the flow

      # 5. Shared failure handler (reached via error_edge / timeout_edge;
      #    error.* is populated on the error path)
      - id: record-failure
        node_type: raisin:FlowStep
        properties:
          action: Record failure
          function_ref: /lib/record-failure
          arguments:
            order_id: "{{ input.order_id }}"
            failed_step: "{{ error.step_id }}"
            message: "{{ error.message }}"
```

Run it:

```bash
curl -X POST "$RAISIN_URL/api/flows/$REPO/run" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"flow_path": "/flows/order-fulfillment",
       "input": {"order_id": "ORD-1042", "amount": 1490, "items": [{"sku": "A1", "qty": 2}]}}'
```

> Caveat for this example: nodes that follow an `error_edge` target in the
> array (here `record-failure` is last, so on the *normal* path the flow
> reaches it after `notify`). To keep a handler out of the happy path,
> either make it the last node and have preceding branches route around
> it, or end handler branches explicitly (runtime format). In this layout
> `record-failure` runs as the final step on both paths — make the
> function idempotent / no-op when `error` is absent.

---

## Appendix A: runtime format and runtime-only step types

Before lowering, the engine also accepts `workflow_data` directly in the
**runtime format**: a flat node list with `step_type`, explicit
`start`/`end` nodes, and explicit `next_node` chaining. Some step types are
**only** expressible there (the designer schema has no step_type for them):

| Step type | Key properties | Notes |
|---|---|---|
| `decision` | `condition` (REL), `yes_branch`, `no_branch` | Two-way branch — **the way to express if/else**. Both arms are named explicitly, so exactly one runs; don't try to emulate it with two sibling containers carrying complementary conditions. (Designer `or` containers lower to cascades of these.) |
| `wait` | `wait_type: delay\|until\|event\|cron`, `duration` (e.g. `"5s"`, `"30m"`, `"1h"`, templated), `until`, ... | Pause for time/event. |
| `loop` | `loop_type: for_each\|while\|times`, `collection` (expr), `item_var`, `index_var`, `body_step`, `condition`, `max_iterations`, `until` (REL early exit) | The body step must chain back to the loop node. `for_each` loops are authorable in the designer format via `container_type: loop` (section 3.6); `while`/`times` remain runtime-only. |
| `sub_flow` | `flow_ref` (path to a `raisin:Flow`), `input_mapping` (templated object), `async` | Child output becomes `steps.<id>.*` in the parent. |
| `parallel` | Static: `branches: [{id, flow_path \| flow_definition, input_mapping?}]`. Dynamic: `for_each` (collection expr) + `branch: {id?, flow_path \| flow_definition, input_mapping?}` + `max_branches`. Both: `merge_strategy: merge_all\|first_success\|all_success` | Fork/join (section 3.3). A `for_each` fan-out creates one child flow per item, with `item`/`index` bound in the branch template. |

Runtime-format example (see `e2e_flows.rs` for many more):

```json
{
  "nodes": [
    { "id": "start", "step_type": "start", "next_node": "each" },
    { "id": "each", "step_type": "loop",
      "properties": { "loop_type": "for_each", "collection": "${input.items}",
                      "item_var": "current", "body_step": "body" },
      "next_node": "end" },
    { "id": "body", "step_type": "function_step",
      "properties": { "function_ref": "/lib/process", "arguments": { "item": "${current}" } },
      "next_node": "each" },
    { "id": "end", "step_type": "end" }
  ]
}
```

Aliases accepted for `step_type`: `agent_step`/`ai_agent` → AgentStep,
`ai_container`/`ai_sequence` → AIContainer, `chat_step`/`chat_session` →
Chat. In runtime format, `agent_step` and AI-container nodes read their
continuation from a **`next_node` property inside `properties`** (not the
top-level field) — set both to be safe.

## Appendix B: known gaps and limitations (as of this writing)

Verified against the code; re-check when upgrading:

1. **Human tasks have a single deadline.** `due_in_seconds` is one terminal
   wait deadline; there is no `reminders: [{after, notify}]` list, so each
   escalation tier must be authored as its own `human_task` +
   `timeout_edge`. See `docs/OPEN-ITEMS.md` §2.5.
2. **`while` / `times` loops** are runtime-format only. `for_each`
   iteration IS representable in the designer format as a
   `container_type: loop` container (section 3.6).
3. **Flow-level `timeout_ms`** is preserved in the lowered flow's metadata
   but not yet enforced as a total-instance deadline (per-step and wait
   timeouts ARE enforced).
4. **Chat `inactivity_timeout_ms`** is carried through to the runtime
   termination config but the chat handler does not yet auto-terminate on
   inactivity.

Previously listed gaps now fixed: `wait`, `sub_flow`, and free-standing
`decision` steps in the designer format (section 2.6); all three parallel
merge strategies
(`merge_all`, `first_success`, `all_success`) plus dynamic `for_each`
fan-out (section 3.3); templated `due_in_seconds`/`priority` on human tasks
(section 2.4); open (application-definable) `task_type` values;
designer chaining after `ai_agent`/
`ai_sequence` steps; `compensation_input_mapping`, `response_format`/
`output_schema`, `max_retries`/`retry_delay_ms` in the designer schema;
flow-level `error_strategy: continue` (lowers to `continue_on_fail` on work
steps without explicit error handling); retry `base_delay_ms`/
`max_delay_ms` honored with exponential backoff (legacy 10s/30s/60s/120s
ladder only applies when no retry config is set); chat TS/Rust drift
(`trigger_condition`, `trigger_phrases`, `modes`/`termination_phrases` are
accepted and normalized).
