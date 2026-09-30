---
name: raisindb-messaging-agents
description: "Build AI chat and messaging on RaisinDB: AI agents with tools, the inbox/outbox chat pipeline, agents that proactively message users and coordinate between them, human-in-the-loop task UIs, chatbox frontends with the JS SDK, and token safeguards (budgets, auto-compaction). Use this whenever the user wants a chatbot, AI assistant, agent with tools, notifications, an inbox, agent-to-user messaging, multi-user coordination ('agent asks staff one by one'), an assistant that answers from the user's own documents (pair it with raisindb-retrieval for the search side), a public website's chat for anonymous visitors without login (db.conversations.startAnonymous, agent `anonymous` settings — never a shared service identity), or anything involving raisin:AIAgent, conversations, or message nodes — even if they just say 'add AI to my app'."
---

# Messaging & AI Agents

Everything is nodes: conversations, messages, tasks, notifications live in
each user's home (`{home}/inbox/...`, `{home}/outbox/...`) in the
`raisin:access_control` workspace. Agents have a mirror home in the `ai`
workspace. The builtin `raisin-messaging` package (auto-installed with every
repo) delivers between them; the builtin `ai-tools` package runs the agent.

**Working reference app** (read it before inventing anything):
`examples/shiftboard/` — Groq agent with tools that read/update nodes,
proactive staff coordination via chat, in-app inbox-task UI, SvelteKit SSR
frontend, plus repeatable test scripts (`smoke.mjs`, `negotiation-test.mjs`).

## How direct chat works (the pipeline)

```
user sends message            agent answers
  └─ raisin:Message in          └─ agent-handler runs the LLM with the
     {home}/outbox/                agent's tools, writes the reply into
       │ process-chat trigger      /agents/{name}/outbox/
       ▼ (builtin)                   │ process-chat again
  delivered into BOTH sides'         ▼
  conversations; for agent      delivered to the user's conversation;
  recipients lands in           SSE events stream to the client
  /agents/{name}/inbox/chats/   (text_chunk, tool_call_*, done)
       │ process-agent-chat
       ▼
  /lib/raisin/ai/agent-handler
```

Key consequences:
- An agent reacts to ANY message delivered to it — including replies from
  users it messaged first. Each conversation thread is its own context.
- Agents can INITIATE conversations: drop a correctly-shaped
  `raisin:Message` into the agent's outbox (see "Proactive messaging").
- The canonical record (tokens, `raisin:AICostRecord`, tool-call audit
  nodes) lives on the AGENT side in the `ai` workspace; user-side copies are
  mirrors without usage data.

## Defining an agent

`raisin:AIAgent` node in the `functions` workspace + a home folder in the
`ai` workspace (`user_id: agent:{name}`, with `inbox/chats` etc. — copy the
structure from `examples/shiftboard/package/content/ai/agents/shift-planner/`).

```yaml
node_type: raisin:AIAgent
properties:
  system_prompt: |
    ...role, protocol, rules...
  provider: groq                  # tenant must have the provider configured:
  model: llama-3.3-70b-versatile  #   raisindb ai provider set groq --api-key-stdin
  temperature: 0.2
  max_tokens: 1024
  tools:
    - /lib/myapp/list-things      # plain function paths
    - /lib/raisin/ai/weather      # builtin tools work too
  # Token safeguards (all optional):
  # auto_compact: true                # summarize old turns into a persisted
  # compact_threshold_messages: 30    #   raisin:AICompaction node
  # max_history_messages: 50          # hard prompt window
  # max_conversation_tokens: 50000    # budget; exceeded -> polite refusal
```

## Tools = functions (the LLM sees your schema)

A tool is a `raisin:Function` whose `description` + `input_schema` become the
LLM tool definition — write them for the model, not for humans.

**An agent that should answer from your own content needs retrieval tools**, or
it answers from the model's training and sounds just as confident:

```yaml
  tools:
    - /lib/raisin/ai/search-documents   # passages, with citation handles
    - /lib/raisin/ai/ask                # retrieve + answer, already cited
    - /lib/raisin/ai/graph-context      # how things relate
```

`/agents/research-assistant` ships wired this way. For what those return,
grounding (`grounded: false` means the model was never called), chunking and
the workspace scope, read the **raisindb-retrieval** skill.

Function-runtime traps (NOT the client SDK):
- `raisin.sql.query(...)` returns the **row array directly** (the client's
  `executeSql` returns `{rows}`); use `raisin.sql.execute` for DML.
