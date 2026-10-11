/**
 * Reading a server for `pull`: full-depth subtree scans, counts, locales, and
 * streamed binary downloads.
 *
 * Why SQL and not the REST tree listing the old code used: the listing caps a
 * deep query at 10 levels and ignores `flatten` unless `format` is given (it
 * returns top-level nodes with nested `children`, which is how `sync --pull`
 * came to see only the first level). SQL with `DESCENDANT_OF` has no depth
 * limit, honours row-level security exactly like every other read, and pages
 * by a keyset on `id` — never OFFSET, which skips or repeats rows when the
 * tree changes mid-scan. A COUNT afterwards is compared with what was read,
 * so a short read is reported instead of silently treated as "deleted".
 */

import fs from 'fs';
import path from 'path';
import crypto from 'crypto';
import { Readable, Transform } from 'stream';
import { pipeline } from 'stream/promises';
import type { JsonObject } from './normalize.js';

/** One node as pull needs it. */
export interface RemoteNode {
  id: string;
  workspace: string;
  /** Workspace-relative, leading slash: "/baden-rhinos/home" */
  path: string;
  name: string;
  node_type: string;
  archetype: string | null;
  properties: JsonObject;
  updated_at?: string;
}

/** The server surface pull depends on — a fake implements it in tests. */
export interface PullClient {
  /** Run a read-only SQL statement (`$1`… bound to `params`), return its rows. */
  sql(query: string, params?: unknown[]): Promise<Array<Record<string, unknown>>>;
  /** Stream a node's binary to `dest`, return its sha256 and size. */
  download(workspace: string, nodePath: string, dest: string): Promise<{ sha256: string; size: number }>;
  /**
   * The repositories these credentials can see. Optional: a client without it
   * skips the repository check.
   */
  listRepositories?(): Promise<RepoListing>;
  /** The repository's configured locales. */
  repoLocales(): Promise<{ defaultLanguage: string | null; supported: string[] } | null>;
}

export type RepoListing =
  | { status: 'ok'; repos: string[] }
  | { status: 'denied'; code: number }
  | { status: 'unavailable' };

export const DEFAULT_PAGE_SIZE = 500;

/** SQL string literal. */
export function sqlString(s: string): string {
  return `'${s.replace(/'/g, "''")}'`;
}

/** The subtree filter, binding the root as `$n`; null for a whole workspace. */
function subtreePredicate(root: string, params: unknown[]): string | null {
  if (root === '/' || root === '') return null;
  params.push(root);
  const n = params.length;
  return `(path = $${n} OR DESCENDANT_OF($${n}))`;
}

function rowToNode(row: Record<string, unknown>, workspace: string): RemoteNode {
  let props = row.properties as unknown;
  if (typeof props === 'string') {
    try {
      props = JSON.parse(props);
    } catch {
      props = {};
    }
  }
  return {
    id: String(row.id),
    workspace,
    path: String(row.path),
    name: String(row.name ?? String(row.path).split('/').pop()),
    node_type: String(row.node_type),
    archetype: (row.archetype as string | null) ?? null,
    properties: (props as JsonObject) ?? {},
    updated_at: row.updated_at as string | undefined,
  };
}

/**
 * Every node at and below `root` in `workspace`, all depths, paged.
 *
 * `locale` reads the locale-resolved view (`WHERE locale = 'x'`), which is how
 * translation overlays are recovered without one request per node.
 */
export async function scanSubtree(
  client: PullClient,
  workspace: string,
  root: string,
  opts: { locale?: string; pageSize?: number; onPage?: (fetched: number) => void } = {}
): Promise<RemoteNode[]> {
  const pageSize = opts.pageSize ?? DEFAULT_PAGE_SIZE;
  const nodes: RemoteNode[] = [];
  const seen = new Set<string>();
  let lastId: string | null = null;

  for (;;) {
    const params: unknown[] = [];
    const where = [subtreePredicate(root, params)];
    if (opts.locale) {
      params.push(opts.locale);
      where.push(`locale = $${params.length}`);
    }
    if (lastId) {
      params.push(lastId);
      where.push(`id > $${params.length}`);
    }
    const clauses = where.filter(Boolean);
    const query =
      `SELECT id, path, name, node_type, archetype, properties, updated_at FROM ${sqlString(workspace)}` +
      (clauses.length ? ` WHERE ${clauses.join(' AND ')}` : '') +
      ` ORDER BY id LIMIT ${pageSize}`;
    const rows = await client.sql(query, params);
    for (const row of rows) {
      const node = rowToNode(row, workspace);
      if (seen.has(node.id)) continue;
      seen.add(node.id);
      nodes.push(node);
    }
    opts.onPage?.(nodes.length);
    if (rows.length < pageSize) break;
    const next = String(rows[rows.length - 1].id);
    // A page that does not advance the keyset would loop for ever.
    if (next === lastId) break;
    lastId = next;
  }
  return nodes;
}

