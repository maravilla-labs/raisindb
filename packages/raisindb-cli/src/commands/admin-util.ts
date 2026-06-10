import { getBaseUrl, getHeaders } from '../api.js';

/**
 * Shared helpers for the gh-style administrative commands
 * (`repo`, `ai provider`, `user`, `cors`).
 *
 * All output here is plain text (no Ink / cursor control sequences) so that
 * CI logs stay clean regardless of TTY detection.
 */

export type FetchLike = typeof fetch;

export interface ApiCallOptions {
  method?: string;
  body?: unknown;
  fetchImpl?: FetchLike;
}

export interface ApiCallResult<T> {
  status: number;
  ok: boolean;
  data: T | null;
  /** Best-effort error message extracted from the response body */
  errorMessage: string;
}

/**
 * Perform a JSON API call against the configured server.
 * Never throws on HTTP errors - callers inspect `status` / `ok` so they can
 * give precise messages (e.g. 409 conflict handling).
 */
export async function apiCall<T>(path: string, opts: ApiCallOptions = {}): Promise<ApiCallResult<T>> {
  const fetchImpl = opts.fetchImpl ?? fetch;
  const url = `${getBaseUrl()}${path}`;

  const response = await fetchImpl(url, {
    method: opts.method ?? 'GET',
    headers: getHeaders(),
    body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
  });

  const text = await response.text().catch(() => '');
  let data: T | null = null;
  let errorMessage = '';

  if (text) {
    try {
      data = JSON.parse(text) as T;
    } catch {
      data = null;
    }
  }

  if (!response.ok) {
    const parsed = (data ?? {}) as { message?: string; error?: string };
    errorMessage = parsed.message || parsed.error || text || `HTTP ${response.status}`;
  }

  return { status: response.status, ok: response.ok, data, errorMessage };
}

/**
 * Format a plain-text table (CI-friendly, no ANSI codes).
 */
export function formatTable(headers: string[], rows: string[][]): string {
  const widths = headers.map((h, i) =>
    Math.max(h.length, ...rows.map((r) => (r[i] ?? '').length))
  );
  const formatRow = (cells: string[]) =>
    cells.map((c, i) => (c ?? '').padEnd(widths[i])).join('  ').trimEnd();
  const lines = [formatRow(headers), formatRow(widths.map((w) => '-'.repeat(w)))];
  for (const row of rows) {
    lines.push(formatRow(row));
  }
  return lines.join('\n');
}

/**
 * Read all of stdin (used for --password-stdin / --api-key-stdin).
 * Trailing newlines are stripped (echo "secret" | ... works as expected).
 */
export async function readStdin(): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) {
    chunks.push(Buffer.from(chunk));
  }
  return Buffer.concat(chunks).toString('utf-8').replace(/\r?\n+$/, '');
}

/**
 * Redact a secret for any logging context. Never exposes any part of the
 * secret value (not even a prefix).
 */
export function redactSecret(secret: string): string {
  return `<redacted:${secret.length} chars>`;
}
