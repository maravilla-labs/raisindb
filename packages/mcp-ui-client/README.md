# @raisindb/mcp-ui-client

Tiny, dependency-free, **browser-only** runtime helper for RaisinDB MCP-UI
widgets. It runs *inside the widget iframe* and papers over the two host bridge
conventions (MCP Apps / ext-apps and MCP-UI) and the two delivery modes
(`mode: "html"` and `mode: "uri-list"`) a RaisinDB widget can be served under,
so widget authors write one call site regardless of host.

> This is **not** the RaisinDB backend SDK. To talk to an MCP server from a
> Node/TS backend, use `McpClient` in [`@raisindb/client`](../raisin-client-js).
> This package has no RaisinDB connection, no auth, and no network calls of its
> own — it only bridges to whatever host is rendering the widget.

## Install

```bash
npm install @raisindb/mcp-ui-client
```

Zero runtime dependencies. Ships ESM + type declarations.

## Delivery modes

A RaisinDB tool binds a widget with `ui: { mode, entry }`:

- **`mode: "html"`** — the host renders raw HTML via `srcdoc`. There is no
  navigable URL, so RaisinDB injects `window.__RAISIN_INITIAL_ROUTE__` (and,
  when available, `window.__RAISIN_INITIAL_DATA__`) before the document.
- **`mode: "uri-list"`** — the host iframes a real URL served from
  `GET /api/static/{repo}/{branch}/{ws}/{*path}`. The route rides on
  `location.hash`; no globals are injected.

`getInitialRoute()` / `getInitialData()` normalize both so your code never needs
to know which mode it ran under.

## API

All exports are framework-agnostic functions.

### `getInitialRoute(): string`

Returns the widget's initial route. Prefers `window.__RAISIN_INITIAL_ROUTE__`
(`mode: "html"`), falls back to `location.hash` (`mode: "uri-list"`) with a
leading `#` stripped so both modes yield the same form (e.g. `"/order-card"`).
Returns `""` when no route is present.

```ts
import { getInitialRoute } from '@raisindb/mcp-ui-client';

// Works unchanged in both delivery modes:
const route = getInitialRoute(); // "/order-card"
router.navigate(route);
```

### `getInitialData<T>(): T | undefined`

Returns the initiating tool's `structuredContent` as delivered by the host on
load, or `undefined`. Reads `window.__RAISIN_INITIAL_DATA__` first, then
best-effort ext-apps globals.

```ts
import { getInitialData } from '@raisindb/mcp-ui-client';

type Order = { id: string; total: number };
const order = getInitialData<Order>();
```

### `callTool(name, args?): Promise<unknown>`

Invokes a server tool. Prefers the ext-apps `window.callServerTool` bridge
(awaiting and returning its result); otherwise falls back to the MCP-UI
convention — `postMessage({ type: 'tool', payload: { toolName, params } }, '*')`
— and resolves `undefined`, since under MCP-UI the result arrives asynchronously
via `onToolResult`.

Hosts may require explicit user approval before running a UI-initiated tool
call. Keep destructive operations out of the set a widget can trigger without a
second confirmation.

### `onToolResult(cb): () => void`

Registers a callback for tool results and returns an unsubscribe function. Wires
up **both** bridges: it installs a single shared `window.ontoolresult`
dispatcher (ext-apps, preserving any pre-existing handler) and listens for
MCP-UI `message`-delivered results.

```ts
import { callTool, onToolResult } from '@raisindb/mcp-ui-client';

const off = onToolResult((result) => render(result));
await callTool('approve_order', { orderId: '42' });
// later: off();
```

### `updateModelContext(content): Promise<boolean>`

Passthrough to the ext-apps `window.updateModelContext` when the host exposes
it, pushing structured content back into the conversation for the model to see.
On a host without ext-apps support this is a documented no-op and returns
`false`.

## Full example

```ts
import {
  getInitialRoute,
  getInitialData,
  callTool,
  onToolResult,
  updateModelContext,
} from '@raisindb/mcp-ui-client';

// One bootstrap that works in mode:html AND mode:uri-list.
const route = getInitialRoute();
const data = getInitialData<{ orderId: string }>();
renderRoute(route, data);

onToolResult((result) => renderRoute(getInitialRoute(), result));

document.querySelector('#approve')?.addEventListener('click', async () => {
  await callTool('approve_order', { orderId: data?.orderId });
  await updateModelContext({
    content: [{ type: 'text', text: 'Order approved from the widget.' }],
  });
});
```

## License

BSL-1.1
