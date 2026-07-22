/**
 * `@raisindb/mcp-ui-client` — the browser-side runtime helper for RaisinDB
 * MCP-UI widgets.
 *
 * This package runs **inside the widget iframe**, not inside a RaisinDB-connected
 * backend, so it is deliberately tiny, dependency-free, and framework-agnostic.
 * Its job is to hide the differences between the two host bridge conventions a
 * RaisinDB widget can end up running under, and between the two delivery modes a
 * widget can be served with:
 *
 * - **Delivery modes** (how the widget's HTML reached the iframe):
 *   - `mode: "html"` — the host rendered raw HTML via `srcdoc`. RaisinDB injects
 *     `window.__RAISIN_INITIAL_ROUTE__` (and, when present,
 *     `window.__RAISIN_INITIAL_DATA__`) before the document so the widget can
 *     recover its route/data without a navigable URL.
 *   - `mode: "uri-list"` — the host iframed a real URL. The route rides on
 *     `location.hash`; there is no injected global.
 *   `getInitialRoute()` / `getInitialData()` normalize across both so widget
 *   code never has to know which mode it was served under.
 *
 * - **Host bridges** (how a widget triggers a tool call and receives results):
 *   - **MCP Apps / ext-apps** — the official extension. Exposes
 *     `window.callServerTool`, `window.ontoolresult`, and
 *     `window.updateModelContext`.
 *   - **MCP-UI** — the community convention. Fire-and-forget
 *     `postMessage({ type: 'tool', payload: { toolName, params } }, '*')`, with
 *     results delivered back as `message` events.
 *   `callTool()` / `onToolResult()` prefer ext-apps when present and fall back
 *   to MCP-UI, so a widget author writes one call site regardless of host.
 *
 * @packageDocumentation
 */

/**
 * The ext-apps globals a host may inject onto `window`.
 *
 * All are optional — their presence is exactly how {@link callTool},
 * {@link onToolResult}, and {@link updateModelContext} detect an ext-apps host.
 */
interface ExtAppsGlobals {
  /**
   * Invoke a server tool through the host. Mirrors the MCP Apps
   * `app.callServerTool({ name, arguments })` convention and returns the tool
   * result.
   */
  callServerTool?: (req: {
    name: string;
    arguments: Record<string, unknown>;
  }) => Promise<unknown> | unknown;
  /**
   * Host-assigned handler invoked with each tool result. This package installs
   * a single dispatcher here and fans out to every {@link onToolResult}
   * listener, preserving any handler that was already set.
   */
  ontoolresult?: ((result: unknown) => void) | null;
  /**
   * Push structured content back into the conversation for the model to see.
   */
  updateModelContext?: (content: unknown) => Promise<unknown> | unknown;
}

/** The RaisinDB-injected globals for `mode: "html"` delivery. */
interface RaisinInjectedGlobals {
  /** Initial route injected by the engine for `mode: "html"` widgets. */
  __RAISIN_INITIAL_ROUTE__?: string;
  /**
   * Initial tool `structuredContent` injected by the engine for `mode: "html"`
   * widgets, when the host cannot deliver it any other way.
   */
  __RAISIN_INITIAL_DATA__?: unknown;
}

/** The subset of `window` this helper reads/writes. */
type WidgetWindow = Window & ExtAppsGlobals & RaisinInjectedGlobals;

/** Message shape posted to an MCP-UI host to trigger a tool call. */
interface McpUiToolMessage {
  type: 'tool';
  payload: { toolName: string; params: Record<string, unknown> };
  /** Correlation id echoed back on the result, best-effort. */
  messageId: string;
}

/**
 * Safely resolve the widget `window`, or `undefined` in a non-browser context
 * (e.g. SSR / tests) so importing this module never throws.
 */
function widgetWindow(): WidgetWindow | undefined {
  return typeof window === 'undefined'
    ? undefined
    : (window as unknown as WidgetWindow);
}

