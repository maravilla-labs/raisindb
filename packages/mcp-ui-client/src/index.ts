/**
 * `@raisindb/mcp-ui-client` — the browser-side runtime helper for RaisinDB
 * MCP-UI widgets, speaking MCP Apps (SEP-1865).
 *
 * This package runs **inside the widget iframe** (the spec's "View"), not in a
 * RaisinDB-connected backend, so it is deliberately tiny, dependency-free, and
 * framework-agnostic. The protocol is JSON-RPC 2.0 over `postMessage` with the
 * embedding host:
 *
 * - On load the helper sends `ui/initialize`, stores the returned
 *   `hostContext` (theme, style variables, dimensions), sends
 *   `ui/notifications/initialized`, and starts reporting content size via
 *   `ui/notifications/size-changed` (ResizeObserver, debounced).
 * - Host notifications are routed to listeners: `ui/notifications/tool-input`,
 *   `ui/notifications/tool-result`, `ui/notifications/host-context-changed`.
 * - Host requests are answered: `ui/resource-teardown`, `ping`, `tools/list`
 *   (no app-registered tools yet).
 * - Widget-initiated calls are plain requests: `tools/call`,
 *   `ui/update-model-context`, `ui/open-link`, `ui/message`.
 *
 * @packageDocumentation
 */

/** Host context delivered by the host in `McpUiInitializeResult` and updated
 * via `ui/notifications/host-context-changed`. */
export interface HostContext {
  /** Metadata of the tool call that instantiated the view. */
  toolInfo?: { id?: unknown; tool?: { name?: string; [key: string]: unknown } };
  theme?: 'light' | 'dark';
  styles?: {
    variables?: Record<string, string | undefined>;
    css?: { fonts?: string };
  };
  displayMode?: string;
  availableDisplayModes?: string[];
  containerDimensions?: Record<string, number>;
  locale?: string;
  timeZone?: string;
  userAgent?: string;
  platform?: string;
  [key: string]: unknown;
}

/** Standard MCP `CallToolResult` as delivered to the view. */
export interface ToolResult {
  content?: Array<Record<string, unknown>>;
  structuredContent?: unknown;
  isError?: boolean;
  [key: string]: unknown;
}

const PROTOCOL_VERSION = '2025-06-18';

function widgetWindow(): Window | undefined {
  return typeof window === 'undefined' ? undefined : window;
}

let rpcId = 1;
const pending = new Map<
  number,
  { resolve: (v: unknown) => void; reject: (e: Error) => void }
>();
const toolResultListeners = new Set<(result: ToolResult) => void>();
const toolInputListeners = new Set<(args: Record<string, unknown>) => void>();
const hostContextListeners = new Set<(ctx: HostContext) => void>();

let hostContext: HostContext | undefined;
let lastToolInput: Record<string, unknown> | undefined;
let initialized: Promise<void> | undefined;
let listenerInstalled = false;

/** Live bridge diagnostics (for widget debug footers). */
export interface BridgeDebug {
  handshake: 'pending' | 'ok';
  /** message events seen from the parent frame */
  received: number;
  /** message events seen from OTHER sources (dropped) */
  foreign: number;
  /** last few jsonrpc methods received */
  methods: string[];
}
const bridgeDebug: BridgeDebug = { handshake: 'pending', received: 0, foreign: 0, methods: [] };
const debugListeners = new Set<(d: BridgeDebug) => void>();
function touchDebug(method?: string) {
  if (method) {
    bridgeDebug.methods.push(method);
    if (bridgeDebug.methods.length > 5) bridgeDebug.methods.shift();
  }
  for (const listener of debugListeners) listener(bridgeDebug);
}

/** Current bridge diagnostics snapshot. */
export function getBridgeDebug(): BridgeDebug {
  return bridgeDebug;
}

/** Subscribe to bridge diagnostics changes. Returns unsubscribe. */
export function onBridgeDebug(callback: (d: BridgeDebug) => void): () => void {
  debugListeners.add(callback);
  callback(bridgeDebug);
  return () => debugListeners.delete(callback);
}

function post(message: Record<string, unknown>) {
  widgetWindow()?.parent?.postMessage(message, '*');
}

function request(method: string, params: unknown): Promise<unknown> {
  const id = rpcId++;
  const promise = new Promise<unknown>((resolve, reject) => {
    pending.set(id, { resolve, reject });
  });
  post({ jsonrpc: '2.0', id, method, params });
  return promise;
}

