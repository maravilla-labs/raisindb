/**
 * The server half of the wasm function dev loop.
 *
 * Every call takes a `ServerContext` carrying its own `fetch`, so the whole
 * module is testable with a stub and no server. The routes are the ones the
 * plan fixes as the dev-loop contract:
 *
 * - `GET  /api/functions/{repo}/{name}`      — is the function deployed?
 * - `GET  /api/repository/{repo}/{branch}/head/{ws}/{path}` — the artifact node
 * - `POST /api/repository/…?override_existing=true` — multipart artifact upload
 * - `POST /api/functions/{repo}/{name}/invoke`      — run the deployed function
 * - `POST /api/files/{repo}/run`                    — run an artifact by node id,
 *   streaming `started` / `log` / `result` / `done` over SSE.
 *
 * `run` and `invoke` resolve on the `main` branch server-side
 * (`find_function_node` / `find_asset_node_by_id` both pass `DEFAULT_BRANCH`),
 * so a non-main branch is only meaningful for the upload.
 */

/** A `fetch` implementation — injected so tests need no network. */
export type FetchLike = typeof fetch;

/** Where and as whom a dev-loop call runs. */
export interface ServerContext {
  /** Base URL, already http(s). */
  baseUrl: string;
  repo: string;
  branch: string;
  /** Bearer token, or null for an anonymous server. */
  token: string | null;
  fetchImpl: FetchLike;
}

/** Headers for a JSON request. */
function jsonHeaders(ctx: ServerContext, accept = 'application/json'): Record<string, string> {
  const headers: Record<string, string> = { 'Content-Type': 'application/json', Accept: accept };
  if (ctx.token) headers.Authorization = `Bearer ${ctx.token}`;
  return headers;
}

/** Best-effort message from an error response body. */
async function errorMessage(response: Response, fallback: string): Promise<string> {
  const text = await response.text().catch(() => '');
  try {
    const body = JSON.parse(text) as { message?: string; error?: string };
    return body.message || body.error || text || fallback;
  } catch {
    return text || fallback;
  }
}

/** The artifact node as the dev loop reads it. */
export interface ArtifactNode {
  id: string;
  /** Hex sha256 the server recorded, when it recorded one. */
  contentHash: string | null;
  size: number | null;
}

/**
 * Read the artifact's `content_hash`.
 *
 * Mirrors the admin console's `wasmArtifactMeta.ts`: the package installer
 * writes it as a flat property, an upload path may nest it in the `file`
 * Resource's metadata, and a plain multipart upload writes neither.
 * It is TENANT-WRITABLE, so it is a cache key for "should I re-upload?" and
 * never a proof of what the bytes are — the server hashes them itself.
 */
function readContentHash(properties: Record<string, unknown>): string | null {
  const file = properties.file;
  if (file && typeof file === 'object') {
    const metadata = (file as { metadata?: Record<string, unknown> }).metadata;
    const nested = metadata?.content_hash;
    if (typeof nested === 'string') return nested;
  }
  const flat = properties.content_hash;
  return typeof flat === 'string' ? flat : null;
}

/** Artifact size, from the `file` Resource or the flat property. */
function readSize(properties: Record<string, unknown>): number | null {
  const file = properties.file;
  if (file && typeof file === 'object') {
    const size = (file as { size?: unknown }).size;
    if (typeof size === 'number') return size;
  }
  return typeof properties.file_size === 'number' ? (properties.file_size as number) : null;
}

/** Is the `raisin:Function` deployed? Null when it is not. */
export async function getFunctionDetails(
  ctx: ServerContext,
  name: string
): Promise<Record<string, unknown> | null> {
  const url = `${ctx.baseUrl}/api/functions/${ctx.repo}/${encodeURIComponent(name)}`;
  const response = await ctx.fetchImpl(url, { method: 'GET', headers: jsonHeaders(ctx) });
  if (response.status === 404) return null;
  if (!response.ok) {
    throw new Error(await errorMessage(response, `GET ${url} failed: ${response.status}`));
  }
  return (await response.json()) as Record<string, unknown>;
}

/** The uploaded artifact node, or null when the server has none at that path. */
export async function getArtifactNode(
  ctx: ServerContext,
  workspace: string,
  nodePath: string
): Promise<ArtifactNode | null> {
  const url = `${ctx.baseUrl}/api/repository/${ctx.repo}/${ctx.branch}/head/${workspace}/${nodePath}`;
  const response = await ctx.fetchImpl(url, { method: 'GET', headers: jsonHeaders(ctx) });
  if (response.status === 404) return null;
  if (!response.ok) {
    throw new Error(await errorMessage(response, `GET ${url} failed: ${response.status}`));
  }
  const node = (await response.json()) as { id?: string; properties?: Record<string, unknown> };
  const properties = node.properties || {};
  return {
    id: typeof node.id === 'string' ? node.id : '',
    contentHash: readContentHash(properties),
    size: readSize(properties),
  };
}

/**
 * Upload the artifact as the Function node's child asset.
 *
 * `override_existing=true` because a dev loop replaces the same node over and
 * over; without it the second run of the day 409s.
 */
