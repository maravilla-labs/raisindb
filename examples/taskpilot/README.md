# TaskPilot

A focused demo for the **RaisinDB plan/task system**, and the first real
application on the JS SDK's **React integration**
(`createRaisinReact(React)`), built with **React Router 7** (framework
mode, SPA).

A Groq-backed planning agent (`/agents/pilot`) manages a small launch
checklist. Ask it to *"Plan and execute: …"* and it proposes a plan
(`raisin:AIPlan` + ordered `raisin:AITask` nodes); the plan panel shows the
proposal with **Approve / Reject** buttons, the tasks execute live against
the checklist after approval, and the checklist updates in realtime.

## What it demonstrates

- **Plan/task system, end to end in a UI** — `task_creation_enabled` agent,
  `execution_mode` gates (`approve_then_auto` by default), plan proposal
  card, approval/rejection from the browser, live task status progression,
  step-by-step pausing with a Continue button.
- **The SDK React integration** — `RaisinProvider`, `useAuth` (login +
  session restore), `useConnection` (status dot), `useSql` (checklist +
  agent-mode chip, with a realtime refetch subscription), `useConversation`
  (streaming chat, tool badges, plan projection, approve/reject), and
  `useConversationList` (resume the latest conversation).
- **Custom tools + a package** — three JS functions (`list-items`,
  `update-item`, `radio-check`) deployed from a CLI-installable package.

## Setup

1. **Server** — a running raisin-server with the Groq provider configured
   for the tenant (admin console → AI settings):

   ```bash
   RUST_LOG=info ./target/release/raisin-server --config examples/cluster/node1.toml
   ```

2. **Content** — either the package flow via the CLI:

   ```bash
   raisindb login --server http://localhost:8081 --username admin --password ...
   raisindb package deploy ./package --repo taskpilot --install
   ```

   or the no-CLI fallback (creates the repo, workspace, items, tools, agent
   and the demo user in one go):

   ```bash
   npm install
   npm run setup                      # default mode: approve_then_auto
   node setup.mjs --mode step_by_step # switch the agent's execution_mode
   node setup.mjs --reset-items       # reset all items back to todo
   ```

3. **App**:

   ```bash
   npm run dev          # http://localhost:5177
   ```

   Sign in as `pilot@example.com` / `Pilot12345!` and send:

   > Plan and execute: mark "Design hero section" as done, then summarize
   > the checklist.

   HTTP (login, conversations SSE, SQL) goes same-origin through the Vite
   dev proxy (`vite.config.ts`), so the server needs **no CORS entry**; the
   WebSocket connects directly (`ws://localhost:8081/ws/taskpilot`).

4. **Headless proof** (costs one real Groq run, ~5 LLM calls):

   ```bash
   npm run check
   ```

   Login → ask for a 2-task plan → proposal card appears
   (`pending_approval`) → Approve → both tasks complete → the checklist
   item actually changed (server + live UI) → final reply summarizes.

## The SDK contract (plan/task recap)

- The agent needs `task_creation_enabled: true` plus the builtin planning
  tools (`/lib/raisin/ai/create-plan`, `add-task`, `update-task`,
  `get-plan-status`) in its `tools` list.
- `execution_mode` on the agent node controls the lifecycle:
  - `automatic` — plan executes immediately, no approval.
  - `approve_then_auto` — plan waits in `pending_approval`; after
    `approvePlan()` all tasks run to completion.
  - `step_by_step` — after approval exactly one task runs, then the turn
    ends with `finish_reason: awaiting_step_continue`; a normal `continue`
    message resumes (the plan panel's **Continue** button).
  - `manual` — the plan is created and nothing executes.
- `ConversationStore` (what `useConversation` wraps) exposes:
  - `plans` — a deterministic projection (`PlanProjection[]`) built from
    the persisted `ai_plan` / `ai_task_update` messages: title, status,
    `requiresApproval`, ordered tasks with statuses.
  - `approvePlan(planPath)` / `rejectPlan(planPath, feedback?)` — enqueue
    the decision; execution progress arrives via the persistent
    conversation subscription.
  - A turn that pauses for approval ends with
    `finish_reason: awaiting_plan_approval`.
- Transcript hygiene: messages with `messageType` `ai_plan` /
  `ai_task_update` are plan-panel data, not chat bubbles — filter them out
  of the message list (see `ChatPanel.tsx`).
- Realtime node subscriptions: a plain `path` filter matches only that
  exact node — use `'/checklist/**'` for descendants (see
  `ChecklistPanel.tsx`).

## Layout

```
app/                    React Router 7 app (SPA mode, port 5177)
  lib/raisin.ts         RaisinClient singleton + createRaisinReact hooks
  components/           LoginCard, Workspace, ChecklistPanel,
                        ChatWorkspace, ChatPanel, PlanPanel
package/                CLI-installable package (workspace, items, tools,
                        agent + ai-workspace home)
setup.mjs               No-CLI setup fallback (idempotent), --mode flag
check.mjs               Headless Playwright proof (one Groq run)
```
