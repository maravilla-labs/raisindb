import { afterEach, describe, expect, it, vi } from 'vitest';
import { EventEmitter } from 'events';
import { encode, decode } from '@msgpack/msgpack';
import { RaisinClient } from './client';
import { Connection, ConnectionState, type ConnectionOptions } from './connection';
import { MemoryTokenStorage } from './auth';
import { RaisinAuthError, RaisinTimeoutError } from './errors';

// ---------------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------------

class FakeConnection extends EventEmitter {
  state: ConnectionState = ConnectionState.Disconnected;
  sent: Uint8Array[] = [];
  autoReconnect = true;

  getState(): ConnectionState {
    return this.state;
  }

  isConnected(): boolean {
    return this.state === ConnectionState.Connected;
  }

  isAutoReconnectEnabled(): boolean {
    return this.autoReconnect;
  }

  send(data: Uint8Array): void {
    if (this.state !== ConnectionState.Connected) {
      throw new Error('Cannot send data: not connected');
    }
    this.sent.push(data);
  }

  async connect(): Promise<void> {
    this.setState(ConnectionState.Connected);
  }

  disconnect(): void {
    this.setState(ConnectionState.Closed);
  }

  setState(state: ConnectionState): void {
    if (this.state !== state) {
      this.state = state;
      this.emit('stateChange', state);
    }
  }

  /** Simulate a binary message from the server */
  receive(message: unknown): void {
    const encoded = encode(message);
    const buffer = encoded.buffer.slice(
      encoded.byteOffset,
      encoded.byteOffset + encoded.byteLength,
    );
    this.emit('message', buffer);
  }

  /** Decode the n-th sent request envelope */
  sentRequest(index: number): { request_id: string; type: string; payload: unknown } {
    return decode(this.sent[index]) as { request_id: string; type: string; payload: unknown };
  }
}

class TestClient extends RaisinClient {
  protected createConnection(_url: string, _options?: ConnectionOptions): Connection {
    return new FakeConnection() as unknown as Connection;
  }
}

function makeClient(options: ConstructorParameters<typeof RaisinClient>[1] = {}): {
  client: TestClient;
  conn: FakeConnection;
} {
  const client = new TestClient('raisin://localhost:9999/sys/default/demo', options);
  const conn = (client as unknown as { connection: FakeConnection }).connection;
  return { client, conn };
}

function sendInternal(client: TestClient, payload: unknown, type = 'sql_query'): Promise<unknown> {
  return (
    client as unknown as {
      sendRequestInternal: (p: unknown, t: string) => Promise<unknown>;
    }
  ).sendRequestInternal(payload, type);
}

/** Build an unsigned JWT-shaped token with the given payload */
function fakeJwt(payload: Record<string, unknown>): string {
  const b64 = (obj: unknown) =>
    Buffer.from(JSON.stringify(obj)).toString('base64url');
  return `${b64({ alg: 'none' })}.${b64(payload)}.sig`;
}

/** Simulate a connect → reconnect cycle so queueing kicks in */
function simulateReconnecting(conn: FakeConnection): void {
  conn.setState(ConnectionState.Connected);
  conn.setState(ConnectionState.Reconnecting);
}

// ---------------------------------------------------------------------------
// Request queueing during reconnect (item 2)
// ---------------------------------------------------------------------------

