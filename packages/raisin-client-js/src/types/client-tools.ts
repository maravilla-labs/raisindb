/**
 * Client tools: browser-side tools a conversation's agent may call.
 *
 * The page (e.g. a WebMCP host) registers tools; the conversation store sends
 * their descriptions with every user message, answers `client_tool_call`
 * events addressed to it, and posts the results back to the agent run.
 */

/** One tool a page offers to the agent. */
export interface ClientTool {
  name: string;
  description?: string;
  inputSchema?: object;
  annotations?: Record<string, boolean>;
}

/** What a client tool returns to the agent. */
export interface ClientToolResult {
  ok: boolean;
  content?: unknown;
  error?: string;
  /** The content is author-written data: frame it as data, never instructions. */
  untrusted?: boolean;
}

/** Supplies a page's tools and executes them. */
export interface ClientToolsProvider {
  getTools(): Promise<ClientTool[]>;
  getContext?(): Promise<unknown>;
  execute(name: string, args: Record<string, unknown>): Promise<ClientToolResult>;
}

/** Tools + context attached to a user message (`properties.metadata.client`). */
export interface ClientToolsMessageMeta {
  tools: ClientTool[];
  context?: unknown;
}

/** A reference to an uploaded file attached to a user message. */
export interface Attachment {
  'raisin:ref': string;
  'raisin:workspace': string;
  mime_type?: string;
  name?: string;
  /**
   * The picture itself, for a vision-capable agent: base64 (no `data:` prefix),
   * already shrunk by the caller (long side <= 1568 px, about 1.5 MB at most,
   * at most 4 per message). The reference above stays either way. The engine
   * forwards it for the latest message only and removes it when the run ends.
   */
  /** One picture, or one per PDF page (each sent as its own image). */
  inline?:
    | { media_type: 'image/jpeg' | 'image/png' | 'image/webp' | 'image/gif'; data: string }
    | Array<{ media_type: 'image/jpeg' | 'image/png' | 'image/webp' | 'image/gif'; data: string }>;
}

/** Conversation event: the agent asks the sending page to run a tool. */
export interface ClientToolCallEvent {
  type: 'client_tool_call';
  run_id: string;
  call_id: string;
  resume_key: string;
  name: string;
  args: Record<string, unknown>;
  /** The `client_id` of the message that started this turn. */
  client_id: string;
  expires_at_ms: number;
  timestamp?: string;
}

/** Body of `POST /api/agent-runs/{repo}/{run}/client-result`. */
export interface ClientToolResultBody {
  resume_key: string;
  /** Idempotency key. */
  delivery_id: string;
  result: ClientToolResult;
  /** The page's CURRENT tools; replaces the run's set. */
  client_tools?: ClientTool[];
  client_context?: unknown;
}

export interface ClientToolResultAck {
  status: 'applied' | 'duplicate' | 'rejected';
  reason?: 'request_not_open' | 'not_a_client_call' | 'forbidden' | 'expired';
}

/** Options for sending a message with client tools / attachments. */
export interface ClientMessageOptions {
  /** Tools + context to attach (`metadata.client`). */
  client?: ClientToolsMessageMeta;
  /** Files attached to the message (`metadata.attachments`). */
  attachments?: Attachment[];
  /** Use this as the message's `client_id` (default: a fresh UUID). */
  clientId?: string;
}

/**
 * Build the `properties.metadata` object for a user message (contract §2).
 * `metadata.client.client_id` equals the message's `client_id`.
 */
export function buildMessageMetadata(
  clientId: string,
  options?: ClientMessageOptions,
): Record<string, unknown> | undefined {
  const metadata: Record<string, unknown> = {};
  if (options?.client) {
    const client: Record<string, unknown> = { client_id: clientId, tools: options.client.tools };
    if (options.client.context !== undefined) client.context = options.client.context;
    metadata.client = client;
  }
  if (options?.attachments && options.attachments.length > 0) {
    // An attachment is a reference envelope ({raisin:ref, raisin:workspace}), and the server
    // normalizes a stored reference to exactly those keys: anything else on it (inline image
    // bytes, the file name, the type) would be dropped on write. So the inline bytes travel in
    // their own list, keyed by the attachment's path, and the envelope keeps only the reference.
    const images: Array<{ path: string; name?: string; media_type: string; data: string }> = [];
    metadata.attachments = options.attachments.map((a) => {
      type Bytes = { media_type: string; data: string };
      const { inline, ...rest } = a as typeof a & { inline?: Bytes | Bytes[] };
      // One picture, or several (a PDF's pages, each its own image named with its page).
      const list = (Array.isArray(inline) ? inline : inline ? [inline] : []).filter(
        (b): b is Bytes => !!b && typeof b.data === 'string' && b.data.length > 0,
      );
      list.forEach((b, k) => {
        const name = list.length > 1 && rest.name ? `${rest.name} (page ${k + 1})` : rest.name;
        images.push({ path: String(rest['raisin:ref'] ?? ''), name, media_type: b.media_type, data: b.data });
      });
      return rest;
    });
    if (images.length > 0) metadata.attachment_images = images;
  }
  return Object.keys(metadata).length > 0 ? metadata : undefined;
}
