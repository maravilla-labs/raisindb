# AI Tools Package

A comprehensive AI-powered toolkit for RaisinDB that enables intelligent content generation, conversational AI agents, and automated workflows.

## Features

- **AI Prompts** - Create and manage reusable prompt templates
- **AI Conversations** - Build multi-turn conversational experiences
- **AI Agents** - Deploy autonomous agents with tool-calling capabilities
- **AI Tasks & Plans** - Orchestrate complex AI workflows

## Screenshots

### Agent Configuration

![Agent Configuration Screen](static/screen1.png)

Configure your AI agents with custom system prompts, tool access, and behavioral settings.

### Conversation View

![Conversation Interface](static/screen2.png)

Real-time conversation interface with message threading and tool call visualization.

## Installation

Install this package from the RaisinDB admin console:

1. Navigate to **Packages** in the sidebar
2. Find **AI Tools** in the available packages
3. Click **Install** and select your preferred install mode

## Node Types

This package provides the following node types:

| Node Type | Description |
|-----------|-------------|
| `raisin:AIPrompt` | Reusable prompt templates with variable interpolation |
| `raisin:AIConversation` | Multi-turn conversation containers |
| `raisin:AIMessage` | Individual messages within conversations |
| `raisin:AIModel` | Model configuration and parameters |
| `raisin:AIAgent` | Autonomous agent definitions |
| `raisin:AIThought` | Agent reasoning traces |
| `raisin:AITask` | Discrete task definitions |
| `raisin:AIPlan` | Multi-step execution plans |
| `raisin:AIToolCall` | Tool invocation records |
| `raisin:AIToolResult` | Tool execution results |

## Quick Start

### Creating an AI Agent

```yaml
node_type: raisin:AIAgent
name: my-assistant
properties:
  title: My Assistant
  system_prompt: |
    You are a helpful assistant that answers questions
    about the RaisinDB documentation.
  model: gpt-4
  temperature: 0.7
  tools:
    - search_docs
    - create_note
```

### Creating a Prompt Template

```yaml
node_type: raisin:AIPrompt
name: summarize-template
properties:
  title: Document Summarizer
  template: |
    Summarize the following document in {{style}} style:

    {{content}}
  variables:
    - name: style
      default: concise
    - name: content
      required: true
```

## Workspaces

This package creates the `ai` workspace for organizing AI-related content:

- `/agents` - Agent definitions
- `/prompts` - Prompt templates
- `/conversations` - Active conversations

## Functions

