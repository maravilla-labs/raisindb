import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { aiProviderSet } from './ai.js';
import { corsAdd, corsRemove, validateOrigin, addOrigin, removeOrigin } from './cors.js';
import { userRegister, resolvePassword } from './user.js';
import { repoCreate, repoDelete } from './repo.js';
import type { FetchLike } from './admin-util.js';

/**
 * Command-flow tests with an injected fetch: verify the exact endpoints hit
 * and the read-modify-write request bodies, without a live server.
 */

interface RecordedCall {
  url: string;
  method: string;
  body: unknown;
}

function mockFetch(
  routes: Array<{ match: (url: string, method: string) => boolean; status: number; body: unknown }>
): { fetchImpl: FetchLike; calls: RecordedCall[] } {
  const calls: RecordedCall[] = [];
  const fetchImpl = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? 'GET';
    calls.push({
      url,
      method,
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
    });
    const route = routes.find((r) => r.match(url, method));
    if (!route) {
      return new Response(JSON.stringify({ message: `no mock for ${method} ${url}` }), { status: 500 });
    }
    if (route.status === 204 || route.body === null) {
      return new Response(null, { status: route.status });
    }
    return new Response(JSON.stringify(route.body), { status: route.status });
  }) as FetchLike;
  return { fetchImpl, calls };
}

const savedEnv: Record<string, string | undefined> = {};

beforeEach(() => {
  for (const key of ['RAISINDB_SERVER', 'RAISINDB_TOKEN']) {
    savedEnv[key] = process.env[key];
  }
  process.env.RAISINDB_SERVER = 'http://test-server:1234';
  process.env.RAISINDB_TOKEN = 'test-token';
  vi.spyOn(console, 'log').mockImplementation(() => {});
});

afterEach(() => {
  for (const key of ['RAISINDB_SERVER', 'RAISINDB_TOKEN']) {
    if (savedEnv[key] === undefined) {
      delete process.env[key];
    } else {
      process.env[key] = savedEnv[key];
    }
  }
  vi.restoreAllMocks();
});

describe('aiProviderSet (GET -> merge -> PUT)', () => {
  const currentConfig = {
    tenant_id: 'default',
    providers: [
      { provider: 'openai', has_api_key: true, enabled: true, models: [] },
    ],
    embedding_settings: { provider: 'openai' },
  };

  it('reads current config and PUTs the merged provider list', async () => {
    const { fetchImpl, calls } = mockFetch([
      { match: (u, m) => m === 'GET' && u.includes('/ai/config'), status: 200, body: currentConfig },
      { match: (u, m) => m === 'PUT' && u.includes('/ai/config'), status: 200, body: { success: true } },
    ]);

    await aiProviderSet(
      'groq',
      { apiKeyEnv: 'TEST_GROQ_KEY', enabled: true, model: ['llama-3.3-70b-versatile'] },
      fetchImpl,
      { env: { TEST_GROQ_KEY: 'gsk-secret' } }
    );

    expect(calls).toHaveLength(2);
    expect(calls[0].url).toBe('http://test-server:1234/api/tenants/default/ai/config');
    expect(calls[1].method).toBe('PUT');

    const putBody = calls[1].body as {
      providers: Array<Record<string, unknown>>;
      embedding_settings: unknown;
    };
    // openai preserved without api_key_plain
    const openai = putBody.providers.find((p) => p.provider === 'openai')!;
    expect(openai.api_key_plain).toBeUndefined();
    // groq added with the key, default model settings
    const groq = putBody.providers.find((p) => p.provider === 'groq')!;
    expect(groq.api_key_plain).toBe('gsk-secret');
    expect(groq.enabled).toBe(true);
    expect(groq.models).toEqual([
      {
        model_id: 'llama-3.3-70b-versatile',
        display_name: 'llama-3.3-70b-versatile',
        use_cases: ['chat', 'agent'],
        default_temperature: 0.3,
        default_max_tokens: 1024,
        is_default: true,
      },
    ]);
    // embedding settings passed through
    expect(putBody.embedding_settings).toEqual({ provider: 'openai' });
  });

  it('never prints the API key', async () => {
    const logSpy = console.log as ReturnType<typeof vi.fn>;
    const { fetchImpl } = mockFetch([
      { match: (u, m) => m === 'GET', status: 200, body: currentConfig },
      { match: (u, m) => m === 'PUT', status: 200, body: { success: true } },
    ]);

    await aiProviderSet('groq', { apiKey: 'gsk-very-secret' }, fetchImpl);

    const allOutput = logSpy.mock.calls.flat().join('\n');
    expect(allOutput).not.toContain('gsk-very-secret');
    expect(allOutput).toContain('api_key=updated');
  });

  it('rejects unknown providers before any network call', async () => {
    const { fetchImpl, calls } = mockFetch([]);
    await expect(aiProviderSet('not-a-provider', {}, fetchImpl)).rejects.toThrow(/Unknown provider/);
    expect(calls).toHaveLength(0);
  });

  it('rejects --enabled with --disabled', async () => {
    const { fetchImpl } = mockFetch([]);
    await expect(
      aiProviderSet('groq', { enabled: true, disabled: true }, fetchImpl)
    ).rejects.toThrow(/mutually exclusive/);
  });
});

