# Agent Chat for Anonymous Visitors

A public website can offer an AI assistant to visitors who are not logged in.
The chat runs straight from the browser over the WebSocket SDK. There is no
server endpoint of the site's own, no shared "chat user", and no credential in
the page.

## 1. Allow it on the agent

```yaml
node_type: raisin:AIAgent
properties:
  # … system_prompt, provider, model, tools …
  anonymous:
    enabled: true
    allowed_origins: [https://www.example.com]
    tool_roles: [site_assistant]
    max_messages: 20
    max_conversation_tokens: 50000
  stream_tool_results: true   # optional: tool results in tool_call_completed events
```

Agents without `anonymous.enabled` (or `allow_anonymous: true`) refuse
visitors. Anonymous access must be enabled for the repository.

## 2. Grant the tools exactly what they need

A visitor's turn runs its tools under `anonymous.tool_roles`, never as the
visitor and never as the system, whatever the agent's `execution_context` is.
A read-only role over the public content is typical:

```yaml
node_type: raisin:Role
properties:
  role_id: site_assistant
  permissions:
    - { workspace: content, path: "/site/**", operations: [read] }
```

## 3. Chat from the page

```typescript
const client = new RaisinClient('wss://db.example.com/ws/website');
await client.connect();
const db = client.database('website');

const convo = await db.conversations.startAnonymous('/agents/website-assistant');
for await (const ev of db.conversations.sendMessage(convo.conversationPath, 'Hi!')) {
  if (ev.type === 'text_chunk') append(ev.text);
  if (ev.type === 'failed') showLimit(ev.code);
}
```

`ConversationStore` and `useConversation` take
`createOptions: { participant, anonymous: true }`.

## What the platform guarantees

- **Isolation.** Each visitor session has an ephemeral home
  (`/visitors/<key>` in `raisin:access_control`). Only that session can read
  it, no role grant can widen that, and nobody but the server writes it.
  The session is keyed by a secret the browser keeps in `sessionStorage`.
- **Delivery.** A conversation's events reach only the WebSocket connection
  that proved its session.
- **Limits.** Message size and count, a token budget per conversation, rate
  limits per session and per IP, and one turn at a time. The turn is held by a
  lease that a hung turn frees on its own.
- **Expiry.** Sessions are purged after `session_ttl_hours` without activity
  (default 24). Cost records and tool audit stay on the agent side.

The full reference, including every limit, the refusal codes and a migration
guide for sites that used a service identity, is in
`docs/developer/messaging/anonymous-visitors.md`.
