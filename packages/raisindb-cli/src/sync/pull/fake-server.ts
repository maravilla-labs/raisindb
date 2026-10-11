/**
 * An in-memory server for pull tests: it answers exactly the SQL shapes
 * remote.ts sends (keyset pages, counts, top-level listings) and streams
 * downloads, so the tests exercise the real paging and the real decisions.
 */

import { Readable } from 'stream';
import type { JsonObject } from './normalize.js';
import { streamToFile, type PullClient, type RepoListing } from './remote.js';

export interface FakeNode {
  workspace: string;
  path: string;
  node_type: string;
  archetype?: string | null;
  properties: JsonObject;
  /** Locale → localized properties (what `WHERE locale = x` returns); null = hidden */
  locales?: Record<string, JsonObject | null>;
  /** Bytes served by raisin:cmd/download */
  bytes?: Buffer;
  updated_at?: string;
}

const isUnder = (p: string, root: string) => root === '/' || p === root || p.startsWith(`${root}/`);

export class FakeServer implements PullClient {
  nodes: FakeNode[] = [];
  sqlCalls = 0;
  downloads: string[] = [];
  /** Answer every read with nothing, as an unauthenticated server does. */
  anonymous = false;
  locales: { defaultLanguage: string | null; supported: string[] } | null = null;
  repos: string[] = ['site'];
  /** HTTP status for the repository listing (401/403 to refuse it) */
  repoListingStatus = 200;
  private ids = new Map<string, string>();

  add(node: FakeNode): this {
    this.nodes.push(node);
    return this;
  }

  idOf(n: FakeNode): string {
    const k = `${n.workspace}:${n.path}`;
    if (!this.ids.has(k)) this.ids.set(k, `id${String(this.ids.size).padStart(6, '0')}`);
    return this.ids.get(k)!;
  }

  async sql(query: string, params: unknown[] = []): Promise<Array<Record<string, unknown>>> {
    this.sqlCalls++;
    const ws = /FROM '((?:[^']|'')+)'/.exec(query)?.[1].replace(/''/g, "'");
    if (!ws) throw new Error(`fake: no workspace in ${query}`);
    if (!this.nodes.some((n) => n.workspace === ws) && !this.anonymous) {
      throw new Error(`Batch analysis error: Table not found: ${ws}`);
    }
    if (this.anonymous) return query.includes('COUNT(*)') ? [{ n: 0 }] : [];

    const param = (re: RegExp) => {
      const m = re.exec(query);
      return m ? (params[Number(m[1]) - 1] as string) : undefined;
    };
    const root = param(/path = \$(\d+)/) ?? '/';
    const locale = param(/locale = \$(\d+)/);
    const after = param(/id > \$(\d+)/);
    const limit = Number(/LIMIT (\d+)/.exec(query)?.[1] ?? Infinity);

    let rows = this.nodes.filter((n) => n.workspace === ws);
    if (query.includes("CHILD_OF('/')")) {
      return rows.filter((n) => n.path.split('/').length === 2).map((n) => ({ path: n.path }));
    }
    rows = rows.filter((n) => isUnder(n.path, root));
    if (query.includes('COUNT(*)')) return [{ n: rows.length }];
    if (locale) rows = rows.filter((n) => !n.locales || n.locales[locale] !== null);

    return rows
      .map((n) => ({
        id: this.idOf(n),
        path: n.path,
        name: n.path.split('/').pop(),
        node_type: n.node_type,
        archetype: n.archetype ?? null,
        properties: locale && n.locales?.[locale] ? n.locales[locale] : n.properties,
        updated_at: n.updated_at ?? '2026-10-01T00:00:00Z',
      }))
      .filter((r) => !after || r.id > after)
      .sort((a, b) => (a.id < b.id ? -1 : 1))
      .slice(0, limit);
  }

  async download(workspace: string, nodePath: string, dest: string) {
    this.downloads.push(`${workspace}:${nodePath}`);
    const node = this.nodes.find((n) => n.workspace === workspace && n.path === nodePath);
    if (!node?.bytes) throw new Error(`fake: no bytes for ${workspace}:${nodePath}`);
    // Small chunks, so the streaming path is really exercised.
    const chunks: Buffer[] = [];
    for (let i = 0; i < node.bytes.length; i += 1024) chunks.push(node.bytes.subarray(i, i + 1024));
    return streamToFile(Readable.from(chunks), dest);
  }

  async listRepositories(): Promise<RepoListing> {
    if (this.repoListingStatus === 401 || this.repoListingStatus === 403) {
      return { status: 'denied', code: this.repoListingStatus };
    }
    return { status: 'ok', repos: this.anonymous ? [] : this.repos };
  }

  async repoLocales() {
    return this.locales;
  }
}