describe('RaisinClient request queueing during reconnect', () => {
  it('queues a request while reconnecting and delivers it after connected + anonymous ready', async () => {
    const { client: testClient, conn: testConn } = makeClient();

    simulateReconnecting(testConn);

    const promise = sendInternal(testClient, { query: 'SELECT 1' });
    // Nothing sent yet — the request is held
    expect(testConn.sent).toHaveLength(0);

    // Connection comes back; server sends connected message (anonymous, no stored token)
    testConn.setState(ConnectionState.Connected);
    testConn.receive({
      type: 'connected',
      connection_id: 'conn-1',
      anonymous: true,
      user_id: 'anon-1',
    });

    await vi.waitFor(() => expect(testConn.sent).toHaveLength(1));
    const request = testConn.sentRequest(0);
    expect(request.type).toBe('sql_query');

    // Server responds; the original promise resolves
    testConn.receive({
      request_id: request.request_id,
      status: 'success',
      result: { rows: [] },
    });
    await expect(promise).resolves.toEqual({ rows: [] });
  });

  it('delivers queued requests only after re-authentication succeeds', async () => {
    const storage = new MemoryTokenStorage();
    const token = fakeJwt({
      sub: 'u1',
      exp: Math.floor(Date.now() / 1000) + 3600,
    });
    storage.setAccessToken(token);

    const { client, conn } = makeClient({ tokenStorage: storage });
    simulateReconnecting(conn);

    const promise = sendInternal(client, { query: 'SELECT 1' });
    expect(conn.sent).toHaveLength(0);

    conn.setState(ConnectionState.Connected);
    conn.receive({
      type: 'connected',
      connection_id: 'conn-1',
      anonymous: true,
      user_id: 'anon-1',
    });

    // First the auth request goes out — the queued request must still be held
    await vi.waitFor(() => expect(conn.sent).toHaveLength(1));
    const authRequest = conn.sentRequest(0);
    expect(authRequest.type).toBe('authenticate_jwt');
    expect(conn.sent).toHaveLength(1);

    // Auth succeeds → queued request is flushed
    conn.receive({
      request_id: authRequest.request_id,
      status: 'success',
      result: { user_id: 'u1', roles: [] },
    });
    await vi.waitFor(() => expect(conn.sent).toHaveLength(2));
    const queuedRequest = conn.sentRequest(1);
    expect(queuedRequest.type).toBe('sql_query');

    conn.receive({
      request_id: queuedRequest.request_id,
      status: 'success',
      result: { ok: true },
    });
    await expect(promise).resolves.toEqual({ ok: true });
  });

  it('rejects queued requests when re-authentication fails', async () => {
    const storage = new MemoryTokenStorage();
    storage.setAccessToken(
      fakeJwt({ sub: 'u1', exp: Math.floor(Date.now() / 1000) + 3600 }),
    );
    // No refresh token → refresh fallback fails

    const { client, conn } = makeClient({ tokenStorage: storage });
    simulateReconnecting(conn);

    const promise = sendInternal(client, { query: 'SELECT 1' });
    const assertion = expect(promise).rejects.toBeInstanceOf(RaisinAuthError);

    conn.setState(ConnectionState.Connected);
    conn.receive({
      type: 'connected',
      connection_id: 'conn-1',
      anonymous: true,
      user_id: 'anon-1',
    });

    await vi.waitFor(() => expect(conn.sent).toHaveLength(1));
    const authRequest = conn.sentRequest(0);

    // Auth fails server-side
    conn.receive({
      request_id: authRequest.request_id,
      status: 'error',
      error: { code: 'AUTH_FAILED', message: 'invalid token' },
    });

    await assertion;
    // The queued request was never sent
    expect(conn.sent).toHaveLength(1);
  });

  it('rejects immediately when the queue cap is exceeded', async () => {
    const { client, conn } = makeClient();
    simulateReconnecting(conn);

    const queued: Promise<unknown>[] = [];
    for (let i = 0; i < 100; i++) {
      const p = sendInternal(client, { i });
      p.catch(() => undefined); // silenced — cleaned up at the end
      queued.push(p);
    }

    await expect(sendInternal(client, { overflow: true })).rejects.toThrow(
      /queue is full/i,
    );

    // Cleanup: permanent disconnect cancels everything queued
    conn.setState(ConnectionState.Disconnected);
    await Promise.allSettled(queued);
  });

  it('still throws immediately when never connected and not reconnecting', async () => {
    const { client } = makeClient();
    await expect(sendInternal(client, {})).rejects.toThrow('Not connected to server');
  });
});

// ---------------------------------------------------------------------------
// Subscription restore retry (item 4)
// ---------------------------------------------------------------------------

describe('RaisinClient subscription restore retry', () => {
  function withFakeRestore(client: TestClient, restore: ReturnType<typeof vi.fn>) {
    const anyClient = client as unknown as {
      eventHandler: { restoreSubscriptions: unknown };
      _restoreRetryDelaysMs: number[];
      _restoreSubscriptionsAndNotify: () => Promise<void>;
    };
    anyClient.eventHandler.restoreSubscriptions = restore;
    anyClient._restoreRetryDelaysMs = [1, 1, 1];
    return anyClient;
  }

  it('retries failed restores and emits reconnected on eventual success', async () => {
    const { client } = makeClient();
    const restore = vi
      .fn()
      .mockRejectedValueOnce(new Error('boom 1'))
      .mockRejectedValueOnce(new Error('boom 2'))
      .mockResolvedValueOnce(undefined);
    const anyClient = withFakeRestore(client, restore);

    const reconnected = vi.fn();
    const restoreFailed = vi.fn();
    client.onReconnected(reconnected);
    client.on('subscription_restore_failed', restoreFailed);

    await anyClient._restoreSubscriptionsAndNotify();

    expect(restore).toHaveBeenCalledTimes(3);
    // First call is a fresh restore, retries are flagged
    expect(restore.mock.calls[0][0]).toEqual({ retry: false });
    expect(restore.mock.calls[1][0]).toEqual({ retry: true });
    expect(reconnected).toHaveBeenCalledTimes(1);
    expect(restoreFailed).not.toHaveBeenCalled();
  });

  it('emits subscription_restore_failed after exhausting retries', async () => {
    const { client } = makeClient();
    const restore = vi.fn().mockRejectedValue(new Error('still broken'));
    const anyClient = withFakeRestore(client, restore);

    const reconnected = vi.fn();
    const restoreFailed = vi.fn();
    client.onReconnected(reconnected);
    client.on('subscription_restore_failed', restoreFailed);

    await anyClient._restoreSubscriptionsAndNotify();

    expect(restore).toHaveBeenCalledTimes(4); // initial + 3 retries
    expect(reconnected).not.toHaveBeenCalled();
    expect(restoreFailed).toHaveBeenCalledTimes(1);
    expect(restoreFailed.mock.calls[0][0]).toBeInstanceOf(Error);
    expect((restoreFailed.mock.calls[0][0] as Error).message).toContain('still broken');
  });
});