export async function uploadArtifact(
  ctx: ServerContext,
  workspace: string,
  nodePath: string,
  bytes: Uint8Array,
  fileName: string
): Promise<void> {
  const url =
    `${ctx.baseUrl}/api/repository/${ctx.repo}/${ctx.branch}/head/${workspace}/${nodePath}` +
    '?override_existing=true';
  const form = new FormData();
  form.append('file', new Blob([bytes], { type: 'application/wasm' }), fileName);
  const headers: Record<string, string> = {};
  if (ctx.token) headers.Authorization = `Bearer ${ctx.token}`;
  const response = await ctx.fetchImpl(url, { method: 'POST', headers, body: form });
  if (!response.ok) {
    throw new Error(await errorMessage(response, `POST ${url} failed: ${response.status}`));
  }
}

/** The shape both run routes reduce to. */
export interface RunOutcome {
  success: boolean;
  result?: unknown;
  error?: string;
  durationMs?: number;
  logs: string[];
}

/** Invoke a deployed function synchronously. */
export async function invokeFunction(
  ctx: ServerContext,
  name: string,
  input: unknown,
  timeoutMs?: number
): Promise<RunOutcome> {
  const url = `${ctx.baseUrl}/api/functions/${ctx.repo}/${encodeURIComponent(name)}/invoke`;
  const response = await ctx.fetchImpl(url, {
    method: 'POST',
    headers: jsonHeaders(ctx),
    body: JSON.stringify({ input, sync: true, timeout_ms: timeoutMs }),
  });
  if (!response.ok) {
    throw new Error(await errorMessage(response, `POST ${url} failed: ${response.status}`));
  }
  const body = (await response.json()) as {
    result?: unknown;
    error?: string;
    duration_ms?: number;
    logs?: string[];
  };
  return {
    success: !body.error,
    result: body.result,
    error: body.error,
    durationMs: body.duration_ms,
    logs: body.logs || [],
  };
}

/** One SSE frame: its `event:` name and its parsed `data:` payload. */
export interface SseFrame {
  event: string;
  data: Record<string, unknown>;
}

/**
 * Split a growing SSE buffer into whole frames.
 *
 * Returns the frames it could complete and the trailing partial text, which the
 * caller feeds back in with the next chunk. Kept pure so the parser is tested
 * without a socket — the streaming loop below is the only stateful part.
 */
export function parseSseFrames(buffer: string): { frames: SseFrame[]; rest: string } {
  const frames: SseFrame[] = [];
  const parts = buffer.split(/\r?\n\r?\n/);
  const rest = parts.pop() ?? '';
  for (const part of parts) {
    let event = '';
    const dataLines: string[] = [];
    for (const line of part.split(/\r?\n/)) {
      if (line.startsWith('event:')) event = line.slice(6).trim();
      else if (line.startsWith('data:')) dataLines.push(line.slice(5).trim());
    }
    if (dataLines.length === 0) continue;
    try {
      const data = JSON.parse(dataLines.join('\n')) as Record<string, unknown>;
      // No `event:` line: fall back to the payload's own `type`, which every
      // RunFileEvent carries (`#[serde(tag = "type")]` on the server side).
      frames.push({ event: event || String(data.type || 'message'), data });
    } catch {
      // A non-JSON payload is a keep-alive comment or a server bug; skip it.
    }
  }
  return { frames, rest };
}

/** Body of `POST /api/files/{repo}/run`. */
export interface RunFileRequest {
  node_id: string;
  handler: string;
  input: unknown;
  timeout_ms?: number;
}

/**
 * Run an uploaded artifact and stream its SSE events.
 *
 * `onFrame` sees every frame in order; the resolved outcome is built from the
 * `result` frame, so a stream that ends without one is reported as a failure
 * rather than a silent success.
 */
export async function streamRunFile(
  ctx: ServerContext,
  request: RunFileRequest,
  onFrame: (frame: SseFrame) => void
): Promise<RunOutcome> {
  const url = `${ctx.baseUrl}/api/files/${ctx.repo}/run`;
  const response = await ctx.fetchImpl(url, {
    method: 'POST',
    headers: jsonHeaders(ctx, 'text/event-stream'),
    body: JSON.stringify(request),
  });
  if (!response.ok) {
    throw new Error(await errorMessage(response, `POST ${url} failed: ${response.status}`));
  }
  if (!response.body) throw new Error(`POST ${url} returned no body to stream`);

  const logs: string[] = [];
  let outcome: RunOutcome | null = null;
  const reader = (response.body as ReadableStream<Uint8Array>).getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const { frames, rest } = parseSseFrames(buffer);
    buffer = rest;
    for (const frame of frames) {
      onFrame(frame);
      if (frame.event === 'log') {
        const level = String(frame.data.level ?? 'info');
        logs.push(`[${level}] ${String(frame.data.message ?? '')}`);
      } else if (frame.event === 'result') {
        outcome = {
          success: frame.data.success === true,
          result: frame.data.result,
          error: typeof frame.data.error === 'string' ? frame.data.error : undefined,
          durationMs:
            typeof frame.data.duration_ms === 'number' ? frame.data.duration_ms : undefined,
          logs,
        };
      }
    }
  }
  return outcome ?? { success: false, error: 'the run stream ended without a result', logs };
}
