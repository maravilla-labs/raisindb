import { describe, expect, it } from 'vitest';
import { RaisinHttpClient } from './http-client';

// ---------------------------------------------------------------------------
// Mock fetch that records the last call and returns a JSON 200 response.
// ---------------------------------------------------------------------------

interface RecordedCall {
  url: string;
  method: string;
  body: unknown;
}

function makeClient(): { client: RaisinHttpClient; calls: RecordedCall[] } {
  const calls: RecordedCall[] = [];
  const fetchImpl = (async (url: string, init: RequestInit) => {
    calls.push({
      url: String(url),
      method: String(init.method),
      body: init.body ? JSON.parse(init.body as string) : undefined,
    });
    return {
      ok: true,
      status: 200,
      headers: { get: (k: string) => (k === 'content-type' ? 'application/json' : null) },
      json: async () => ({ ok: true }),
      text: async () => '{"ok":true}',
    } as unknown as Response;
  }) as unknown as typeof fetch;

  const client = new RaisinHttpClient('http://localhost:8080', { fetch: fetchImpl });
  return { client, calls };
}

const BASE = 'http://localhost:8080/api/management/repo/main';

describe('HTTP schema management', () => {
  it('lists all and published', async () => {
    const { client, calls } = makeClient();
    const db = client.database('repo');

    await db.elementTypes().list();
    expect(calls.at(-1)).toMatchObject({ method: 'GET', url: `${BASE}/elementtypes` });

    await db.archetypes().list(true);
    expect(calls.at(-1)).toMatchObject({ method: 'GET', url: `${BASE}/archetypes/published` });
  });

  it('gets and resolves by name (name is URL-encoded)', async () => {
    const { client, calls } = makeClient();
    const db = client.database('repo');

    await db.elementTypes().get('marketing:Hero');
    expect(calls.at(-1)).toMatchObject({
      method: 'GET',
      url: `${BASE}/elementtypes/marketing%3AHero`,
    });

    await db.archetypes().getResolved('blog:Section');
    expect(calls.at(-1)).toMatchObject({
      method: 'GET',
      url: `${BASE}/archetypes/blog%3ASection/resolved`,
    });
  });

  it('passes workspace to NodeType getResolved', async () => {
    const { client, calls } = makeClient();
    await client.database('repo').nodeTypes().getResolved('blog:Article', { workspace: 'content' });
    expect(calls.at(-1)).toMatchObject({
      method: 'GET',
      url: `${BASE}/nodetypes/blog%3AArticle/resolved?workspace=content`,
    });
  });

  it('creates with def wrapped under defField and folds in name', async () => {
    const { client, calls } = makeClient();
    await client
      .database('repo')
      .elementTypes()
      .create('marketing:Hero', { fields: [] }, { message: 'add hero', actor: 'tester' });

    expect(calls.at(-1)).toMatchObject({
      method: 'POST',
      url: `${BASE}/elementtypes`,
      body: {
        element_type: { name: 'marketing:Hero', fields: [] },
        commit: { message: 'add hero', actor: 'tester' },
      },
    });
  });

  it('updates via PUT under the named URL', async () => {
    const { client, calls } = makeClient();
    await client.database('repo').nodeTypes().update('blog:Article', { properties: [] });
    expect(calls.at(-1)).toMatchObject({
      method: 'PUT',
      url: `${BASE}/nodetypes/blog%3AArticle`,
      body: { node_type: { name: 'blog:Article', properties: [] } },
    });
  });

  it('deletes without a body when no commit given', async () => {
    const { client, calls } = makeClient();
    await client.database('repo').archetypes().delete('blog:Section');
    expect(calls.at(-1)).toMatchObject({
      method: 'DELETE',
      url: `${BASE}/archetypes/blog%3ASection`,
      body: undefined,
    });
  });

  it('publishes and unpublishes via POST', async () => {
    const { client, calls } = makeClient();
    const db = client.database('repo');

    await db.elementTypes().publish('marketing:Hero');
    expect(calls.at(-1)).toMatchObject({
      method: 'POST',
      url: `${BASE}/elementtypes/marketing%3AHero/publish`,
    });

    await db.elementTypes().unpublish('marketing:Hero');
    expect(calls.at(-1)).toMatchObject({
      method: 'POST',
      url: `${BASE}/elementtypes/marketing%3AHero/unpublish`,
    });
  });

  it('honors a branch-scoped database', async () => {
    const { client, calls } = makeClient();
    await client.database('repo').onBranch('staging').nodeTypes().list();
    expect(calls.at(-1)).toMatchObject({
      method: 'GET',
      url: 'http://localhost:8080/api/management/repo/staging/nodetypes',
    });
  });
});