/**
 * Return the widget's initial route, normalized across both delivery modes.
 *
 * Resolution order:
 * 1. `window.__RAISIN_INITIAL_ROUTE__` — injected for `mode: "html"`.
 * 2. `location.hash` — the fragment carried on the iframe `src` for
 *    `mode: "uri-list"`.
 *
 * A leading `#` from `location.hash` is stripped so both modes yield the same
 * form (e.g. `"/order-card"`). Returns `""` when no route is available.
 *
 * @returns The initial route string (never `undefined`).
 *
 * @example
 * ```ts
 * import { getInitialRoute } from '@raisindb/mcp-ui-client';
 * const route = getInitialRoute(); // "/order-card" in both delivery modes
 * ```
 */
export function getInitialRoute(): string {
  const win = widgetWindow();
  if (!win) return '';
  const injected = win.__RAISIN_INITIAL_ROUTE__;
  if (typeof injected === 'string' && injected.length > 0) {
    return injected;
  }
  const hash = win.location?.hash ?? '';
  return hash.startsWith('#') ? hash.slice(1) : hash;
}

/**
 * Return the tool's initial `structuredContent` as delivered by the host on
 * load, or `undefined` when none was provided.
 *
 * Because hosts differ in how they hand a widget its initial data, this reads a
 * documented priority list of known delivery points:
 * 1. `window.__RAISIN_INITIAL_DATA__` — RaisinDB's own `mode: "html"` injection.
 * 2. `window.callServerTool`-host tool output, when the ext-apps host exposed
 *    the initial result as `window.toolOutput` / `window.structuredContent`.
 *
 * @typeParam T - Expected shape of the structured content.
 * @returns The initial data, or `undefined` if the host delivered none.
 *
 * @example
 * ```ts
 * import { getInitialData } from '@raisindb/mcp-ui-client';
 * type Order = { id: string; total: number };
 * const order = getInitialData<Order>();
 * ```
 */
export function getInitialData<T = unknown>(): T | undefined {
  const win = widgetWindow();
  if (!win) return undefined;
  if (win.__RAISIN_INITIAL_DATA__ !== undefined) {
    return win.__RAISIN_INITIAL_DATA__ as T;
  }
  // Best-effort ext-apps fallbacks: some hosts expose the initiating tool's
  // output on a well-known global rather than injecting our own.
  const anyWin = win as unknown as Record<string, unknown>;
  const structured = anyWin['structuredContent'] ?? anyWin['toolOutput'];
  return structured === undefined ? undefined : (structured as T);
}

/**
 * Invoke a server tool from inside the widget.
 *
 * Prefers the ext-apps bridge (`window.callServerTool`) when present, awaiting
 * and returning its result. Otherwise falls back to the MCP-UI convention,
 * posting `{ type: 'tool', payload: { toolName, params } }` to the host and
 * resolving `undefined` immediately — under MCP-UI the actual result arrives
 * asynchronously via {@link onToolResult}, since it is a fire-and-forget bridge.
 *
 * Hosts may require explicit user approval before executing a UI-initiated tool
 * call; that is host policy and outside this helper's control.
 *
 * @param name - Tool name to invoke.
 * @param args - Arguments object for the tool (defaults to `{}`).
 * @returns The ext-apps tool result, or `undefined` under the MCP-UI fallback.
 *
 * @example
 * ```ts
 * import { callTool, onToolResult } from '@raisindb/mcp-ui-client';
 * onToolResult((result) => render(result));   // needed for the MCP-UI path
 * await callTool('approve_order', { orderId: '42' });
 * ```
 */
export async function callTool(
  name: string,
  args: Record<string, unknown> = {},
): Promise<unknown> {
  const win = widgetWindow();
  if (!win) return undefined;

  if (typeof win.callServerTool === 'function') {
    return await win.callServerTool({ name, arguments: args });
  }

  // MCP-UI fallback: fire-and-forget postMessage to the host frame.
  const message: McpUiToolMessage = {
    type: 'tool',
    payload: { toolName: name, params: args },
    messageId: `raisin-${Date.now()}-${Math.random().toString(36).slice(2)}`,
  };
  win.parent?.postMessage(message, '*');
  return undefined;
}

