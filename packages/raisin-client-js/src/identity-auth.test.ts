import { describe, it, expect } from 'vitest';
import { IdentityAuthApi, readTokensFromFragment } from './identity-auth';

/** Records what the API asked the transport to do. */
function recorder(reply: unknown = {}) {
  const calls: Array<{ method: string; path: string; body?: unknown; skipAuth?: boolean }> = [];
  const transport = async <T>(options: {
    method: string;
    path: string;
    body?: unknown;
    skipAuth?: boolean;
  }): Promise<T> => {
    calls.push(options);
    return reply as T;
  };
  return { calls, transport };
}

describe('IdentityAuthApi', () => {
  it('scopes every call to the repository it was built for', async () => {
    const { calls, transport } = recorder();
    const auth = new IdentityAuthApi('studio', transport);

    await auth.sendMagicLink('a@example.com');
    await auth.providers();
    await auth.me();

    expect(calls.map((c) => c.path)).toEqual([
      '/auth/studio/magic-link',
      '/auth/studio/providers',
      '/auth/studio/me',
    ]);
  });

  it('escapes the repository so a crafted name cannot reach another route', async () => {
    const { calls, transport } = recorder();
    await new IdentityAuthApi('../admin', transport).providers();
    expect(calls[0].path).toBe('/auth/..%2Fadmin/providers');
  });

  // A sign-in call must not carry the *previous* identity's token: on a shared
  // browser that is how one person's session gets attached to another's
  // sign-in attempt.
  it('sends sign-in calls unauthenticated', async () => {
    const { calls, transport } = recorder();
    const auth = new IdentityAuthApi('studio', transport);

    await auth.sendMagicLink('a@example.com');
    await auth.login('a@example.com', 'pw');
    await auth.register('a@example.com', 'pw');
    await auth.refresh('rt');
    await auth.verifyMagicLink('tok');

    expect(calls.every((c) => c.skipAuth === true)).toBe(true);
  });

  it('sends `me` WITH the token — it is the one call that asks about the caller', async () => {
    const { calls, transport } = recorder();
    await new IdentityAuthApi('studio', transport).me();
    expect(calls[0].skipAuth).toBeUndefined();
  });

  it('passes the redirect URL through under the name the server expects', async () => {
    const { calls, transport } = recorder();
    await new IdentityAuthApi('studio', transport).sendMagicLink('a@example.com', {
      redirectUrl: 'https://app.example.com/account/callback',
    });
    expect(calls[0].body).toEqual({
      email: 'a@example.com',
      redirect_url: 'https://app.example.com/account/callback',
    });
  });

  it('encodes the verify token rather than splicing it into the query', async () => {
    const { calls, transport } = recorder();
    await new IdentityAuthApi('studio', transport).verifyMagicLink('a&b=c');
    expect(calls[0].path).toBe('/auth/studio/magic-link/verify?token=a%26b%3Dc');
  });
});

describe('readTokensFromFragment', () => {
  it('reads the pair a verify redirect leaves behind', () => {
    expect(readTokensFromFragment('#access_token=at&refresh_token=rt')).toEqual({
      accessToken: 'at',
      refreshToken: 'rt',
    });
  });

  it('accepts a whole URL, not just the hash', () => {
    expect(
      readTokensFromFragment('https://app.example.com/cb?x=1#access_token=at'),
    ).toEqual({ accessToken: 'at', refreshToken: null });
  });

  // A plain visit to the callback route must be distinguishable from an
  // arrival from a link, or the app shows a sign-in failure to someone who
  // simply navigated there.
  it('returns null when there is nothing to read', () => {
    expect(readTokensFromFragment('')).toBeNull();
    expect(readTokensFromFragment('#')).toBeNull();
    expect(readTokensFromFragment('#state=abc')).toBeNull();
  });
});
