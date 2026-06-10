# Vue Board — RaisinDB Vue composables demo

The first real Vue 3 app on the RaisinDB JS SDK's Vue integration
(`@raisindb/client/vue`, `createRaisinVue`). A small Vite SPA that reuses the
[shiftboard](../shiftboard/) demo's server-side content (workspace
`staffing`, agent `/agents/shift-planner`) and exercises every composable:

| Composable           | Where                                                  |
|----------------------|--------------------------------------------------------|
| `useAuth`            | `src/App.vue` (initSession) + `src/components/LoginScreen.vue` (login) |
| `useConnection`      | `src/components/AppHeader.vue` (status dot)            |
| `useSql`             | `src/components/BoardPanel.vue` (shifts + staff)       |
| `useSubscription`    | `BoardPanel.vue` (`/shifts/*` node:updated → card flash) and `AppHeader.vue` (`${home}/inbox/**` node:created → bell counter) |
| `useConversationList`| `src/components/ChatPanel.vue` (resume latest ai_chat conversation with the agent) |
| `useConversation`    | `src/components/ChatSession.vue` (streaming chat, tool-call badges, Enter-to-send) |

## Setup

1. **Server running** — a raisin-server in dev mode, e.g.:

   ```bash
   cargo build --package raisin-server --features "storage-rocksdb,websocket,pgwire"
   ./target/debug/raisin-server --config <your-config>.toml --dev-mode
   ```

2. **Shiftboard package installed** into the repository the app talks to
   (default `shiftboard2`):

   ```bash
   cd ../shiftboard
   npm install
   RAISIN_URL=http://localhost:8081 RAISIN_REPO=shiftboard2 npm run setup
   ```

   For real agent replies the tenant needs a Groq provider key (admin
   console → AI settings). Without it the chat pipeline still works but the
   agent answers with the backend's "AI config not found" error.

3. **Run the app**:

   ```bash
   npm install
   npm run dev          # http://localhost:5176
   ```

   Sign in with the prefilled demo account `planner@example.com` /
   `Planner12345!`.

## Configuration

| Env var               | Default                               | Purpose |
|-----------------------|---------------------------------------|---------|
| `VITE_RAISIN_WS_URL`  | `ws://localhost:8081/ws/shiftboard2`  | WebSocket endpoint (tenant-less form, no `tenantId`) |
| `VITE_RAISIN_REPO`    | `shiftboard2`                         | Repository name |
| `VITE_RAISIN_HTTP_URL`| same-origin (Vite proxy)              | HTTP base for login + conversations SSE |

By default the SDK's HTTP calls go same-origin through the Vite dev proxy
(`vite.config.ts`), so the server needs no CORS entry for this app. To talk
to the server directly instead, allow the origin
(`raisindb cors add http://localhost:5176 --repo shiftboard2`) and set
`VITE_RAISIN_HTTP_URL=http://localhost:8081`.

## Headless check

`check.mjs` drives the app with Playwright (looked up in
`/tmp/shots/node_modules` by default, override with `PLAYWRIGHT_DIR`):

```bash
node check.mjs                 # starts vite itself if :5176 is free
RAISIN_URL=http://localhost:8082 VITE_RAISIN_WS_URL=ws://localhost:8082/ws/shiftboard2 node check.mjs
SKIP_TOUCH=1 node check.mjs    # skip the live-update assertion
```

It verifies: login → board renders shifts + staff → connection dot green →
a harmless live update (rewrites a shift's own title via the admin SDK,
asserts the card flash from the `node:updated` event, then confirms the
board state is byte-identical) → one cheap chat turn ("Which shifts are
open this weekend?", read-only tool) with a streamed reply → bell counter.

Never wait on `networkidle` in browser checks against this app — the
conversations SSE subscription keeps the network busy permanently.

## Build

```bash
npm run build   # vue-tsc --noEmit && vite build
```
