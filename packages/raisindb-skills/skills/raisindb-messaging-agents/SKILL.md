---
name: raisindb-messaging-agents
description: "Build AI chat and messaging on RaisinDB: AI agents with tools, the inbox/outbox chat pipeline, agents that proactively message users and coordinate between them, human-in-the-loop task UIs, chatbox frontends with the JS SDK, and token safeguards (budgets, auto-compaction). Use this whenever the user wants a chatbot, AI assistant, agent with tools, notifications, an inbox, agent-to-user messaging, multi-user coordination ('agent asks staff one by one'), or anything involving raisin:AIAgent, conversations, or message nodes — even if they just say 'add AI to my app'."
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

Function-runtime traps (NOT the client SDK):
- `raisin.sql.query(...)` returns the **row array directly** (the client's
  `executeSql` returns `{rows}`); use `raisin.sql.execute` for DML.
- Tool calls receive an injected `__raisin_context` argument
  (`agent_name`, `conversation_path`, `sender_id`, ...) — use it to know who
  is talking and from which thread.
- Return **graceful errors as data** (`{error: "...pick another candidate"}`)
  instead of throwing — the model self-corrects in the same turn.

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

## Tokens, cost, safety

- Every AI call writes a `raisin:AICostRecord` child (input/output tokens,
  model, provider) under the assistant reply in the `ai` workspace — your
  usage dashboard is one SQL query away.
- `max_conversation_tokens` enforces a hard budget per conversation
  (`finish_reason: budget_exceeded`, no provider call); `auto_compact`
  summarizes old turns into a persisted `raisin:AICompaction` node so facts
  survive but tokens don't.
- Configure providers per tenant with the CLI:
  `raisindb ai provider set groq --api-key-stdin && raisindb ai provider test groq`.
- Repeatable proof scripts in `examples/shiftboard/`: `npm run smoke`
  (chat+tools+tokens), `npm run negotiation-test` (3-party coordination),
  `npm run compaction-test` (budget + compaction). Copy their patterns for
  your own apps' CI.
