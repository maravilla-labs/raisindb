import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
  encodeSecretName,
  formatSecretTable,
  resolveScope,
  resolveSecretValue,
  secretRotatePath,
  secretsBasePath,
  secretList,
  secretRemove,
  secretRotate,
  secretSet,
  secretShow,
  validateSecretName,
} from './secret.js';
import type { FetchLike } from './admin-util.js';

/**
 * Two kinds of test here:
 *  - pure logic (scope resolution, value sourcing, name validation, table);
 *  - command flows against an injected fetch, asserting the exact endpoint,
 *    method and body — including that no command ever asks for plaintext back.
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
      return new Response(JSON.stringify({ message: `no mock for ${method} ${url}` }), {
        status: 500,
      });
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

describe('resolveScope', () => {
  it('uses --repo and defaults the branch to main', () => {
    expect(resolveScope({ repo: 'shop' }, () => null)).toEqual({ repo: 'shop', branch: 'main' });
  });

  it('falls back to the configured default repo', () => {
    expect(resolveScope({}, () => 'configured')).toEqual({
      repo: 'configured',
      branch: 'main',
    });
  });

  it('honours an explicit branch (secrets are branch-scoped)', () => {
    expect(resolveScope({ repo: 'shop', branch: 'staging' }, () => null)).toEqual({
      repo: 'shop',
      branch: 'staging',
    });
  });

  it('errors rather than silently defaulting to "default"', () => {
    expect(() => resolveScope({}, () => null)).toThrow(/No repository specified/);
  });
});

describe('secretsBasePath', () => {
  it('encodes each segment', () => {
    expect(secretsBasePath('my repo', 'feature/x')).toBe('/api/secrets/my%20repo/feature%2Fx');
  });
});

describe('encodeSecretName', () => {
  it('keeps `/` as a real separator (the server captures the name as a wildcard)', () => {
    expect(encodeSecretName('node/01H8XY/api_key')).toBe('node/01H8XY/api_key');
  });

  it('still escapes everything else, so a `#` or space cannot truncate the request', () => {
    expect(encodeSecretName('weird name#1')).toBe('weird%20name%231');
    expect(encodeSecretName('a/b c')).toBe('a/b%20c');
  });
});

describe('secretRotatePath', () => {
  // The literal segment comes FIRST: the name is a wildcard capture and a
  // wildcard must be the last path segment, so `{name}/rotate` cannot exist.
  it('puts `rotate` before the name', () => {
    expect(secretRotatePath('shop', 'main', 'api_key')).toBe(
      '/api/secrets/shop/main/rotate/api_key'
    );
  });

  it('keeps a slashed name intact after the rotate segment', () => {
    expect(secretRotatePath('shop', 'main', 'node/01H8XY/api_key')).toBe(
      '/api/secrets/shop/main/rotate/node/01H8XY/api_key'
    );
  });
});

describe('validateSecretName', () => {
  it('accepts an ordinary name', () => {
    expect(validateSecretName('stripe/live')).toBe('stripe/live');
  });

  it('rejects empty, padded and NUL-bearing names', () => {
    expect(() => validateSecretName('')).toThrow(/must not be empty/);
    expect(() => validateSecretName('   ')).toThrow(/must not be empty/);
    expect(() => validateSecretName(' padded')).toThrow(/whitespace/);
    expect(() => validateSecretName('a\0b')).toThrow(/NUL/);
  });
});

describe('resolveSecretValue', () => {
  it('reads stdin by DEFAULT, with no flag (keeps values out of shell history)', async () => {
    const result = await resolveSecretValue({}, { readStdinImpl: async () => 'sk_live_x' });
    expect(result).toEqual({ value: 'sk_live_x', source: 'stdin' });
  });

  it('accepts --value when explicitly given', async () => {
    expect(await resolveSecretValue({ value: 'v' })).toEqual({ value: 'v', source: 'flag' });
  });

  it('reads --value-env from the provided env', async () => {
    const result = await resolveSecretValue(
      { valueEnv: 'MY_SECRET' },
      { env: { MY_SECRET: '  sk_from_env  ' } }
    );
    expect(result).toEqual({ value: 'sk_from_env', source: 'env:MY_SECRET' });
  });

  it('errors when the env var is missing', async () => {
    await expect(resolveSecretValue({ valueEnv: 'NOPE' }, { env: {} })).rejects.toThrow(
      /NOPE is not set or empty/
    );
  });

  it('rejects empty stdin with an actionable message', async () => {
    await expect(resolveSecretValue({}, { readStdinImpl: async () => '' })).rejects.toThrow(
      /No secret value received on stdin/
    );
  });

  it('rejects two value sources at once', async () => {
    await expect(resolveSecretValue({ value: 'a', valueEnv: 'B' })).rejects.toThrow(
      /mutually exclusive/
    );
  });

  it('never includes the value in an error message', async () => {
    const secret = 'super-secret-value-9000';
    try {
      await resolveSecretValue({ value: secret, valueEnv: 'B' }, { env: { B: secret } });
      expect.unreachable('should have thrown');
    } catch (e) {
      expect((e as Error).message).not.toContain(secret);
    }
  });
});

describe('formatSecretTable', () => {
  it('shows tombstoned names as deleted rather than hiding them', () => {
    const table = formatSecretTable([
      { name: 'stripe/live', version: 3, created_at: '2026-08-01T00:00:00Z', created_by: 'ada' },
      { name: 'old_key', version: 2, created_at: '2026-07-01T00:00:00Z', deleted: true },
    ]);
    expect(table).toContain('stripe/live');
    expect(table).toContain('active');
    expect(table).toContain('old_key');
    expect(table).toContain('deleted');
    // eslint-disable-next-line no-control-regex
    expect(table).not.toMatch(//);
  });
});

describe('secretSet', () => {
  it('PUTs { value } to /api/secrets/{repo}/{branch}/{name}', async () => {
    const { fetchImpl, calls } = mockFetch([
      { match: (_u, m) => m === 'PUT', status: 200, body: { name: 'stripe/live', version: 1 } },
    ]);

    await secretSet(
      'stripe/live',
      { repo: 'shop', branch: 'main' },
      fetchImpl,
      { readStdinImpl: async () => 'sk_live_x' }
    );

    expect(calls).toHaveLength(1);
    expect(calls[0].method).toBe('PUT');
    expect(calls[0].url).toBe('http://test-server:1234/api/secrets/shop/main/stripe/live');
    expect(calls[0].body).toEqual({ value: 'sk_live_x' });
  });

  it('prints the secret:// reference to paste into a property', async () => {
    const logs: string[] = [];
    (console.log as unknown as ReturnType<typeof vi.fn>).mockImplementation((...a: unknown[]) =>
      logs.push(a.join(' '))
    );
    const { fetchImpl } = mockFetch([
      {
        match: (_u, m) => m === 'PUT',
        status: 200,
        body: { name: 'stripe-key', version: 1, reference: 'secret://stripe-key' },
      },
    ]);

    await secretSet('stripe-key', { repo: 'shop' }, fetchImpl, {
      readStdinImpl: async () => 'v',
    });

    expect(logs.join('\n')).toContain('secret://stripe-key');
  });

  it('never prints the value it wrote', async () => {
    const logs: string[] = [];
    (console.log as unknown as ReturnType<typeof vi.fn>).mockImplementation((...a: unknown[]) =>
      logs.push(a.join(' '))
    );
    const { fetchImpl } = mockFetch([
      { match: (_u, m) => m === 'PUT', status: 200, body: { name: 'k', version: 1 } },
    ]);

    await secretSet('k', { repo: 'shop' }, fetchImpl, {
      readStdinImpl: async () => 'TOP_SECRET_VALUE',
    });

    expect(logs.join('\n')).not.toContain('TOP_SECRET_VALUE');
  });

  it('surfaces a server error with the scope in the message', async () => {
    const { fetchImpl } = mockFetch([
      { match: (_u, m) => m === 'PUT', status: 500, body: { message: 'keyring missing' } },
    ]);
    await expect(
      secretSet('k', { repo: 'shop' }, fetchImpl, { readStdinImpl: async () => 'v' })
    ).rejects.toThrow(/shop\/main.*keyring missing/);
  });
});

describe('secretList', () => {
  it('GETs the branch collection', async () => {
    const { fetchImpl, calls } = mockFetch([
      {
        match: (_u, m) => m === 'GET',
        status: 200,
        body: { secrets: [{ name: 'a', version: 1 }] },
      },
    ]);

    await secretList({ repo: 'shop', branch: 'staging' }, fetchImpl);

    expect(calls[0].method).toBe('GET');
    expect(calls[0].url).toBe('http://test-server:1234/api/secrets/shop/staging');
  });

  it('reports an empty branch without erroring', async () => {
    const { fetchImpl } = mockFetch([
      { match: (_u, m) => m === 'GET', status: 200, body: { secrets: [] } },
    ]);
    await expect(secretList({ repo: 'shop' }, fetchImpl)).resolves.toBeUndefined();
  });
});

describe('secretShow', () => {
  it('prints every version when the body carries history', async () => {
    const logs: string[] = [];
    (console.log as unknown as ReturnType<typeof vi.fn>).mockImplementation((...a: unknown[]) =>
      logs.push(a.join(' '))
    );
    const { fetchImpl } = mockFetch([
      {
        match: (_u, m) => m === 'GET',
        status: 200,
        body: {
          name: 'stripe-key',
          version: 2,
          created_by: 'ada',
          deleted: false,
          versions: [
            { name: 'stripe-key', version: 2, created_by: 'ada' },
            { name: 'stripe-key', version: 1, created_by: 'grace' },
          ],
        },
      },
    ]);

    await secretShow('stripe-key', { repo: 'shop' }, fetchImpl);

    const out = logs.join('\n');
    expect(out).toContain('grace');
    expect(out).toContain('ada');
  });

  it('GETs one name and prints metadata only', async () => {
    const logs: string[] = [];
    (console.log as unknown as ReturnType<typeof vi.fn>).mockImplementation((...a: unknown[]) =>
      logs.push(a.join(' '))
    );
    const { fetchImpl, calls } = mockFetch([
      {
        match: (_u, m) => m === 'GET',
        status: 200,
        body: { name: 'stripe/live', version: 2, created_by: 'ada' },
      },
    ]);

    await secretShow('stripe/live', { repo: 'shop' }, fetchImpl);

    expect(calls[0].url).toBe('http://test-server:1234/api/secrets/shop/main/stripe/live');
    const out = logs.join('\n');
    expect(out).toContain('stripe/live');
    expect(out).toContain('ada');
  });

  it('turns a 404 into a clear not-found message', async () => {
    const { fetchImpl } = mockFetch([
      { match: (_u, m) => m === 'GET', status: 404, body: { message: 'nope' } },
    ]);
    await expect(secretShow('missing', { repo: 'shop' }, fetchImpl)).rejects.toThrow(
      /Secret 'missing' not found in shop\/main/
    );
  });
});

describe('secretRotate', () => {
  it('POSTs { value } to .../rotate/{name} (literal segment first)', async () => {
    const { fetchImpl, calls } = mockFetch([
      {
        match: (u, m) => m === 'POST' && u.includes('/rotate/'),
        status: 200,
        body: { name: 'api_key', version: 3, reference: 'secret://api_key' },
      },
    ]);

    await secretRotate('api_key', { repo: 'shop' }, fetchImpl, {
      readStdinImpl: async () => 'new_value',
    });

    expect(calls[0].method).toBe('POST');
    expect(calls[0].url).toBe('http://test-server:1234/api/secrets/shop/main/rotate/api_key');
    expect(calls[0].body).toEqual({ value: 'new_value' });
  });

  it('handles a slashed (auto-vault) name', async () => {
    const { fetchImpl, calls } = mockFetch([
      { match: (_u, m) => m === 'POST', status: 200, body: { name: 'x', version: 2 } },
    ]);

    await secretRotate('node/01H8XY/api_key', { repo: 'shop' }, fetchImpl, {
      readStdinImpl: async () => 'v',
    });

    expect(calls[0].url).toBe(
      'http://test-server:1234/api/secrets/shop/main/rotate/node/01H8XY/api_key'
    );
  });
});

describe('secretRemove', () => {
  it('refuses without --yes, and makes no request', async () => {
    const { fetchImpl, calls } = mockFetch([]);
    await expect(secretRemove('api_key', { repo: 'shop' }, fetchImpl)).rejects.toThrow(/--yes/);
    expect(calls).toHaveLength(0);
  });

  it('DELETEs the name when confirmed', async () => {
    const { fetchImpl, calls } = mockFetch([
      { match: (_u, m) => m === 'DELETE', status: 200, body: { name: 'api_key', version: 4 } },
    ]);

    await secretRemove('api_key', { repo: 'shop', yes: true }, fetchImpl);

    expect(calls[0].method).toBe('DELETE');
    expect(calls[0].url).toBe('http://test-server:1234/api/secrets/shop/main/api_key');
  });

  it('reports the tombstone ordinal (a delete IS a version)', async () => {
    const logs: string[] = [];
    (console.log as unknown as ReturnType<typeof vi.fn>).mockImplementation((...a: unknown[]) =>
      logs.push(a.join(' '))
    );
    const { fetchImpl } = mockFetch([
      {
        match: (_u, m) => m === 'DELETE',
        status: 200,
        body: { name: 'api_key', version: 4, deleted: true },
      },
    ]);

    await secretRemove('api_key', { repo: 'shop', yes: true }, fetchImpl);

    expect(logs.join('\n')).toContain('version 4');
  });
});