// ---------------------------------------------------------------------------
// Auth fetch timeout + error normalization (item 3)
// ---------------------------------------------------------------------------

describe('RaisinClient auth fetch hardening', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('throws a real RaisinAuthError (with code/message) on login failure', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        new Response(
          JSON.stringify({ code: 'INVALID_CREDENTIALS', message: 'Bad password' }),
          { status: 401, headers: { 'content-type': 'application/json' } },
        ),
      ),
    );

    const { client } = makeClient();
    const promise = client.loginWithEmail('a@b.c', 'wrong', 'demo');

    await expect(promise).rejects.toBeInstanceOf(RaisinAuthError);
    await expect(promise).rejects.toMatchObject({
      code: 'INVALID_CREDENTIALS',
      message: 'Bad password',
      status: 401,
    });
  });

  it('aborts a hanging login fetch with a timeout error', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(
        (_url: string, init?: RequestInit) =>
          new Promise<Response>((_resolve, reject) => {
            init?.signal?.addEventListener('abort', () => {
              reject(Object.assign(new Error('aborted'), { name: 'AbortError' }));
            });
          }),
      ),
    );

    const { client } = makeClient({ requestTimeout: 30 });
    const promise = client.loginWithEmail('a@b.c', 'pw', 'demo');

    await expect(promise).rejects.toBeInstanceOf(RaisinTimeoutError);
    await expect(promise).rejects.toMatchObject({ code: 'REQUEST_TIMEOUT' });
  });

  it('aborts a hanging refresh fetch with a timeout error (returns null)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(
        (_url: string, init?: RequestInit) =>
          new Promise<Response>((_resolve, reject) => {
            init?.signal?.addEventListener('abort', () => {
              reject(Object.assign(new Error('aborted'), { name: 'AbortError' }));
            });
          }),
      ),
    );

    const storage = new MemoryTokenStorage();
    storage.setRefreshToken('refresh-token');
    const { client } = makeClient({ tokenStorage: storage, requestTimeout: 30 });

    // refreshToken catches errors and returns null instead of hanging
    await expect(client.refreshToken()).resolves.toBeNull();
  });

  it('single-flights concurrent refreshes onto one request', async () => {
    // Refresh tokens are single-use: the server rotates them and revokes the
    // session if it sees a stale generation. Concurrent callers (initSession,
    // autoReauthenticate, the refresh timer, 401 retries) must therefore share
    // one in-flight request rather than each POSTing the stored token.
    let resolveFetch: (r: Response) => void = () => {};
    const fetchMock = vi.fn(
      () =>
        new Promise<Response>((resolve) => {
          resolveFetch = resolve;
        }),
    );
    vi.stubGlobal('fetch', fetchMock);

    const storage = new MemoryTokenStorage();
    storage.setRefreshToken('refresh-token-generation-1');
    const { client } = makeClient({ tokenStorage: storage });

    const all = Promise.all([
      client.refreshToken(),
      client.refreshToken(),
      client.refreshToken(),
    ]);

    expect(fetchMock).toHaveBeenCalledTimes(1);

    resolveFetch(
      new Response(
        JSON.stringify({
          access_token: fakeJwt({ sub: 'u1', exp: Math.floor(Date.now() / 1000) + 3600 }),
          refresh_token: 'refresh-token-generation-2',
          expires_at: new Date(Date.now() + 3600_000).toISOString(),
          identity: {
            id: 'u1',
            email: 'a@b.c',
            display_name: 'A',
            avatar_url: null,
            email_verified: true,
          },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      ),
    );

    const results = await all;
    // One request, one rotation — every caller gets the same result.
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(results.every((r) => r?.id === 'u1')).toBe(true);
    expect(storage.getRefreshToken()).toBe('refresh-token-generation-2');

    // The guard clears afterwards, so a later refresh issues a fresh request.
    void client.refreshToken();
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });
});

