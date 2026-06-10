# Shiftboard

A small end-to-end demo of building an AI-assisted app on RaisinDB with the
`@raisindb/client` SDK. A cafe manager plans weekend shifts on a live board and
chats with an AI agent that reads and updates the same data through tools.

What it demonstrates:

- **AI chat with tool calls** — a Groq-backed agent (`/agents/shift-planner`)
  with four custom tool functions (`list-shifts`, `list-staff`, `assign-shift`,
  `message-staff`) plus the builtin weather tool. The frontend streams the
  reply and shows tool-call badges while tools run.
- **Agent coordinates with staff over chat** — ask the agent to *fill* a
  shift and it does not assign anyone right away: it chats with the staff
  members themselves (`message-staff`), handles declines by asking the next
  available candidate, assigns only after someone confirms, and then reports
  back to the manager. See the scenario below.
- **Durable workflow variant** — the same coordination as a `raisin:Flow`
  (`/flows/fill-shift`): the workflow engine asks each candidate via an
  **inbox approval task** (accept/decline buttons, deadline, full audit
  trail) instead of chat, assigns the first accepter and notifies the
  manager. The agent starts it conversationally with its `start-shift-fill`
  tool when the manager asks for a tracked process. See the workflow
  scenario below.
- **Planner view: plans + workflows composed** — a second, plan-enabled
  agent (`/agents/shift-coordinator`, `task_creation_enabled` +
  `execution_mode: approve_then_auto`) fills the whole weekend board: it
  proposes a **plan** with one task per open shift that the manager must
  **approve**; each approved task then calls `start-shift-fill`, i.e.
  starts one durable fill-shift workflow. See the Planner section below.
- **Live node subscriptions** — when the agent assigns a shift, the shift node
  in workspace `staffing` is updated server-side and the matching board card
  updates in place (with a highlight flash) via a WebSocket node subscription.
  No polling, no manual refresh.
- **Inbox notifications** — a node subscription on the logged-in user's home
  inbox (`raisin:access_control` workspace) drives the bell badge and toasts.
- **Server-side rendering** — the frontend is a SvelteKit app (adapter-node).
  Auth lives in httpOnly cookies; the board and the chat history are fetched
  via `RaisinHttpClient` (SQL over HTTP) during SSR, so the first response is
  complete HTML. After hydration the same data is kept live over WebSocket.
  `frontend/ssr-check.sh` proves it with curl (no JavaScript).

## Setup

Prerequisites: a running raisin-server (default `http://localhost:8081`, dev
mode with tenant `default`) and a Groq provider configured for the tenant
(admin console → AI settings).

### Recommended: install as a package via the raisindb CLI

The app ships as an installable package in `package/` (workspace `staffing`
with seed shifts/staff, the four tool functions, and the shift-planner agent
incl. its home folder in the `ai` workspace). Works against remote instances
and in CI — `ci.sh` scripts the whole flow:

```bash
cd examples/shiftboard

# Non-interactive (CI): env vars win over .raisinrc
RAISINDB_SERVER=http://localhost:8081 REPO=shiftboard ./ci.sh
# (ci.sh creates the repo, deploys + installs the package, registers the demo
#  identity user, checks the tenant Groq config, and runs the smoke test.
#  RUN_SMOKE=0 to skip the Groq-spending smoke run.)

# Or step by step:
raisindb login --server http://localhost:8081 --username admin --password '...'
raisindb package deploy ./package --repo shiftboard --install
```

What a package cannot do (repo creation, identity users, tenant AI keys) is
handled by `ci.sh` / `setup.mjs` — see `GAPS.md`.

### Development setup (live edit loop)

The canonical package-based dev loop — deploy once, then sync changes live:

```bash
# 1. Authenticate (non-interactive; or set RAISINDB_SERVER + RAISINDB_TOKEN)
raisindb login --server http://localhost:8081 --username admin --password '...'

# 2. Create the target repository
raisindb repo create myshiftboard --exists-ok

# 3. First install: build the .rap, upload it, install it, and wait for the
#    final status ('installed' = success; 'failed' prints the error detail)
raisindb deploy ./package --repo myshiftboard --install

# 4. Develop: watch the package dir and push each changed node live
raisindb sync ./package --repo myshiftboard --watch
```