describe('cors repo-level (raisin:system RepoAuthConfig node)', () => {
  const nodeUrl = (repo: string) =>
    `http://test-server:1234/api/repository/${repo}/main/head/raisin:system/config/repos/${repo}`;

  it('updates the existing node via PUT (read-modify-write)', async () => {
    const { fetchImpl, calls } = mockFetch([
      {
        match: (u, m) => m === 'GET' && u === nodeUrl('myrepo'),
        status: 200,
        body: { properties: { cors_allowed_origins: ['http://a.example'] } },
      },
      { match: (u, m) => m === 'PUT' && u === nodeUrl('myrepo'), status: 200, body: {} },
    ]);

    await corsAdd('http://localhost:5173', { repo: 'myrepo' }, fetchImpl);

    const put = calls.find((c) => c.method === 'PUT')!;
    expect(put.body).toMatchObject({
      properties: { cors_allowed_origins: ['http://a.example', 'http://localhost:5173'] },
    });
  });

  it('creates the RepoAuthConfig node (and parent folders) when missing', async () => {
    const base = 'http://test-server:1234/api/repository/myrepo/main/head/raisin:system';
    const { fetchImpl, calls } = mockFetch([
      { match: (u, m) => m === 'GET' && u === nodeUrl('myrepo'), status: 404, body: { message: 'not found' } },
      { match: (u, m) => m === 'POST' && u === `${base}/`, status: 409, body: { message: 'exists' } },
      { match: (u, m) => m === 'POST' && u === `${base}/config`, status: 201, body: {} },
      { match: (u, m) => m === 'POST' && u === `${base}/config/repos`, status: 201, body: {} },
    ]);

    await corsAdd('http://localhost:5173', { repo: 'myrepo' }, fetchImpl);

    const createCall = calls.find((c) => c.method === 'POST' && c.url === `${base}/config/repos`)!;
    expect(createCall.body).toMatchObject({
      name: 'myrepo',
      node_type: 'raisin:RepoAuthConfig',
      properties: { repo_id: 'myrepo', cors_allowed_origins: ['http://localhost:5173'] },
    });
  });

  it('is idempotent: does not PUT when the origin is already present', async () => {
    const { fetchImpl, calls } = mockFetch([
      {
        match: (u, m) => m === 'GET',
        status: 200,
        body: { properties: { cors_allowed_origins: ['http://localhost:5173'] } },
      },
    ]);

    await corsAdd('http://localhost:5173', { repo: 'myrepo' }, fetchImpl);
    expect(calls.filter((c) => c.method !== 'GET')).toHaveLength(0);
  });

  it('removes an origin via PUT', async () => {
    const { fetchImpl, calls } = mockFetch([
      {
        match: (u, m) => m === 'GET',
        status: 200,
        body: { properties: { cors_allowed_origins: ['http://a.example', 'http://b.example'] } },
      },
      { match: (u, m) => m === 'PUT', status: 200, body: {} },
    ]);

    await corsRemove('http://a.example', { repo: 'myrepo' }, fetchImpl);
    const put = calls.find((c) => c.method === 'PUT')!;
    expect(put.body).toMatchObject({
      properties: { cors_allowed_origins: ['http://b.example'] },
    });
  });
});

describe('cors tenant-level (PUT /api/tenants/{t}/auth/config)', () => {
  it('adds an origin to the tenant auth config', async () => {
    const url = 'http://test-server:1234/api/tenants/default/auth/config';
    const { fetchImpl, calls } = mockFetch([
      { match: (u, m) => m === 'GET' && u === url, status: 200, body: { cors_allowed_origins: [] } },
      { match: (u, m) => m === 'PUT' && u === url, status: 200, body: {} },
    ]);

    await corsAdd('https://app.example.com', { tenantLevel: true }, fetchImpl);

    const put = calls.find((c) => c.method === 'PUT')!;
    expect(put.body).toEqual({ cors_allowed_origins: ['https://app.example.com'] });
  });
});