/**
 * Register a callback invoked with each tool result the host delivers.
 *
 * Wires up both bridge conventions and returns an unsubscribe function:
 * - **ext-apps** — installs a single dispatcher on `window.ontoolresult` (once)
 *   that fans out to every registered listener, preserving any handler that was
 *   already assigned by the host or other code.
 * - **MCP-UI** — listens for `message` events whose data looks like a tool
 *   result (`type` of `tool-result` / `toolResult`, or a `result` field beside a
 *   `toolName`), passing the result payload to the callback.
 *
 * @param callback - Invoked with each tool result payload.
 * @returns An unsubscribe function that removes this listener.
 *
 * @example
 * ```ts
 * import { onToolResult } from '@raisindb/mcp-ui-client';
 * const off = onToolResult((result) => console.log('tool said', result));
 * // later: off();
 * ```
 */
export function onToolResult(callback: (result: unknown) => void): () => void {
  const win = widgetWindow();
  if (!win) return () => {};

  // ext-apps: maintain a shared listener set behind a single `ontoolresult`.
  const listeners = ensureToolResultListeners(win);
  listeners.add(callback);

  // MCP-UI: also listen for postMessage-delivered results.
  const messageHandler = (event: MessageEvent) => {
    const data = event.data as
      | { type?: string; toolName?: string; result?: unknown; payload?: unknown }
      | null
      | undefined;
    if (!data || typeof data !== 'object') return;
    const isToolResult =
      data.type === 'tool-result' ||
      data.type === 'toolResult' ||
      (data.result !== undefined && data.toolName !== undefined);
    if (isToolResult) {
      callback(data.result ?? data.payload ?? data);
    }
  };
  win.addEventListener('message', messageHandler);

  return () => {
    listeners.delete(callback);
    win.removeEventListener('message', messageHandler);
  };
}

/** Per-window registry backing the shared `ontoolresult` dispatcher. */
const TOOL_RESULT_LISTENERS = new WeakMap<
  WidgetWindow,
  Set<(result: unknown) => void>
>();

/**
 * Install (once per window) a single `window.ontoolresult` dispatcher that fans
 * out to a shared listener set, chaining any pre-existing handler.
 */
function ensureToolResultListeners(
  win: WidgetWindow,
): Set<(result: unknown) => void> {
  let listeners = TOOL_RESULT_LISTENERS.get(win);
  if (listeners) return listeners;

  listeners = new Set();
  TOOL_RESULT_LISTENERS.set(win, listeners);

  const previous = typeof win.ontoolresult === 'function' ? win.ontoolresult : null;
  win.ontoolresult = (result: unknown) => {
    previous?.(result);
    for (const listener of listeners!) {
      listener(result);
    }
  };
  return listeners;
}

/**
 * Push structured content back into the conversation for the model to see.
 *
 * Passthrough to the ext-apps `window.updateModelContext` when the host exposes
 * it. On a host without ext-apps support this is a documented no-op (the MCP-UI
 * convention has no equivalent), returning `false` so callers can detect it.
 *
 * @param content - Structured content to surface to the model.
 * @returns `true` if the update was forwarded to a host, `false` on no-op.
 *
 * @example
 * ```ts
 * import { updateModelContext } from '@raisindb/mcp-ui-client';
 * await updateModelContext({ content: [{ type: 'text', text: 'Order approved' }] });
 * ```
 */
export async function updateModelContext(content: unknown): Promise<boolean> {
  const win = widgetWindow();
  if (!win || typeof win.updateModelContext !== 'function') {
    return false;
  }
  await win.updateModelContext(content);
  return true;
}