// ---------------------------------------------------------------------------
// URL parsing: tenant-less /ws routes, /sys operator routes, repository option
// ---------------------------------------------------------------------------

/**
 * Captures the URL and options the client passes to the connection.
 * Module-level because subclass field initializers would run after the
 * base constructor (which calls createConnection) and clobber the values.
 */
let capturedConnection: { url: string; options?: ConnectionOptions } = { url: '' };

class UrlCapturingClient extends RaisinClient {
  protected createConnection(url: string, options?: ConnectionOptions): Connection {
    capturedConnection = { url, options };
    return new FakeConnection() as unknown as Connection;
  }
}

function internals(client: RaisinClient): { tenantId: string; repository: string } {
  const c = client as unknown as {
    options: { tenantId: string };
    _repository: string;
  };
  return { tenantId: c.options.tenantId, repository: c._repository };
}

describe('RaisinClient URL parsing', () => {
  it('parses tenant-less ws://host/ws/{repo} URLs (tenant defaults to "default")', () => {
    const client = new UrlCapturingClient('ws://localhost:8081/ws/shiftboard');
    expect(internals(client)).toEqual({ tenantId: 'default', repository: 'shiftboard' });
  });

  it('parses bare tenant-less ws://host/ws URLs (no repository)', () => {
    const client = new UrlCapturingClient('ws://localhost:8081/ws');
    expect(internals(client)).toEqual({ tenantId: 'default', repository: '' });
  });

  it('parses raisin://host/ws/{repo} URLs', () => {
    const client = new UrlCapturingClient('raisin://localhost:8081/ws/myrepo');
    expect(internals(client)).toEqual({ tenantId: 'default', repository: 'myrepo' });
  });

  it('keeps the explicit tenantId option on tenant-less URLs', () => {
    const client = new UrlCapturingClient('ws://localhost:8081/ws/shiftboard', {
      tenantId: 'acme',
    });
    expect(internals(client)).toEqual({ tenantId: 'acme', repository: 'shiftboard' });
  });

  it('parses operator /sys/{tenant}/{repo} URLs (back-compat)', () => {
    const client = new UrlCapturingClient('ws://localhost:8081/sys/acme/myrepo');
    expect(internals(client)).toEqual({ tenantId: 'acme', repository: 'myrepo' });
  });

  it('parses operator /sys/{tenant} URLs without a repository', () => {
    const client = new UrlCapturingClient('ws://localhost:8081/sys/acme');
    expect(internals(client)).toEqual({ tenantId: 'acme', repository: '' });
  });

  it('builds /ws/{repo} URL internally from the repository option', () => {
    const client = new UrlCapturingClient('ws://localhost:8081', { repository: 'myrepo' });
    expect(capturedConnection.url).toBe('ws://localhost:8081/ws/myrepo');
    expect(internals(client)).toEqual({ tenantId: 'default', repository: 'myrepo' });
  });

  it('repository option overrides URL extraction but keeps an explicit path URL', () => {
    const client = new UrlCapturingClient('ws://localhost:8081/sys/acme/other', {
      repository: 'myrepo',
    });
    expect(capturedConnection.url).toBe('ws://localhost:8081/sys/acme/other');
    expect(internals(client)).toEqual({ tenantId: 'acme', repository: 'myrepo' });
  });

  it('sends the resolved tenant as an x-tenant-id upgrade header', () => {
    new UrlCapturingClient('ws://localhost:8081/ws/shiftboard', {
      tenantId: 'acme',
    });
    expect(capturedConnection.options?.headers).toMatchObject({ 'x-tenant-id': 'acme' });
  });

  it('omits the x-tenant-id upgrade header when no tenant was specified', () => {
    // Header ABSENCE is the protocol signal for "use the server's default
    // resolution" - the client must not assert 'default' on its own (it
    // would fight proxies that inject the real tenant header).
    new UrlCapturingClient('ws://localhost:8081/ws/shiftboard');
    expect(capturedConnection.options?.headers?.['x-tenant-id']).toBeUndefined();
  });

  it('sends the x-tenant-id upgrade header when the URL carries a tenant', () => {
    new UrlCapturingClient('ws://localhost:8081/sys/acme/shiftboard');
    expect(capturedConnection.options?.headers).toMatchObject({ 'x-tenant-id': 'acme' });
  });
});
