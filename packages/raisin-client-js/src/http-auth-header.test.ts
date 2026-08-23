import { describe, it, expect } from 'vitest';
import { RaisinHttpClient } from './http-client';

/** Captures the headers of every call the client makes. */
function spyFetch() {
  const seen: Array<{ url: string; headers: Record<string, string> }> = [];
  const impl = (async (url: string | URL, init?: RequestInit) => {
    seen.push({
      url: String(url),
      headers: (init?.headers ?? {}) as Record<string, string>,
    });
    return new Response(JSON.stringify({ id: 'x' }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }) as unknown as typeof fetch;
  return { seen, impl };
}

describe('Authorization header', () => {
  // The regression: a client holding a bare token — no expiry, because nothing
  // told it one — sent no Authorization header at all, so every call ran
  // anonymously. It failed SILENTLY, since an anonymous read returns an empty
  // result rather than a 401.
  it('is sent for a bare token, whose expiry is unknown', async () => {
    const { seen, impl } = spyFetch();
    const client = new RaisinHttpClient('http://db.test', { fetch: impl });
    client.setIdentityTokens('tok-123');

    await client.auth('studio').me();

    expect(seen[0].headers['Authorization']).toBe('Bearer tok-123');
  });

  it('is sent after authenticate({type: jwt}) too', async () => {
    const { seen, impl } = spyFetch();
    const client = new RaisinHttpClient('http://db.test', { fetch: impl });
    await client.authenticate({ type: 'jwt', token: 'tok-jwt' });

    await client.auth('studio').me();

    const me = seen.find((c) => c.url.endsWith('/auth/studio/me'));
    expect(me?.headers['Authorization']).toBe('Bearer tok-jwt');
  });

  // The other half: a sign-in must NOT carry the previous identity's token.
  it('is withheld from sign-in calls even when a token is held', async () => {
    const { seen, impl } = spyFetch();
    const client = new RaisinHttpClient('http://db.test', { fetch: impl });
    client.setIdentityTokens('tok-123');

    await client.auth('studio').login('a@example.com', 'pw');

    expect(seen[0].headers['Authorization']).toBeUndefined();
  });

  it('is absent when no token is held at all', async () => {
    const { seen, impl } = spyFetch();
    const client = new RaisinHttpClient('http://db.test', { fetch: impl });

    await client.auth('studio').me();

    expect(seen[0].headers['Authorization']).toBeUndefined();
  });
});
