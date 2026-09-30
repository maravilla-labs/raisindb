# Anonymous Visitors: Agent Chat Without Login

A public website can run an agent chat **straight from the browser**, over the
WebSocket SDK. It needs no site server endpoint, no service identity and no
credential in the page. The browser talks to RaisinDB directly, so chat load
stays off the site's rendering pipeline.

```
browser (anonymous WS session)
  │ visitor_chat_start {agent}          → session secret, conversation id
  │ visitor_chat_send {conversation, text}
  ▼
RaisinDB (checks the agent's anonymous limits, then writes AS THE SYSTEM)
  /visitors/<key>/outbox/msg-…          (raisin:access_control)
  │ process-chat → handle-chat          (the ordinary pipeline)
  ▼
/agents/<name>/inbox/chats/<conv>       (ai) → agent-handler → agent run
  │ tools run under the agent's ANONYMOUS TOOL GRANT
  ▼
reply → /visitors/<key>/inbox/chats/<conv>
events (text_chunk, tool_call_*, done) → this WebSocket connection only
```

## Enable it on an agent

Anonymous visitors are refused unless the agent says otherwise:

```yaml
node_type: raisin:AIAgent
properties:
  system_prompt: …
  provider: marvel
  model: …
  tools:
    - /lib/site/search-pages
    - /lib/site/find-flights
  anonymous:
    enabled: true                       # the flag (or: allow_anonymous: true)
    allowed_origins:                    # the page's Origin header; empty = any
      - https://www.example.com
    tool_roles: [site_assistant]        # what the TOOLS may do (see below)
    max_messages: 20                    # visitor messages per conversation
    max_message_chars: 2000
    max_conversation_tokens: 50000      # token budget per conversation
    max_conversations: 5                # per visitor session
    rate_per_session_per_minute: 6
    rate_per_ip_per_minute: 30
    max_concurrent_turns: 1             # per session
    turn_lease_seconds: 120             # a hung turn frees its slot after this
    session_ttl_hours: 24               # purge after this long without activity
```