describe('cors helpers', () => {
  it('validateOrigin normalizes and rejects paths', () => {
    expect(validateOrigin('http://localhost:5173/')).toBe('http://localhost:5173');
    expect(validateOrigin('*')).toBe('*');
    expect(() => validateOrigin('not a url')).toThrow(/Invalid origin/);
    expect(() => validateOrigin('http://x.example/path')).toThrow(/path/);
  });

  it('addOrigin / removeOrigin report change status', () => {
    expect(addOrigin(['a'], 'a').changed).toBe(false);
    expect(addOrigin(['a'], 'b')).toEqual({ origins: ['a', 'b'], changed: true });
    expect(removeOrigin(['a'], 'b').changed).toBe(false);
    expect(removeOrigin(['a', 'b'], 'a')).toEqual({ origins: ['b'], changed: true });
  });
});

describe('userRegister', () => {
  const adminUrl = 'http://test-server:1234/api/raisindb/sys/default/identity-users';

  it('uses the admin endpoint with repos + verified email', async () => {
    const { fetchImpl, calls } = mockFetch([
      { match: (u, m) => m === 'POST' && u === adminUrl, status: 201, body: { id: 'x', email: 'a@b.c' } },
    ]);

    await userRegister(
      'a@b.c',
      { password: 'Secret12345!', repo: 'myrepo', displayName: 'A' },
      fetchImpl
    );

    expect(calls).toHaveLength(1);
    expect(calls[0].body).toEqual({
      email: 'a@b.c',
      password: 'Secret12345!',
      display_name: 'A',
      email_verified: true,
      repos: ['myrepo'],
    });
  });

  it('falls back to /auth/{repo}/register when the token lacks admin access', async () => {
    const { fetchImpl, calls } = mockFetch([
      { match: (u, m) => m === 'POST' && u === adminUrl, status: 403, body: { message: 'forbidden' } },
      {
        match: (u, m) => m === 'POST' && u === 'http://test-server:1234/auth/myrepo/register',
        status: 201,
        body: {},
      },
    ]);

    await userRegister('a@b.c', { password: 'Secret12345!', repo: 'myrepo' }, fetchImpl);
    expect(calls).toHaveLength(2);
    expect(calls[1].body).toMatchObject({ email: 'a@b.c', password: 'Secret12345!' });
  });

  it('treats 409 as success only with --exists-ok', async () => {
    const conflict = mockFetch([
      { match: (u, m) => m === 'POST' && u === adminUrl, status: 409, body: { message: 'EMAIL_EXISTS' } },
    ]);
    await expect(
      userRegister('a@b.c', { password: 'p1234567890!', repo: 'r' }, conflict.fetchImpl)
    ).rejects.toThrow(/already exists/);

    const conflictOk = mockFetch([
      { match: (u, m) => m === 'POST' && u === adminUrl, status: 409, body: { message: 'EMAIL_EXISTS' } },
    ]);
    await expect(
      userRegister('a@b.c', { password: 'p1234567890!', repo: 'r', existsOk: true }, conflictOk.fetchImpl)
    ).resolves.toBeUndefined();
  });

  it('resolvePassword: requires exactly one source and supports stdin', async () => {
    await expect(resolvePassword({})).rejects.toThrow(/password is required/i);
    await expect(resolvePassword({ password: 'a', passwordStdin: true })).rejects.toThrow(
      /mutually exclusive/
    );
    await expect(
      resolvePassword({ passwordStdin: true }, { readStdinImpl: async () => 'pw\n' })
    ).resolves.toBe('pw');
  });
});

describe('repo create/delete', () => {
  it('creates a repository via POST /api/repositories', async () => {
    const { fetchImpl, calls } = mockFetch([
      {
        match: (u, m) => m === 'POST' && u === 'http://test-server:1234/api/repositories',
        status: 201,
        body: { repo_id: 'newrepo' },
      },
    ]);
    await repoCreate('newrepo', {}, fetchImpl);
    expect(calls[0].body).toEqual({ repo_id: 'newrepo' });
  });

  it('409 fails without --exists-ok and succeeds with it', async () => {
    const make = () =>
      mockFetch([
        { match: (u, m) => m === 'POST', status: 409, body: { message: 'already exists' } },
      ]);
    await expect(repoCreate('r', {}, make().fetchImpl)).rejects.toThrow(/already exists/);
    await expect(repoCreate('r', { existsOk: true }, make().fetchImpl)).resolves.toBeUndefined();
  });

  it('delete refuses to run without --yes', async () => {
    const { fetchImpl, calls } = mockFetch([]);
    await expect(repoDelete('r', {}, fetchImpl)).rejects.toThrow(/--yes/);
    expect(calls).toHaveLength(0);
  });

  it('delete issues DELETE /api/repositories/{repo} with --yes', async () => {
    const { fetchImpl, calls } = mockFetch([
      { match: (u, m) => m === 'DELETE', status: 204, body: null },
    ]);
    await repoDelete('r', { yes: true }, fetchImpl);
    expect(calls[0].url).toBe('http://test-server:1234/api/repositories/r');
    expect(calls[0].method).toBe('DELETE');
  });
});