- `agent-handler` — the entry of every agent conversation (called by
  `raisin-messaging`'s `messaging-agent-chat` trigger): it starts the
  conversation's agent run or steers the live one, nothing else.
- `agent-run-*` — the run's reducer, model turn, projection and user controls
  (below).
- The agent tools: planning, memory (`remember`, `read-user-context`,
  `forget`), delegation, search / ask / graph context, `load-skill`, and the
  node-development tools under `/lib/raisin/node-dev`.

There is no other chat loop. A package update retires what an older
installation still holds of the earlier one (`migrations/`).

## Token safeguards & compaction

All of these agent properties (`raisin:AIAgent`) are optional — when absent,
behavior is unchanged.

| Property | Type | Default | Effect |
|----------|------|---------|--------|
| `max_history_messages` | number | 50 | Per-turn history window sent to the model (system prompt + compaction summary are always kept on top). |
| `auto_compact` | boolean | `false` | Enable conversation auto-compaction. |
| `compact_threshold_messages` | number | 30 | Active (non-compacted) message count that triggers compaction at turn start. |
| `compact_keep_messages` | number | max(2, threshold/3) | Floor of most recent messages kept verbatim when compacting. |
| `max_conversation_tokens` | number | — | Token budget between compactions. With auto-compaction, compaction runs at 80% pressure and starts a fresh window; otherwise reaching the limit refuses the turn. |

### How compaction works

When `auto_compact` is enabled and the conversation's active message count
exceeds `compact_threshold_messages` at the start of a turn, the older
messages (everything except the most recent third, with a
`compact_keep_messages` floor) are summarized with one extra AI call to the
agent's own model. The result is persisted as a `raisin:AICompaction` child
of the agent-side conversation node (`summary`, `summary_preview`,
`messages_compacted`, `messages_kept`, `cutoff_message_path`), so it is never
recomputed per turn. History building then sends:

```
[system prompt, "Earlier conversation summary: …", messages after the cutoff]
```

Later compactions supersede earlier ones (latest by `created_at` wins) and
chain the previous summary into the new one. The summarization call's tokens
are recorded as a `raisin:AICostRecord` under the compaction node.

### Token accounting

Every AI call writes a `raisin:AICostRecord` and increments a lifetime
`total_tokens_used` property on the agent-side conversation node. When
auto-compaction is enabled, `max_conversation_tokens` acts as a rolling
window: at 80% pressure the handler summarizes older messages and stores the
current lifetime total as `token_checkpoint` on the compaction. Later turns
measure usage from that checkpoint. If compaction is disabled or cannot make
progress, reaching the limit creates a normal budget-exceeded assistant reply
and emits the standard `done` event without making the main provider call.

## Agent runs (the durable runtime)

Every agent conversation runs as an **AgentRun** — RaisinDB core's durable run
(one live run per conversation, stoppable, steerable, resumable after a worker
dies; its steps are jobs any cluster node can pick up). Messages are the
transcript of a run, never its state.

| Function | Role |
|----------|------|
| `agent-run-reducer` | The generic agent loop as a `raisin.agent-run.reducer/1` reducer, run under the deterministic execution policy. Tool gate, loop detection, progress accounting; the plan is run state and planning tools are domain tools it answers itself. |
| `agent-run-model-turn` | One model turn (core's model-turn seam): context from the transcript plus the run's authoritative facts, compaction to a structured checkpoint, output budget, provider request with streaming and bounded retries, and the turn's transcript message. |
| `agent-run-project` | The projection operation the reducer issues: plan card, pending approval, and the final answer with the runtime's status. |
| `agent-run-control` | User controls (stop, pause, resume, steer, approve, reject, answer, status), applied by the runtime and acknowledged. |

Rules the runtime enforces rather than the prompt:

- **Completion is requested, never written.** `update-task(completed)` is granted
  only when a tool operation succeeded while the task was in progress; the card
  shows `completion_requested` otherwise, and the run ends `partial`.
- **Stop / steer / pause / approve** go to the runtime (`request-conversation-stop`,
  `plan-approval-handler` and `agent-run-control` route them). A message sent
  while a run works is a steer: `run_steer_state` on the message and the
  `conversation:steer_queued` / `conversation:steer_consumed` events say whether it
  is queued or has been read.
- **Tool results** from a run come back as `raisin.tool-result/1` envelopes
  (`operation_id`, `status`, `writes`, `artifact_refs`, `evidence`, `diagnostics`,
  `suggested_next_actions`, `retry_policy`); outside a run every tool answers as
  before. The operation id is the idempotency key of a mutating tool.

Agent properties: `run_reducer` names another reducer (any function language); `model_turn_function`, `run_budgets`,
`run_config` (loop and no-progress limits), `max_context_tokens`,
`max_tool_result_chars`, `max_output_tokens`, `transient_retries` tune it.
A server that cannot start a run answers the user with an error turn; there
is no fallback loop. Historical conversations stay readable as transcripts.

**Memory** (`remember`, `read-user-context`, `forget`) belongs to the run's
OWNER: the agent the run executes and the user it acts for, read from the run
record — never from a tool's arguments. The tools run in a system context (no
user role reaches `ai:/agents/*/memory`) and first prove they are the run's
active operation; outside a run they refuse.

### Delegation: child runs

An agent delegates by starting a **child run** through core
(`raisin.agentRuns.spawnChild`): an ordinary AgentRun of an installed agent (or
a worker copy of itself), acting for the same user, with its own conversation
under `ai:/agents/<child>/inbox/chats/deleg-…` as its transcript. Delegation
exists only inside a run.

| Tool | What it does |
|------|--------------|
| `spawn-agent` | Starts a child with a typed objective (`goal`, `deliverable`, `done_when`, `constraints`), selected context (`none`, `recent` turns, or a `snapshot` — core checkpoints the parent and hands the child a reference), a tool grant (a subset of the child agent's own tools; core refuses any other call and the model turn does not offer it), a write grant (`writes: [{workspace, path}]`, `[]` = read-only), expected artifacts, acceptance checks and a budget (a child that overruns fails; it never parks). Idempotent by its spawn key. |
| `inspect-agent` | Status, outcome, summary, artifacts, acceptance and recent run events of one child or all, plus messages children posted to the run's mailbox (read = acknowledged). |
| `message-agent` | New input for a working child (`mode: message` informs, `steer` redirects): written to its transcript, then queued through core, which applies it at the child's next safe boundary. |
| `wait-for-agents` | Waits for all (or `any`) named children. Answers at once when they are done; otherwise it answers `waiting` on core's `child:{id}` resume key and core delivers that child's hand-back into the waiting operation — the reducer then waits again for the rest, so the model gets one answer. A user message interrupts the wait; the children keep working. |
| `interrupt-agent` | Stops (or pauses) one child or all. |
| `delegate-task` / `get-delegation-status` | The plan-task spellings of spawn and inspect, keyed by task (`task-<id>`): delegating a task twice finds its first child. |

Core owns the lineage: admission and budget reservation on the parent, the
hand-back into the parent's mailbox, the cascade stop of live children when the
parent ends (a user stop included), and the repair of all of it after a crash —
jobs any cluster node picks up. ai-tools stores no delegation state. Rules it
adds: a second child may run in parallel only for work that does not depend on
the first (`independent: true`), within `delegation.max_parallel` (default 2);
`delegation.max_depth` (default 1: children cannot delegate further),
`max_children` and `allowed_agents` narrow the rest. A delegated run's write
grant is also enforced by the reducer — a call naming a path outside it is
refused, and a write reported outside it ends the child `blocked`. Acceptance
checks (`node_exists`, `property_equals`, `artifact_written`, `outcome_is`)
travel on the child's objective and are evaluated on stored state when the
parent reads the child, never on the child's summary.

## Configuration

Configure AI model providers in your RaisinDB settings:

```yaml
ai:
  providers:
    openai:
      api_key: ${OPENAI_API_KEY}
      default_model: gpt-4
    anthropic:
      api_key: ${ANTHROPIC_API_KEY}
      default_model: claude-3-opus
```


raisindb package create ./ai-tools      

raisindb package upload ai-tools-1.0.0.rap -r social_feed_demo_rel4

## License

MIT License - See LICENSE file for details.

---

*Built with RaisinDB Package System*