/** How many nodes the server says are at and below `root`. */
export async function countSubtree(client: PullClient, workspace: string, root: string): Promise<number> {
  const params: unknown[] = [];
  const base = subtreePredicate(root, params);
  const rows = await client.sql(
    `SELECT COUNT(*) AS n FROM ${sqlString(workspace)}${base ? ` WHERE ${base}` : ''}`,
    params
  );
  return Number(rows[0]?.n ?? 0);
}

/** Top-level node paths of a workspace (for "outside the package" reporting). */
export async function listTopLevel(client: PullClient, workspace: string): Promise<string[]> {
  const rows = await client.sql(`SELECT path FROM ${sqlString(workspace)} WHERE CHILD_OF('/')`);
  return rows.map((r) => String(r.path));
}

// ---------------------------------------------------------------------------
// HTTP implementation
// ---------------------------------------------------------------------------

export interface HttpClientOptions {
  server: string;
  repo: string;
  branch: string;
  token: string | null;
  fetchImpl?: typeof fetch;
}

function encodePath(nodePath: string): string {
  return nodePath
    .replace(/^\/+/, '')
    .split('/')
    .map((s) => encodeURIComponent(s))
    .join('/');
}

export class HttpError extends Error {
  constructor(message: string, public status: number) {
    super(message);
  }
}

export function createHttpClient(opts: HttpClientOptions): PullClient {
  const base = opts.server.replace(/\/+$/, '');
  const doFetch = opts.fetchImpl ?? fetch;
  const headers = (): Record<string, string> => {
    const h: Record<string, string> = { 'Content-Type': 'application/json' };
    if (opts.token) h.Authorization = `Bearer ${opts.token}`;
    return h;
  };

  return {
    async sql(query, params) {
      const url = `${base}/api/sql/${encodeURIComponent(opts.repo)}/${encodeURIComponent(opts.branch)}`;
      const res = await doFetch(url, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify(params && params.length ? { sql: query, params } : { sql: query }),
      });
      if (!res.ok) {
        const text = await res.text();
        let msg = text;
        try {
          const j = JSON.parse(text) as { message?: string; error?: string };
          msg = j.message || j.error || text;
        } catch {
          /* plain text */
        }
        throw new HttpError(`SQL failed (${res.status}): ${msg}`, res.status);
      }
      const data = (await res.json()) as { rows?: Array<Record<string, unknown>> };
      return data.rows ?? [];
    },

    async download(workspace, nodePath, dest) {
      const url =
        `${base}/api/repository/${encodeURIComponent(opts.repo)}/${encodeURIComponent(opts.branch)}` +
        `/head/${encodeURIComponent(workspace)}/${encodePath(nodePath)}/raisin:cmd/download`;
      const res = await doFetch(url, { headers: opts.token ? { Authorization: `Bearer ${opts.token}` } : {} });
      if (!res.ok || !res.body) {
        throw new HttpError(`download failed (${res.status}) for ${workspace}:${nodePath}`, res.status);
      }
      return streamToFile(Readable.fromWeb(res.body as never), dest);
    },

    async listRepositories(): Promise<RepoListing> {
      try {
        const res = await doFetch(`${base}/api/repositories`, { headers: headers() });
        if (res.status === 401 || res.status === 403) return { status: 'denied', code: res.status };
        if (!res.ok) return { status: 'unavailable' };
        const list = (await res.json()) as Array<{ repo_id?: string }>;
        if (!Array.isArray(list)) return { status: 'unavailable' };
        return { status: 'ok', repos: list.map((r) => String(r.repo_id ?? '')).filter(Boolean) };
      } catch {
        return { status: 'unavailable' };
      }
    },

    async repoLocales() {
      const res = await doFetch(`${base}/api/repositories`, { headers: headers() });
      if (!res.ok) return null;
      const list = (await res.json()) as Array<{
        repo_id?: string;
        config?: { default_language?: string; supported_languages?: string[] };
      }>;
      const repo = Array.isArray(list) ? list.find((r) => r.repo_id === opts.repo) : undefined;
      if (!repo) return null;
      return {
        defaultLanguage: repo.config?.default_language ?? null,
        supported: repo.config?.supported_languages ?? [],
      };
    },
  };
}

/**
 * Write a stream to `dest`, hashing as it goes — nothing is buffered whole.
 * Callers pass a temporary path and rename it into place once they have
 * decided to keep it, so an interrupted pull never leaves a truncated file
 * where a complete one was.
 */
export async function streamToFile(
  source: Readable,
  dest: string
): Promise<{ sha256: string; size: number }> {
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  const hash = crypto.createHash('sha256');
  let size = 0;
  const tap = new Transform({
    transform(chunk: Buffer, _enc, cb) {
      hash.update(chunk);
      size += chunk.length;
      cb(null, chunk);
    },
  });
  try {
    // `wx`: never write through something already sitting at `dest`.
    await pipeline(source, tap, fs.createWriteStream(dest, { flags: 'wx' }));
  } catch (err) {
    fs.rmSync(dest, { force: true });
    throw err;
  }
  return { sha256: hash.digest('hex'), size };
}
