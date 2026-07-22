/**
 * McpClient — typed wrappers over a RaisinDB MCP server's JSON-RPC endpoint.
 *
 * A `raisin:McpServer` node is exposed over the MCP Streamable HTTP binding at
 * `POST /mcp/{repo}/{branch}/{slug}`, carrying one JSON-RPC 2.0 message per
 * request. This client mirrors the {@link FunctionsApi} shape: it is a thin,
 * transport-injected wrapper so any JS/TS backend (not just AI agents) can talk
 * to a RaisinDB MCP server programmatically, using the exact JSON-RPC method
 * names the engine implements (`tools/list`, `tools/call`, `resources/read`,
 * `resources/subscribe`).
 *
 * The transport is injected (see {@link McpTransport}) so the client stays
 * unit-testable and independent of the concrete HTTP layer — the HTTP client
 * supplies a real transport via its `.mcp(slug)` accessor.
 */

/**
 * A single content block returned by a tool call or resource read.
 *
 * The engine serializes structured results into `text` blocks for spec
 * compliance; machine-readable output is carried separately in
 * {@link McpCallToolResult.structuredContent}.
 */
export interface McpContentBlock {
  /** Block discriminator, e.g. `"text"`. */
  type: string;
  /** Present on `text` blocks. */
  text?: string;
  /** Allow forward-compatible block variants (`image`, `resource`, …). */
  [key: string]: unknown;
}

/** A tool descriptor as advertised by `tools/list`. */
export interface McpToolDescriptor {
  /** Unique tool name used as the `name` argument to {@link McpClient.callTool}. */
  name: string;
  /** Human-readable description. */
  description?: string;
  /** JSON Schema describing the tool's accepted arguments. */
  inputSchema?: unknown;
  /** JSON Schema describing the tool's structured output, when declared. */
  outputSchema?: unknown;
  /** Forward-compatible extra fields (e.g. `ui`, annotations). */
  [key: string]: unknown;
}

/** Result of a `tools/call` invocation. */
export interface McpCallToolResult {
  /** Result content blocks (typically a single `text` block). */
  content: McpContentBlock[];
  /** `true` when the tool reported a domain-level failure. */
  isError?: boolean;
  /**
   * Machine-readable result conforming to the tool's `outputSchema`, present
   * only for tools that declare one.
   */
  structuredContent?: unknown;
  /** Forward-compatible extra fields. */
  [key: string]: unknown;
}

/** One entry returned by `resources/read`. */
export interface McpResourceContents {
  /** `raisin://` URI of the resource that was read. */
  uri: string;
  /** MIME type of the content, when known. */
  mimeType?: string;
  /** Text payload (JSON-only resources return their properties here). */
  text?: string;
  /** Base64-encoded binary payload, for byte-serving resources. */
  blob?: string;
  /** Forward-compatible extra fields. */
  [key: string]: unknown;
}

/** Result of `resources/read`. */
export interface McpReadResourceResult {
  /** One content entry per URI read. */
  contents: McpResourceContents[];
}

/**
 * A `notifications/resources/updated` payload pushed over the subscription
 * stream when a watched resource changes.
 */
export interface McpResourceUpdate {
  /** URI of the resource that changed. */
  uri: string;
}

/**
 * A live subscription to resource-change notifications.
 *
 * Iterate it with `for await (const update of subscription) { … }`; call
 * {@link McpResourceSubscription.close} to tear the stream down.
 */
export interface McpResourceSubscription extends AsyncIterable<McpResourceUpdate> {
  /** Close the underlying stream and stop receiving updates. */
  close(): void;
}

/**
 * Transport abstraction the {@link McpClient} is built on.
 *
 * `rpc` performs one request/response JSON-RPC round trip and returns the
 * already-unwrapped `result` (throwing on a JSON-RPC `error`). `subscribe`
 * opens the `resources/subscribe` SSE stream and yields each raw JSON-RPC frame
 * forwarded by the server.
 */
export interface McpTransport {
  /**
   * Send one JSON-RPC request and resolve with its `result` payload.
   *
   * @param method - JSON-RPC method name (e.g. `"tools/list"`).
   * @param params - Method parameters, shape depends on `method`.
   */
  rpc(method: string, params?: unknown): Promise<unknown>;