While `sync --watch` runs:

- Editing a **function source** (e.g.
  `package/content/functions/lib/shiftboard/list-shifts/index.js`) updates
  the asset node's inline `code` property — the next function call runs the
  new code, no reinstall needed.
- Editing a **`.node.yaml`** or a named node YAML (e.g.
  `content/staffing/shifts/fri-evening.yaml`) PUTs the node's properties.
- Editing **`manifest.yaml`**, **`workspaces/`**, or **`nodetypes/`** cannot
  be hot-synced — the watcher prints a hint to re-run
  `raisindb deploy ./package --repo <repo> --install`.

Output is the interactive watch UI on a terminal, or plain log lines when
piped/CI (non-TTY).

Install lifecycle (visible in `raisindb package list --repo <repo>` and on
the `raisin:Package` node's `status` property):
`processing` (upload accepted) → `uploaded` → `installing` → `installed`
or `failed` — on failure the CLI prints the error detail (from the package
node or the install job record). Builtin packages installed at repo
creation have no `status` property.

### Fallback: imperative setup script (no CLI, dev server)

```bash
cd examples/shiftboard

# 1. Install + create everything on the server (idempotent, re-runnable):
#    repo `shiftboard`, workspace `staffing` with shift/staff nodes, the tool
#    functions, the agent, its home folders, and the demo identity user.
#    Function sources are shared with the package (single source of truth:
#    package/content/functions/lib/shiftboard/<name>/index.js).
npm install
node setup.mjs

# 2. Optional: prove the backend pipeline headlessly (costs a few Groq tokens)
npm run smoke

# 2b. Optional: prove the agent<->staff chat coordination end to end
#     (manager + two staff users on separate sessions; ~6 Groq turns)
npm run negotiation-test

# 2c. Prove the DURABLE workflow variant end to end (no LLM, free):
#     /flows/fill-shift asks staff via inbox accept/decline tasks
npm run workflow-test

# 3. Frontend (SvelteKit, SSR)
cd frontend
npm install
npm run dev        # dev server with SSR, http://localhost:5175

# Production build (Node server via adapter-node):
npm run build
npm run start      # ORIGIN=http://localhost:5175 PORT=5175 node build
./ssr-check.sh     # login via curl + assert shift titles in the raw HTML
```

Log in with `planner@example.com` / `Planner12345!` (prefilled). The staff
chat accounts are `anna@example.com` and `cara@example.com` with password
`Staff12345!`.

## Scenario: the agent coordinates with staff

The shift-planner is not just a board operator — it negotiates with the
team. The staff nodes in workspace `staffing` carry an `email` property and
Anna + Cara have real chat accounts, so the agent can connect board staff to
chat addresses (`list-staff` reports `reachable: true` for them).

1. The **manager** writes: *"Please fill the Saturday morning shift. Ask the
   staff via chat first - only assign someone who confirms."*
2. The **agent** checks the board (`list-shifts`, `list-staff`), picks an
   available + reachable candidate and sends her a chat message via
   `message-staff` ("Hi Anna, can you take the Saturday Morning shift,
   08:00-14:00, Terrace, /shifts/sat-morning?"). The message is dropped into
   the agent's outbox in the `ai` workspace; the builtin messaging pipeline
   delivers it into Anna's inbox conversation (creating it if needed).
3. **Anna declines** in her own chat. Her reply is delivered into the
   agent's inbox thread, which re-triggers the agent: it asks the next
   candidate (mentioning that Anna declined) — never the same person twice.
4. **Cara accepts** → the agent runs `assign-shift` (the board card updates
   live) and notifies the manager with a `message-staff` confirmation in the
   original manager thread.

`negotiation-test.mjs` scripts exactly this proof with three separate SDK
sessions (manager, Anna, Cara) and asserts every hop, the final node state
(`/shifts/sat-morning` filled by the accepter) and the manager confirmation:

```bash
npm run negotiation-test                      # repo 'shiftboard' (default)
RAISIN_REPO=shiftboard2 npm run negotiation-test
```

## Workflow variant: durable coordination via inbox tasks

Tutorial part 5: the same fill-a-shift coordination, but **durable**. Chat
threads are great for conversation, but the coordination state lives in the
LLM's context. The workflow variant moves the *process* into the engine — a
`raisin:Flow` node (`/flows/fill-shift`, designer format in
`package/content/functions/flows/fill-shift/.node.yaml`) that survives
restarts, enforces deadlines, and leaves an audit trail (every ask is an
`raisin:InboxTask` node with who/when/what answered):

1. **`pick_candidates`** (function step → `pick-candidates`): loads the
   shift, filters staff who are *available on that day* AND *reachable*
   (their email belongs to a registered identity user), and resolves each
   one to their identity-user home path (`/users/internal/anna-at-…`) — the
   inbox-task assignee.
2. **`ask_each`** (loop container over `${steps.pick_candidates.candidates}`,
   item `candidate`): the body is ONE human task (`ask_candidate`,
   `task_type: approval`, `assignee: ${candidate.user_path}`) with
   Accept/Decline options and a 5-minute deadline (`due_in_seconds: 300`,
   `timeout_edge` → step 3). The loop's
   `until: steps.ask_candidate.action == "accept"` stops asking as soon as
   someone accepts — candidates are asked **one at a time**, never broadcast.
3. **`resolve_accepter`** (function step → `resolve-accepter`): pairs the
   loop output (`${steps.ask_each.results}` — one human response per asked
   candidate, in order) back with the candidate list: who accepted, who
   declined, and the outcome summary.
4. **`assign_or_report`** (or-container): rule
   `steps.resolve_accepter.accepted == true` routes into the `assign_shift`
   function step (the same `assign-shift` tool the chat agent uses); no
   match (nobody accepted) skips the container.
5. **`notify_manager`** (function step → `message-staff`,
   `continue_on_fail: true`): the manager always gets the outcome as a chat
   message — "Cara accepted … Declined before that: Anna." or "Could not
   fill …".

The agent bridges both worlds with its **`start-shift-fill`** tool
(`raisin.flows.run('/flows/fill-shift', { shift_path })` in the function
runtime): say *"fill the Sunday evening shift using tasks"* / *"with a
workflow"* and the agent starts the flow instead of chatting with staff
itself, then reports the instance id.

`workflow-test.mjs` proves the whole loop **without any LLM** (pure engine,
free to run): reset `/shifts/sun-evening` → start the flow via the
`FlowClient` → Anna's inbox gets the approval task (and Cara's does NOT
yet) → Anna declines via `POST /api/inbox/{repo}/tasks/{id}/complete` →
Cara gets her task → Cara accepts → flow completes, the shift node is
`filled`/`Cara`, and the planner's inbox conversation has the outcome
message:

```bash
npm run workflow-test                         # repo 'shiftboard' (default)
RAISIN_REPO=shiftboard2 npm run workflow-test
```

The same scenario also runs as engine-level CI against the *shipped* YAML:
`cargo test -p raisin-flow-runtime --test e2e_flows shiftboard`.

## Planner: plans + workflows composed

The **Planner** tab (header toggle `Board | Planner`) is the composition
demo: the AI **plan/task system** on top of the **durable workflow** above.
A second agent, `/agents/shift-coordinator`
(`package/content/functions/agents/shift-coordinator/.node.yaml`), is
configured with `task_creation_enabled: true` and
`execution_mode: approve_then_auto`, and gets the builtin planning tools
(`/lib/raisin/ai/create-plan`, `add-task`, `update-task`,
`get-plan-status`) next to `list-shifts`, `list-staff` and
`start-shift-fill`. It never messages staff and never assigns anyone — it
only plans and starts workflows.

1. The manager (Planner tab, a **separate conversation** from the Board
   chat) writes: *"Fill all open weekend shifts."*
2. The coordinator checks the board (`list-shifts status=open`) and calls
   `create-plan` with **one task per open shift** (task title = shift title
   + path, e.g. `Fill Saturday Morning (/shifts/sat-morning)`). Because the
   agent runs in `approve_then_auto`, the plan lands as a `raisin:AIPlan`
   node in `pending_approval` — nothing executes yet.
3. The proposal renders as a card in the **plan panel** (`PlanPanel.svelte`,
   driven by the SDK's deterministic `plans` projection on
   `ConversationStore`) with the ordered tasks and **Approve / Reject**
   buttons (`store.approvePlan` / `store.rejectPlan`, reject takes optional
   feedback).
4. On approve, the agent executes the tasks: for each one it calls
   `start-shift-fill`, which starts a fill-shift workflow instance and
   **returns immediately**. The task is then marked completed.
5. The board (always visible, left) fills **live** as staff accept the
   inbox approval tasks the workflows created — card by card.

**The seam, stated honestly:** `start-shift-fill` is fire-and-forget, so a
plan task flipping to *completed* means *"the workflow for this shift was
STARTED"* — not *"the shift is filled"*. The plan completes when all
workflows are running; whether and when each shift actually fills is up to
the staff answering their inbox tasks (the agent's system prompt makes it
report exactly that). For this demo that seam is a feature: you can watch
the plan complete in seconds and then see the board fill asynchronously.
A tighter coupling (task completes only when the flow completes) would
need the agent to wait on flow instances — a different, blocking design.

Two client-side event-ordering quirks are handled in
`frontend/src/lib/stores/planner.svelte.ts` with reload polling (the same
approach as the admin console's Test Chat): the safety-net `done` event
(finish reason `awaiting_plan_approval`) can arrive **before** the
async-delivered `ai_plan` card, and after approval the lifecycle messages
(`ai_plan` / `ai_task_update`) arrive without reliable live events on the
user's subscription. Those lifecycle messages are also **filtered out of
the chat transcript** (their content is just the task title) — they belong
to the plan panel.

`planner-tab-check.mjs` proves the whole composition headlessly
(Playwright + inbox API; ONE Groq run, budget-asserted):

```bash
RAISIN_REPO=shiftboard2 npm run planner-tab-check
```

reset all shifts open → login → Planner tab → "Fill all open weekend
shifts" → plan card `pending_approval` with one task per open shift →
Approve → all tasks complete (= 5 workflows started, board still honest:
nothing filled yet) → anna/cara really have pending inbox tasks (API) →
ONE task accepted via the API → that shift flips to `filled` on the open
page **live** → demo state restored (remaining flow instances cancelled,
board re-seeded).

Configuration (frontend): `VITE_RAISIN_WS_URL` (default
`ws://localhost:8081/ws/shiftboard`; multi-tenant operators can use
`ws://host/sys/{tenant}/{repo}`) and `VITE_RAISIN_REPO`
(default `shiftboard`), e.g. via `frontend/.env.local` — baked in at build
time. The SSR server derives its HTTP base from the WS URL; override at
runtime with `RAISIN_HTTP_URL`. When running the built server, set `ORIGIN`
to the public URL (SvelteKit's CSRF check compares it against form-post
Origin headers).

## Human-in-the-loop tasks in your own UI

When a workflow needs a human decision (a `human_task` step, or
`raisin.tasks.create` in a function), the engine creates a
**`raisin:InboxTask` node** under the assignee's home inbox in the
`raisin:access_control` workspace and pauses. The point: tasks are just
nodes the logged-in user can read — no special task UI framework needed.
The Shiftboard frontend renders them as a card list above the chat
(`TaskPanel.svelte`), with **one button per entry in the task's `options`
array** — completely generic, nothing shift-specific.

The node the engine writes (also what `GET /api/inbox/{repo}?status=pending`
returns, plus `id` + `path`):

```json
{
  "path": "/users/internal/planner-at-example-com/inbox/task-approve-1781091873086",
  "task_type": "approval",
  "title": "Cover Sunday Evening?",
  "description": "Dave called in sick — approve the replacement plan?",
  "assignee": "/users/internal/planner-at-example-com",
  "status": "pending",
  "priority": 4,
  "due_at": "2026-06-11T11:24:57Z",
  "options": [
    { "value": "accept",  "label": "Accept",  "style": "success" },
    { "value": "decline", "label": "Decline", "style": "danger" }
  ],
  "flow_instance_id": "28fd62eb-…",
  "step_id": "approve"
}
```

Three pieces wire it into the app (`lib/stores/tasks.svelte.ts`):

1. **SSR seed** — `+page.server.ts` also loads the pending list, so the
   cards are part of the first HTML byte:
   `new InboxApi(httpBase, REPO, authManager).listTasks({ status: 'pending' })`.
2. **Live updates** — the app's single inbox subscription (the same one
   that drives the notification bell, `notifications.svelte.ts`) now also
   listens for `node:updated` and forwards `raisin:InboxTask` events to the
   task store: created+pending upserts a card, any other status removes it.
3. **Complete** — each option button posts the chosen value back with the
   *user's own* bearer; the server validates the caller is the assignee,
   flips the node to `completed` and resumes the waiting flow:

```ts
// tasks.svelte.ts — optimistic removal with rollback on error
async complete(taskId: string, value: string): Promise<void> {
  const removed = this.tasks.find((t) => t.id === taskId);
  this.tasks = this.tasks.filter((t) => t.id !== taskId);
  try {
    // POST /api/inbox/{repo}/tasks/{taskId}/complete  { response: { action } }
    await getInbox().completeTask(taskId, { action: value });
  } catch (err) {
    this.tasks = [removed, ...this.tasks];          // roll back
    this.error = err instanceof Error ? err.message : String(err);
  }
}
```

The flow sees the decision as `__human_response.action` (plus
`completed_by` and `task_path`). `frontend/inbox-task-check.sh` proves the
loop with curl only: deploy + run a probe flow, assert the task card is in
the server-rendered HTML, complete it via the same endpoint the buttons
use, assert the status flip and flow resume, then clean up.

Two current server-side caveats (both observed against this demo repo):

- The flow engine writes task nodes through the storage layer directly
  (`create_deep_node`), which does not publish `node:created`/`node:updated`
  events — only `NodeService` writes do (chat messages, board updates). The
  task store therefore adds a slow 30s poll as a fallback; the subscription
  takes over as soon as the engine emits node events.
- `raisin:User.initial_structure` makes every user's `inbox` a
  `raisin:MessageFolder`; its builtin `allowed_children` did not include
  `raisin:InboxTask`. Task *creation* skips that check, but the completion
  *update* enforces it — completing a task failed with "not allowed as a
  child of 'raisin:MessageFolder'". Fixed: the global nodetype now allows
  `raisin:InboxTask`, and `setup.mjs` patches the nodetype per repo
  (`ensureInboxTasksAllowed`) so repos created by older builds work too.

## Architecture

```
Browser (SvelteKit + Svelte 5, hydrated)
  │  WebSocket (auth, SQL, node subscriptions)  +  HTTP/SSE (chat streaming)
  ▼
SvelteKit Node server (SSR)
  │  httpOnly cookie session; RaisinHttpClient: SQL over HTTP for the
  │  board + chat history (login/refresh via /auth/{repo}/*)
  ▼
raisin-server
  ├─ user message node  →  messaging pipeline  →  agent inbox (ai workspace,
  │                                                /agents/shift-planner/inbox/chats)
  ├─ agent-handler function picks up the message, runs the Groq model with the
  │  agent's tools; tools query/update nodes in workspace `staffing`
  └─ events fan out
       ├─ SSE chat events → ConversationStore (text chunks, tool calls, done)
       ├─ node:updated on /shifts/* → board subscription → card flash
       └─ node:created in the user's inbox → notification bell
```

The key point: chat, board, and notifications are all just **nodes and node
events**. The agent's `assign-shift` tool updates a node; the frontend sees it
through the same subscription API any other writer would trigger.

## Feature → SDK API

| Feature | SDK API |
|---|---|
| Login (form action, server) | `POST /auth/{repo}/login` → tokens in httpOnly cookies |
| Session restore on reload | cookies in `hooks.server.ts`, refresh via `POST /auth/{repo}/refresh` |
| WS auth after hydration | `client.authenticate({ type: 'jwt', token })` (token from layout data) |
| SSR board + staff load | `RaisinHttpClient.database(repo).executeSql(...)` (SQL over HTTP) |
| SSR chat history | `ConversationManager.list({ type: 'ai_chat' })` + `.getMessages(path)` |
| Connection status dot | `createConnectionAdapter(client)` (wraps `client.onConnectionStateChange` / `onReadyStateChange`) |
| Live shift card updates | `db.workspace('staffing').events().subscribe({ path: '/shifts/*', event_types: ['node:updated'], include_node: true }, cb)` |
| Inbox notification bell | `db.workspace('raisin:access_control').events().subscribe({ path: user.home + '/inbox/**', event_types: ['node:created', 'node:updated'], include_node: true }, cb)` |
| Human task panel (list + SSR seed) | `InboxApi.listTasks({ status: 'pending' })` (`GET /api/inbox/{repo}`) |
| Complete a human task | `InboxApi.completeTask(taskId, { action })` (`POST /api/inbox/{repo}/tasks/{id}/complete`) |
| Create chat lazily | `new ConversationStore({ database, createOptions: { participant: '/agents/shift-planner' } })` |
| Chat state (messages, streaming text, tool calls, errors) | `ConversationStore.subscribe(snapshot => …)` |
| Send a message | `ConversationStore.sendMessage(text)` |
| Plan proposal/progress cards | `snapshot.plans` (deterministic projection from `ai_plan` / `ai_task_update` messages) |
| Approve / reject a plan | `ConversationStore.approvePlan(planPath)` / `.rejectPlan(planPath, feedback)` |

## Frontend code layout

```
frontend/src/
  hooks.server.ts                    cookie session: parse, refresh, locals
  routes/+layout.server.ts           validate token, pass { user, token }
  routes/+page.server.ts             SSR load (board + chat history + pending
                                     tasks) and ?/login & ?/logout form actions
  routes/+page.svelte                seeds stores from SSR data, wires live layer
  lib/server/raisin.ts               RaisinHttpClient factory, auth cookies,
                                     SSR inbox task list
  lib/raisin.ts                      browser WS client singleton, InboxApi, env
  lib/board-data.ts                  shared SQL + row mapping (server & client)
  lib/stores/session.svelte.ts       user state (runes)
  lib/stores/connection.svelte.ts    connection dot state
  lib/stores/board.svelte.ts         SSR seed + live node subscription
  lib/stores/notifications.svelte.ts inbox subscription, badge, toasts; feeds
                                     the task store
  lib/stores/tasks.svelte.ts         human task panel state: SSR seed, live
                                     upsert/remove, optimistic complete
  lib/stores/chat.svelte.ts          AgentChatState: ConversationStore →
                                     $state snapshot (one instance per agent)
  lib/stores/planner.svelte.ts       Planner tab: coordinator chat instance +
                                     plan-card grace / execution-watch reloads
  lib/stores/view.svelte.ts          Board | Planner tab state
  lib/components/                    LoginScreen, Header, ShiftBoard,
                                     ShiftCard, TaskPanel, ChatPanel,
                                     PlanPanel, Toasts
  ssr-check.sh (frontend/)           curl-only proof of real SSR
  inbox-task-check.sh (frontend/)    curl-only proof of the human-task loop
```

All state modules are Svelte 5 runes (`$state`/`$derived`/`$effect`) wrapping
the framework-agnostic SDK stores — the same pattern works in React via the
SDK's `useConversation` hook.