Add `stream_tool_results: true` (a top-level agent property) when the page
renders tool results itself, for example as cards: see
[Tool results](#tool-rights).

Every key except `enabled` has the default shown. `allow_anonymous: true`
alone enables the defaults. Changing the agent takes effect on the next
message: turning the flag off stops a running chat at its next send.

Anonymous access must also be enabled for the repository (the same setting
that lets anonymous sessions read public content). No role needs to grant
anonymous callers anything for chat.

## The client

```typescript
import { RaisinClient } from '@raisindb/client';

const client = new RaisinClient('wss://db.example.com/ws/website');
await client.connect();                       // no login: an anonymous session
const db = client.database('website');

// Starts a new conversation, or resumes this tab's (sessionStorage).
const convo = await db.conversations.startAnonymous('/agents/website-assistant');

for await (const ev of db.conversations.sendMessage(convo.conversationPath, 'Wann fliegt LH123?')) {
  if (ev.type === 'text_chunk') append(ev.text);
  if (ev.type === 'tool_call_started') showTool(ev.functionName);
  if (ev.type === 'failed') showLimit(ev.code);   // RATE_LIMITED, BUSY, LIMIT_REACHED, …
}

const history = await db.conversations.getMessages(convo.conversationPath);
```

Or with the stores and the framework adapters. `anonymous` is the only
difference from a logged-in chat:

```typescript
const store = new ConversationStore({
  database: db,
  createOptions: { participant: '/agents/website-assistant', anonymous: true },
});
// React: useConversation(React, { database: db, createOptions: { …, anonymous: true } })
```

- **Resume.** The session secret and the last conversation id are kept in
  `sessionStorage`, so one tab is one session. Pass `store: false` to keep
  nothing, or your own `VisitorSessionStore`. `newConversation: true` starts
  another conversation in the same session, and
  `db.conversations.forgetAnonymous(agent)` forgets the session.
- **Reconnects.** A new WebSocket connection must prove the session again.
  The SDK does this for every conversation after a reconnect, and a send that
  finds the connection replaced re-proves once and retries.
- **Limits and refusals.** A streamed turn ends with
  `{type: 'failed', code}`, and `createUserMessage` throws a
  `VisitorChatError` with `code`:

  | code | meaning |
  |---|---|
  | `ANONYMOUS_NOT_ALLOWED` | the agent does not accept visitors (or does not exist) |
  | `ORIGIN_NOT_ALLOWED` | the page's origin is not in `allowed_origins` |
  | `RATE_LIMITED` | per-session or per-IP rate |
  | `BUSY` | a turn of this conversation (or `max_concurrent_turns`) is still running |
  | `TOO_LONG` / `EMPTY_MESSAGE` | message size |
  | `TOO_MANY_MESSAGES` | `max_messages` reached: start a new conversation |
  | `LIMIT_REACHED` | `max_conversation_tokens` used up: start a new conversation |
  | `CONVERSATION_LIMIT` | `max_conversations` reached for this session |
  | `SESSION_EXPIRED` / `SESSION_MISMATCH` | the session is gone, or belongs to another agent |
  | `NOT_ANONYMOUS` | a signed-in connection: use `conversations.create` |

## How the session is keyed

`visitor_chat_start` mints a **session secret** of 256 random bits. The browser
keeps it. The session's **key** is the first 128 bits of `SHA-256(secret)`, and
the session home is `/visitors/<key>` in `raisin:access_control`. The key shows
up in paths and participant ids (`visitor:<key>`). Knowing it proves nothing:
only the secret binds a connection to the home.

Nothing is written at start. On the first message the server creates the
home: a `raisin:VisitorSession` with `agent_path`, `expires_at`,
`message_counts` and `origin`. Each later message moves `expires_at` forward.
A session belongs to one agent.

Conversation ids (`vchat-<random>`) are minted by the server. The id is also
the event channel and the agent-side thread name. A connection may only send
to and receive from the conversations of the session it proved.

## Isolation (row-level security)

The visitor zone (`/visitors/**` in `raisin:access_control`) is decided before
any role grant, in every RLS entry point (reads, SQL scans, event forwarding,
creates, updates, deletes):

- The session bound to a home may **read** it and everything below it. This is
  how `getMessages` works over SQL.
- **Nobody else** may read it: not another visitor, not a logged-in user, and
  not a role that grants `read` on the whole workspace.
- **Nobody** writes it except the system. That includes the visitor itself, so
  no limit can be bypassed by writing an outbox node directly.
- The system and `system_admin` are unaffected.

A conversation belongs to the two parties that started it. `handle-chat`
refuses a message that would merge a second person into an existing agent-side
conversation (a reused thread id). Before, such a message took over the agent's
replies.

**Delivery.** A visitor conversation's events are forwarded to the one
WebSocket connection that proved its session. The conversation SSE endpoint
(`/api/conversations/{repo}/events`) requires read access to the conversation,
so it cannot be used to listen in either.

## Tool rights

In an agent run, tools act with the rights of the person the run acts for.
A visitor has none worth acting with, and "system" would give the open internet
the repository. So **a visitor's turn always runs its tools under the agent's
anonymous tool grant**, whatever `execution_context` says:

- `anonymous.tool_roles` / `anonymous.tool_groups`, else the agent's own
  `roles` / `groups`;
- no grant, or a grant that resolves to `system_admin`: the tools are refused
  (fail closed).

Grant exactly what the tools need. For example, a read-only role over the
pages, documents and data the assistant may cite:

```yaml
node_type: raisin:Role
properties:
  role_id: site_assistant
  permissions:
    - { workspace: stories, path: "/site/**", operations: [read] }
    - { workspace: assets,  path: "/site/**", operations: [read] }
    - { workspace: flightdata, path: "/**",   operations: [read] }
```

A tool that must write, such as one that files an inquiry, does so through its
own function rights (`execution_context: system` on its `raisin:Function`) and
validates its input itself, as any system function must.

In a visitor's run the memory tools (`remember`, `forget`, `read-user-context`)
and the delegation tools are not offered, and the agent's `node_dev` grant does
not apply.

## Cost and budgets

- `max_conversation_tokens` is checked at every send, against the agent-side
  `total_tokens_used`. It also caps the run itself: the run's
  `max_total_tokens` is what is left of the budget, and a visitor's run
  **fails** instead of pausing when it runs out. A paused run would be resumed,
  with raised budgets, by the next message.
- `max_model_calls_per_turn` (optional) caps model calls per turn.
- Cost records (`raisin:AICostRecord`) and the tool-call audit stay on the
  agent side (`ai:/agents/<name>/inbox/chats/<conv>`), as for every chat.

## Expiry

`/lib/messaging/visitor/purge-expired` runs every 15 minutes (trigger
`purge-expired-visitors`). It deletes the sessions whose `expires_at` has
passed, together with their conversations. The agent side stays: cost records,
tool audit, and the agent's copy of the transcript. Clean that up with your own
retention policy if the transcripts must not outlive the session.

## Limits of the limits

- Rate limits and turn leases are held **per server process**. A visitor's
  WebSocket lives on one server. A cluster multiplies the per-IP allowance by
  its size, while the token budget and message count are persisted and exact.
- The client IP is the **last** `X-Forwarded-For` entry (the nearest proxy),
  else `X-Real-IP`. Put RaisinDB behind a proxy that sets it. Without one, all
  visitors share the `unattributed` IP bucket.
- `allowed_origins` checks the browser's `Origin` header on the WebSocket
  upgrade. That stops other websites from embedding your assistant; it does not
  stop a script that sends any header it likes. The rate and cost limits are
  what bound such a caller.

## Migrating from a service identity

Sites that ran every visitor chat through one service identity behind a server
endpoint (a login in the site's server, a signed cookie with the conversation
path, SSE re-emitted to the browser) can switch as follows:

1. **The agent.** Add `anonymous: {enabled: true, allowed_origins: [...],
   tool_roles: [<the role the service identity held for the tools>]}` and
   tune the limits. Drop `execution_context` workarounds: the anonymous tool
   grant replaces "the service identity's roles".
2. **The browser.** Replace `POST /api/chat` with
   `db.conversations.startAnonymous(agent)` + `sendMessage` (or a
   `ConversationStore` with `anonymous: true`). The `sessionStorage` token and
   the signed cookie go away: the SDK keeps the session secret.
3. **Tool results and post-processing.** A visitor cannot read the agent's
   side of the conversation, where the tool results are stored. Set
   `stream_tool_results: true` on the agent: each `tool_call_completed` event
   then carries its tool's `result` (up to 20000 JSON characters, else
   `{truncated, preview}`), and the browser renders its cards (flights,
   sources) from that. Anything else the site did to a finished answer on its
   server (claim checks, link whitelists) moves into the browser, or into the
   agent's tools and prompt. RaisinDB delivers what the agent wrote.
4. **Actions with side effects** (for example "send this inquiry") become a
   tool with its own function rights, or stay a separate site endpoint that
   takes the conversation id and reads the transcript with a server key.
5. **Remove** the service identity and its home, its rate-limit and busy-lock
   code, and the site's cleanup job: RaisinDB purges expired sessions.