  /**
   * Open an SSE stream for a subscribing method and yield each JSON-RPC frame
   * the server pushes (the initial ack plus subsequent notifications).
   *
   * @param method - Subscribing JSON-RPC method (`"resources/subscribe"`).
   * @param params - Method parameters (`{ uri }`).
   */
  subscribe(method: string, params: unknown): McpFrameStream;
}

/** A raw JSON-RPC frame as delivered over the subscription SSE stream. */
export interface McpJsonRpcFrame {
  jsonrpc?: string;
  id?: unknown;
  method?: string;
  params?: unknown;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

/** A closeable async stream of raw JSON-RPC frames. */
export interface McpFrameStream extends AsyncIterable<McpJsonRpcFrame> {
  /** Close the underlying stream. */
  close(): void;
}

/**
 * Typed client for a single RaisinDB MCP server, addressed by
 * `{repo}/{branch}/{slug}`.
 *
 * @example
 * ```typescript
 * const mcp = client.database('my-repo').mcp('assistant');
 * const { tools } = await mcp.listTools();
 * const result = await mcp.callTool('order_card', { orderId: '42' });
 * const doc = await mcp.readResource('raisin://content/site/home');
 *
 * const sub = mcp.subscribeResource('raisin://content/site/home');
 * for await (const update of sub) {
 *   console.log('changed:', update.uri);
 * }
 * ```
 */
export class McpClient {
  constructor(private readonly transport: McpTransport) {}

  /**
   * List the tools this MCP server advertises to the calling identity.
   *
   * Wraps the `tools/list` JSON-RPC method. The visible set is RLS-scoped to
   * the authenticated caller (or the anonymous role).
   *
   * @returns The advertised tool descriptors.
   */
  async listTools(): Promise<{ tools: McpToolDescriptor[] }> {
    const result = (await this.transport.rpc('tools/list')) as {
      tools?: McpToolDescriptor[];
    };
    return { tools: result?.tools ?? [] };
  }

  /**
   * Invoke a tool by name.
   *
   * Wraps the `tools/call` JSON-RPC method. The underlying `raisin:Function`
   * runs RLS-scoped to the calling identity. A domain-level tool failure is
   * reported via {@link McpCallToolResult.isError} (not thrown); transport and
   * protocol errors are thrown.
   *
   * @param name - Tool name from {@link listTools}.
   * @param args - Arguments object matching the tool's `inputSchema`.
   * @returns The tool's content blocks and optional structured content.
   */
  async callTool(
    name: string,
    args?: Record<string, unknown>,
  ): Promise<McpCallToolResult> {
    const result = (await this.transport.rpc('tools/call', {
      name,
      arguments: args ?? {},
    })) as McpCallToolResult;
    return result;
  }

  /**
   * Read a resource by its `raisin://` URI.
   *
   * Wraps the `resources/read` JSON-RPC method. JSON-only resources return
   * their properties as a `text` entry; byte-serving resources return base64
   * in `blob`.
   *
   * @param uri - `raisin://{workspace}/{path}` URI to read.
   * @returns One content entry per URI read.
   */
  async readResource(uri: string): Promise<McpReadResourceResult> {
    const result = (await this.transport.rpc('resources/read', {
      uri,
    })) as McpReadResourceResult;
    return { contents: result?.contents ?? [] };
  }

  /**
   * Subscribe to change notifications for a resource URI (or URI prefix).
   *
   * Wraps the `resources/subscribe` JSON-RPC method, which upgrades to an SSE
   * stream. The returned {@link McpResourceSubscription} is an async iterable of
   * {@link McpResourceUpdate}; the server's initial subscription ack is consumed
   * internally and not yielded. Call `close()` to end the stream.
   *
   * @param uri - `raisin://` URI (or prefix) to watch.
   * @returns A live, closeable subscription.
   */
  subscribeResource(uri: string): McpResourceSubscription {
    const stream = this.transport.subscribe('resources/subscribe', { uri });
    return {
      close: () => stream.close(),
      async *[Symbol.asyncIterator](): AsyncIterator<McpResourceUpdate> {
        for await (const frame of stream) {
          if (frame.method === 'notifications/resources/updated') {
            const params = (frame.params ?? {}) as Partial<McpResourceUpdate>;
            if (typeof params.uri === 'string') {
              yield { uri: params.uri };
            }
          }
        }
      },
    };
  }
}