- Tool calls receive an injected `__raisin_context` argument
  (`agent_name`, `conversation_path`, `sender_id`, ...) — use it to know who
  is talking and from which thread.
- Return **graceful errors as data** (`{error: "...pick another candidate"}`)
  instead of throwing — the model self-corrects in the same turn.

## Plans & approval-gated execution

Set `task_creation_enabled: true` and add the builtin planning tools
(`/lib/raisin/ai/create-plan|add-task|update-task|get-plan-status`) and the
agent decomposes work into a persisted `raisin:AIPlan` + `raisin:AITask`
tree. `execution_mode` controls the gate: `automatic` (run immediately),
`approve_then_auto` (one approval, then run all), `step_by_step` (one task
per continue signal), `manual` (approval + explicit instructions). With
`task_creation_enabled: false`, planning tools are filtered from the model —
no plan nodes ever.

Client side it's all in the SDK: `ConversationStore.plans` projects plan/task
state from the `ai_plan` / `ai_task_update` message cards;
`approvePlan(planPath)` / `rejectPlan(planPath, feedback?)` resolve the gate;
a `waiting` event (`reason: awaiting_plan_approval`) parks the turn, and
`step_by_step` pauses with `finish_reason: awaiting_step_continue` — any
plain user message ("continue") resumes. Full contract + UI recipe:
`docs/guides/ai/agent-plans-and-tools.md` on the website; reference client:
the admin console's agent Test Chat; scripted proof:
`examples/shiftboard/plan-modes-test.mjs`.

## Proactive messaging (agent → user)

To message a user from a tool, mirror the agent-handler's own outbox shape
(full implementation: `examples/shiftboard/package/content/functions/lib/
shiftboard/message-staff/index.js`): a `raisin:Message` in
`/agents/{name}/outbox` with `role: assistant`, `message_type: chat`,
`status: pending`, `sender_id: agent:{name}`, `recipient_id` = the user's
**raisin:User node id**, `body: {content, message_text, thread_id}`,
`conversation_id`. Delivery creates the user-side conversation if needed.
Reuse the conversation the current turn runs in when the recipient is one of
its participants (so confirmations return to the asking thread), else the
most-recently-updated thread with that user.

## Multi-user coordination — prompt lessons (hard-won)

Each thread only sees its own history. Encode the protocol in the system
prompt:
1. **Threads must be self-sufficient**: restate the subject (title, day,
   time, node path) and prior decliners in every outreach message.
2. **Only facts from tools**: "use ONLY the exact title/time/location from
   tool results; never invent venues, times, or people" — without this the
   model embellishes.
3. **Confirmation before action**: "only assign someone who confirmed in
   chat"; never ask the same person twice for the same thing.
4. **Status questions are read-only**: "report the live board from the list
   tool; take no action" — otherwise a status question in one thread can
   trigger actions based on stale context from another.
5. Don't `message-staff` the person you're currently chatting with — the
   normal reply already reaches them.

When coordination needs deadlines, escalation, or an audit trail, move it
into a workflow the agent STARTS via a tool (`raisin.flows.run`) — see the
`raisindb-workflows` skill. Chat = interface, workflow = engine.

## Client side (JS SDK)

```ts
// Connect tenant-less; tenant resolves server-side (default in dev)
const client = new RaisinClient('ws://localhost:8081/ws/myrepo');
await client.loginWithEmail(email, password, 'myrepo');
const db = client.database('myrepo');

// Chat: create once, stream turns
const convo = await db.conversations.create({ participant: '/agents/helper' });
for await (const ev of db.conversations.sendMessage(convo.conversationPath, text, { stream: true })) {
  // ev.type: text_chunk | tool_call_started | tool_call_completed | done | failed | waiting
}
// Or use ConversationStore (messages, isStreaming, activeToolCalls, hang
// recovery built in) - and useConversation from @raisindb/client/react|vue,
// the svelte adapters from @raisindb/client/svelte.
```

- **Inbox/notifications need NO extra API**: subscribe to node events on
  `${home}/inbox/**` (`**` is required — `*` matches exactly one segment,
  there is no implicit prefix matching) and render whatever arrives:
  messages, notifications, and `raisin:InboxTask` nodes.