function notify(method: string, params: unknown) {
  post({ jsonrpc: '2.0', method, params });
}

function handleMessage(data: Record<string, unknown>) {
  if (data.jsonrpc !== '2.0') return;

  // Response to one of our requests.
  if (data.id !== undefined && (data.result !== undefined || data.error !== undefined)) {
    const entry = pending.get(data.id as number);
    if (!entry) return;
    pending.delete(data.id as number);
    if (data.error !== undefined) {
      const err = data.error as { message?: string } | undefined;
      entry.reject(new Error(err?.message ?? 'MCP host error'));
    } else {
      entry.resolve(data.result);
    }
    return;
  }

  const method = data.method as string | undefined;
  if (!method) return;
  touchDebug(method);
  const params = (data.params ?? {}) as Record<string, unknown>;

  // Requests FROM the host (carry an id, expect a response).
  if (data.id !== undefined) {
    switch (method) {
      case 'ui/resource-teardown':
      case 'ping':
        post({ jsonrpc: '2.0', id: data.id, result: {} });
        return;
      case 'tools/list':
        post({ jsonrpc: '2.0', id: data.id, result: { tools: [] } });
        return;
      default:
        post({
          jsonrpc: '2.0',
          id: data.id,
          error: { code: -32601, message: `Method not found: ${method}` },
        });
        return;
    }
  }

  // Notifications from the host.
  switch (method) {
    case 'ui/notifications/tool-result':
      for (const listener of toolResultListeners) listener(params as ToolResult);
      return;
    case 'ui/notifications/tool-input':
    case 'ui/notifications/tool-input-partial': {
      const args = (params.arguments ?? {}) as Record<string, unknown>;
      if (method === 'ui/notifications/tool-input') lastToolInput = args;
      for (const listener of toolInputListeners) listener(args);
      return;
    }
    case 'ui/notifications/host-context-changed':
      hostContext = { ...(hostContext ?? {}), ...(params as HostContext) };
      applyTheme(hostContext);
      for (const listener of hostContextListeners) listener(hostContext);
      return;
    default:
      return;
  }
}

/** Best-effort theme application: color-scheme + host CSS variables on :root. */
function applyTheme(ctx: HostContext | undefined) {
  const win = widgetWindow();
  const root = win?.document?.documentElement;
  if (!root || !ctx) return;
  if (ctx.theme) {
    root.style.colorScheme = ctx.theme;
    root.dataset.theme = ctx.theme;
  }
  const variables = ctx.styles?.variables;
  if (variables) {
    for (const [name, value] of Object.entries(variables)) {
      if (typeof value === 'string' && name.startsWith('--')) {
        root.style.setProperty(name, value);
      }
    }
  }
}

/** Debounced content-size reporting via `ui/notifications/size-changed`. */
function startAutoResize(win: Window) {
  const body = win.document?.body;
  if (!body || typeof ResizeObserver === 'undefined') return;
  let last = { width: 0, height: 0 };
  let timer: ReturnType<typeof setTimeout> | undefined;
  const observer = new ResizeObserver(() => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      const width = Math.ceil(body.scrollWidth);
      const height = Math.ceil(body.scrollHeight);
      if (width === last.width && height === last.height) return;
      last = { width, height };
      notify('ui/notifications/size-changed', { width, height });
    }, 100);
  });
  observer.observe(body);
}

/**
 * Start (once) the MCP Apps handshake with the embedding host. Resolves when
 * the host answered `ui/initialize`. Called implicitly by every API below and
 * eagerly on module load.
 */
