import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import crypto from 'crypto';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { executeRun, type RunEvent } from './run.js';
import { resolveRunTarget } from './run-target.js';

let root: string;
/** Bytes of the fake artifact — a wasm preamble is enough; nothing parses it here. */
const ARTIFACT = Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00]);
const ARTIFACT_HASH = crypto.createHash('sha256').update(ARTIFACT).digest('hex');

function write(rel: string, content: string | Buffer): void {
  const full = path.join(root, rel);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, content);
}

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'raisin-exec-'));
  write('manifest.yaml', 'name: demo\nversion: 0.1.0\n');
  write(
    'content/functions/lib/demo/greet/.node.yaml',
    'node_type: raisin:Function\nproperties:\n  name: greet\n  language: wasm\n  entry_file: main.wasm\n'
  );
  write('content/functions/lib/demo/greet/main.wasm', ARTIFACT);
});

afterEach(() => {
  fs.rmSync(root, { recursive: true, force: true });
});

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

/** The SSE body `/api/files/{repo}/run` answers with. */
function sse(): Response {
  const body =
    'event: started\ndata: {"execution_id":"e1"}\n\n' +
    'event: log\ndata: {"level":"info","message":"hello from wasm"}\n\n' +
    'event: result\ndata: {"success":true,"result":{"greeting":"Hello, Ada!"},"duration_ms":2}\n\n' +
    'event: done\ndata: {"type":"done"}\n\n';
  return new Response(body, { status: 200 });
}

/** Route a request by method + path, recording the URLs seen. */
function router(routes: Record<string, (init?: RequestInit) => Response>): {
  fetchImpl: typeof fetch;
  seen: string[];
} {
  const seen: string[] = [];
  const fetchImpl = (async (input: Parameters<typeof fetch>[0], init?: RequestInit) => {
    const url = String(input);
    const key = `${(init?.method || 'GET').toUpperCase()} ${url.replace('http://srv', '')}`;
    seen.push(key);
    const handler = routes[key];
    if (!handler) return new Response('', { status: 404 });
    return handler(init);
  }) as unknown as typeof fetch;
  return { fetchImpl, seen };
}

const ARTIFACT_URL = '/api/repository/demo/main/head/functions/lib/demo/greet/main.wasm';

describe('executeRun', () => {
  it('uploads and streams when the function is not deployed', async () => {
    let uploaded = false;
    const { fetchImpl, seen } = router({
      [`GET ${ARTIFACT_URL}`]: () =>
        uploaded ? json({ id: 'asset-1', properties: {} }) : new Response('', { status: 404 }),
      [`POST ${ARTIFACT_URL}?override_existing=true`]: () => {
        uploaded = true;
        return json({ storedKey: 'k' });
      },
      'POST /api/files/demo/run': () => sse(),
    });

    const events: RunEvent[] = [];
    const { outcome, plan } = await executeRun(
      resolveRunTarget(path.join(root, 'content/functions/lib/demo/greet')),
      { input: { name: 'Ada' }, server: 'http://srv', repo: 'demo', fetchImpl },
      (event) => events.push(event)
    );

    expect(plan.mode).toBe('run-file');
    expect(outcome.success).toBe(true);
    expect(outcome.result).toEqual({ greeting: 'Hello, Ada!' });
    expect(seen).toContain(`POST ${ARTIFACT_URL}?override_existing=true`);
    expect(events.filter((e) => e.kind === 'log')).toEqual([
      { kind: 'log', level: 'info', message: 'hello from wasm' },
    ]);
  });

  it('invokes the deployed function when the server holds the same bytes', async () => {
    const { fetchImpl, seen } = router({
      'GET /api/functions/demo/greet': () => json({ name: 'greet', language: 'wasm' }),
      [`GET ${ARTIFACT_URL}`]: () =>
        json({ id: 'asset-1', properties: { content_hash: ARTIFACT_HASH } }),
      'POST /api/functions/demo/greet/invoke': () =>
        json({ execution_id: 'e', sync: true, result: { greeting: 'Hi' }, duration_ms: 1 }),
    });

    const { outcome, plan } = await executeRun(resolveRunTarget(root), {
      input: {},
      server: 'http://srv',
      repo: 'demo',
      fetchImpl,
    });

    expect(plan.mode).toBe('invoke');
    expect(outcome.result).toEqual({ greeting: 'Hi' });
    expect(seen).not.toContain(`POST ${ARTIFACT_URL}?override_existing=true`);
  });

  it('re-uploads when the deployed bytes differ', async () => {
    const { fetchImpl, seen } = router({
      'GET /api/functions/demo/greet': () => json({ name: 'greet' }),
      [`GET ${ARTIFACT_URL}`]: () => json({ id: 'asset-1', properties: { content_hash: 'stale' } }),
      [`POST ${ARTIFACT_URL}?override_existing=true`]: () => json({ storedKey: 'k' }),
      'POST /api/files/demo/run': () => sse(),
    });

    const { plan } = await executeRun(resolveRunTarget(root), {
      input: {},
      server: 'http://srv',
      repo: 'demo',
      fetchImpl,
    });
    expect(plan.mode).toBe('run-file');
    expect(seen).toContain(`POST ${ARTIFACT_URL}?override_existing=true`);
  });

  it('says to build first when the artifact is missing', async () => {
    fs.rmSync(path.join(root, 'content/functions/lib/demo/greet/main.wasm'));
    const { fetchImpl } = router({});
    await expect(
      executeRun(resolveRunTarget(root), {
        input: {},
        server: 'http://srv',
        repo: 'demo',
        fetchImpl,
      })
    ).rejects.toThrow(/raisindb function build/);
  });

  it('fails loudly when the upload leaves no node to run', async () => {
    const { fetchImpl } = router({
      [`POST ${ARTIFACT_URL}?override_existing=true`]: () => json({ storedKey: 'k' }),
    });
    await expect(
      executeRun(resolveRunTarget(root), {
        input: {},
        server: 'http://srv',
        repo: 'demo',
        fetchImpl,
      })
    ).rejects.toThrow(/no node id/);
  });
});