- **Human-task UI is your UI**: render the task node's `options` array as
  buttons; complete via `POST /api/inbox/{repo}/tasks/{id}/complete` with the
  user's own bearer. See `TaskPanel.svelte` + `stores/tasks.svelte.ts` in the
  shiftboard frontend.
- Stability knobs: `sendMessage` inactivity timeout (default 120s →
  synthetic `waiting`), `ConversationStore` `streamingTimeoutMs` +
  watchdog, request queueing during reconnects. Skip `networkidle`-style
  waits in tests — persistent SSE keeps the network busy by design.

## Anonymous visitors (a public website's chat, no login)

A public site runs the agent chat **straight from the browser** over the WS
SDK: no site server endpoint, no service identity, no credential in the page.
Do NOT build a server endpoint that logs in as one shared "chat user" — that
is what this replaces (and it mixes visitors up in one home).

```yaml
# the agent (functions:/agents/website-assistant)
anonymous:
  enabled: true                      # REQUIRED; agents without it refuse visitors
  allowed_origins: [https://www.example.com]
  tool_roles: [site_assistant]       # what the TOOLS may do for a visitor
  max_messages: 20                   # per conversation
  max_conversation_tokens: 50000     # hard budget per conversation
  rate_per_session_per_minute: 6
  rate_per_ip_per_minute: 30
  max_concurrent_turns: 1            # lease; a hung turn frees it after turn_lease_seconds
  session_ttl_hours: 24              # purged after inactivity
stream_tool_results: true            # tool_call_completed carries `result` (for cards)
```

```ts
const client = new RaisinClient('wss://db.example.com/ws/website');
await client.connect();                              // anonymous, no login
const db = client.database('website');
const convo = await db.conversations.startAnonymous('/agents/website-assistant');
for await (const ev of db.conversations.sendMessage(convo.conversationPath, text)) {
  if (ev.type === 'text_chunk') append(ev.text);
  if (ev.type === 'tool_call_completed') renderCard(ev.functionName, ev.result);
  if (ev.type === 'failed') showLimit(ev.code);      // RATE_LIMITED, BUSY, LIMIT_REACHED, TOO_MANY_MESSAGES, …
}
// ConversationStore / useConversation: createOptions: { participant, anonymous: true }
```

What the platform guarantees (don't re-implement it in the site):
- **Session home** `/visitors/<key>` in `raisin:access_control`, keyed by a
  256-bit secret the SDK keeps in sessionStorage (reload = same chat). Only
  that session reads it; nobody but the server writes it; no role grant can
  widen that. Another visitor, a user, a broad `read` role: nothing.
- **Delivery** is structural: a conversation's events go to the one WS
  connection that proved its session; the SSE endpoint needs read access.
- **Tool rights**: a visitor's turn runs tools under `anonymous.tool_roles`
  (else the agent's `roles`), NEVER as the visitor or the system, whatever
  `execution_context` says; no grant → tools refused. A tool that writes
  (e.g. files an inquiry) uses its own `execution_context: system` function
  and validates its input. Memory and delegation tools are not offered.
- **Expiry**: `purge-expired-visitors` (every 15 min) deletes expired sessions
  and their conversations; cost records + tool audit stay on the agent side.
- Rate limits / leases are per server process; the IP is the last
  `X-Forwarded-For` entry — run behind a proxy that sets it.

Full reference and the migration from a service identity:
`docs/developer/messaging/anonymous-visitors.md`.

## Tokens, cost, safety

- Every AI call writes a `raisin:AICostRecord` child (input/output tokens,
  model, provider) under the assistant reply in the `ai` workspace — your
  usage dashboard is one SQL query away.
- `max_conversation_tokens` sizes the model's context window per turn
  (history is trimmed to fit); it is NOT a hard stop for signed-in users —
  cap a run with `run_budgets` (`max_total_tokens`, `max_model_calls`).
  For anonymous visitors, `anonymous.max_conversation_tokens` IS a hard
  per-conversation budget (see below). `auto_compact` summarizes old turns
  into a persisted `raisin:AICompaction` node so facts survive but tokens
  don't.
- Configure providers per tenant with the CLI:
  `raisindb ai provider set groq --api-key-stdin && raisindb ai provider test groq`.
- Repeatable proof scripts in `examples/shiftboard/`: `npm run smoke`
  (chat+tools+tokens), `npm run negotiation-test` (3-party coordination),
  `npm run compaction-test` (budget + compaction). Copy their patterns for
  your own apps' CI.