export function connect(): Promise<void> {
  if (initialized) return initialized;
  const win = widgetWindow();
  if (!win || !win.parent || win.parent === (win as unknown)) {
    initialized = Promise.resolve();
    return initialized;
  }
  if (!listenerInstalled) {
    listenerInstalled = true;
    win.addEventListener('message', (event: MessageEvent) => {
      // Only the embedding host frame may drive the view.
      if (event.source !== win.parent) {
        bridgeDebug.foreign++;
        touchDebug();
        return;
      }
      const data = event.data as Record<string, unknown> | null | undefined;
      if (!data || typeof data !== 'object') return;
      bridgeDebug.received++;
      touchDebug();
      handleMessage(data);
    });
  }
  // The host attaches its bridge listener asynchronously after creating the
  // iframe — an `ui/initialize` fired at script-parse time can be lost. Retry
  // (fresh request id each time) until the host answers; the host only starts
  // sending tool-input/tool-result AFTER our `initialized` notification, so a
  // lost handshake would otherwise wedge the view at its waiting state.
  initialized = new Promise<void>((resolve) => {
    let settled = false;
    const attempt = () => {
      if (settled) return;
      request('ui/initialize', {
        protocolVersion: PROTOCOL_VERSION,
        appInfo: { name: 'raisindb-widget', version: '0.2.0' },
        appCapabilities: { availableDisplayModes: ['inline'] },
      }).then((result) => {
        if (settled) return;
        settled = true;
        bridgeDebug.handshake = 'ok';
        const init = result as { hostContext?: HostContext } | undefined;
        hostContext = init?.hostContext;
        applyTheme(hostContext);
        notify('ui/notifications/initialized', {});
        startAutoResize(win);
        touchDebug();
        if (hostContext) {
          for (const listener of hostContextListeners) listener(hostContext);
        }
        resolve();
      });
    };
    attempt();
    let tries = 0;
    const timer = setInterval(() => {
      if (settled || tries++ > 20) {
        clearInterval(timer);
        return;
      }
      attempt();
    }, 400);
  });
  return initialized;
}

// Connect eagerly so notifications sent right after load are not missed.
if (typeof window !== 'undefined') {
  try {
    connect();
  } catch {
    // Never let bridge setup break the widget.
  }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/** The view's in-app route from `location.hash` (`""` when none). */
export function getInitialRoute(): string {
  const hash = widgetWindow()?.location?.hash ?? '';
  return hash.startsWith('#') ? hash.slice(1) : hash;
}

/** The complete tool-call arguments, once `ui/notifications/tool-input` arrived. */
export function getToolInput(): Record<string, unknown> | undefined {
  return lastToolInput;
}

/** Name of the tool call that instantiated this view, when the host said. */
export function getInitiatingToolName(): string | undefined {
  const name = hostContext?.toolInfo?.tool?.name;
  return typeof name === 'string' && name ? name : undefined;
}

/** Register a callback for tool-input notifications. Returns unsubscribe. */
export function onToolInput(
  callback: (args: Record<string, unknown>) => void,
): () => void {
  connect();
  toolInputListeners.add(callback);
  return () => toolInputListeners.delete(callback);
}

/** The current host context (theme, styles, display mode). */
export function getHostContext(): HostContext | undefined {
  return hostContext;
}

/** Register a callback for host-context arrival/changes. Returns unsubscribe. */
export function onHostContext(callback: (ctx: HostContext) => void): () => void {
  connect();
  hostContextListeners.add(callback);
  if (hostContext) callback(hostContext);
  return () => hostContextListeners.delete(callback);
}

/**
 * Register a callback invoked with each `CallToolResult` the host delivers —
 * both the initiating tool's result (`ui/notifications/tool-result`) and the
 * results of view-initiated {@link callTool} calls. Returns unsubscribe.
 */
export function onToolResult(callback: (result: ToolResult) => void): () => void {
  connect();
  toolResultListeners.add(callback);
  return () => toolResultListeners.delete(callback);
}

/**
 * Invoke a server tool from the view: a plain `tools/call` JSON-RPC request
 * through the host. Returns the `CallToolResult` and ALSO fans it out to
 * {@link onToolResult} listeners, so single-code-path views work either way.
 * Hosts may prompt the user before executing view-initiated calls.
 */
export async function callTool(
  name: string,
  args: Record<string, unknown> = {},
): Promise<ToolResult> {
  await connect();
  const result = (await request('tools/call', { name, arguments: args })) as ToolResult;
  for (const listener of toolResultListeners) listener(result);
  return result;
}

/**
 * Push content back into the conversation for the model's future turns
 * (`ui/update-model-context`). Each call overwrites the previous update.
 */
export async function updateModelContext(content: unknown): Promise<void> {
  await connect();
  await request('ui/update-model-context', { content });
}

/** Ask the host to open an external URL (`ui/open-link`). */
export async function openLink(url: string): Promise<void> {
  await connect();
  await request('ui/open-link', { url });
}

/** Send a user-role text message into the host's chat (`ui/message`). */
export async function sendMessage(text: string): Promise<void> {
  await connect();
  await request('ui/message', { role: 'user', content: { type: 'text', text } });
}
