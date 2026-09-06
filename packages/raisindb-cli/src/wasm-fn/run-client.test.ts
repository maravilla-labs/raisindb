import { describe, it, expect } from 'vitest';
import {
  getArtifactNode,
  getFunctionDetails,
  invokeFunction,
  parseSseFrames,
  streamRunFile,
  uploadArtifact,
  type ServerContext,
} from './run-client.js';

/** One recorded request, so a test can assert on the wire, not the wrapper. */
interface Call {
  url: string;
  init: RequestInit | undefined;
}

/** A `fetch` stub answering a queue of responses and recording every call. */
function stubFetch(responses: Response[]): { fetchImpl: typeof fetch; calls: Call[] } {
  const calls: Call[] = [];
  const queue = [...responses];
  const fetchImpl = (async (input: Parameters<typeof fetch>[0], init?: RequestInit) => {
    calls.push({ url: String(input), init });
    const next = queue.shift();
    if (!next) throw new Error(`unexpected request to ${String(input)}`);
    return next;
  }) as unknown as typeof fetch;
  return { fetchImpl, calls };
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function context(fetchImpl: typeof fetch): ServerContext {
  return { baseUrl: 'http://host:8081', repo: 'demo', branch: 'main', token: 'tok', fetchImpl };
}

describe('getFunctionDetails', () => {
  it('returns null for a function that is not deployed', async () => {
    const { fetchImpl } = stubFetch([new Response('', { status: 404 })]);
    expect(await getFunctionDetails(context(fetchImpl), 'greet')).toBeNull();
  });

  it('sends the bearer token', async () => {
    const { fetchImpl, calls } = stubFetch([json({ name: 'greet' })]);
    await getFunctionDetails(context(fetchImpl), 'greet');
    expect(calls[0].url).toBe('http://host:8081/api/functions/demo/greet');
    expect((calls[0].init?.headers as Record<string, string>).Authorization).toBe('Bearer tok');
  });

  it('surfaces the server message on failure', async () => {
    const { fetchImpl } = stubFetch([json({ message: 'boom' }, 500)]);
    await expect(getFunctionDetails(context(fetchImpl), 'greet')).rejects.toThrow('boom');
  });
});

describe('getArtifactNode', () => {
  it('reads the hash out of the file Resource metadata', async () => {
    const { fetchImpl } = stubFetch([
      json({ id: 'n1', properties: { file: { size: 12, metadata: { content_hash: 'aa' } } } }),
    ]);
    expect(await getArtifactNode(context(fetchImpl), 'functions', 'lib/demo/greet/main.wasm')).toEqual(
      { id: 'n1', contentHash: 'aa', size: 12 }
    );
  });

  it('falls back to the flat properties the package installer writes', async () => {
    const { fetchImpl } = stubFetch([
      json({ id: 'n1', properties: { content_hash: 'bb', file_size: 7 } }),
    ]);
    const node = await getArtifactNode(context(fetchImpl), 'functions', 'p');
    expect(node).toEqual({ id: 'n1', contentHash: 'bb', size: 7 });
  });

  it('reports a missing artifact as null, not an error', async () => {
    const { fetchImpl } = stubFetch([new Response('', { status: 404 })]);
    expect(await getArtifactNode(context(fetchImpl), 'functions', 'p')).toBeNull();
  });

  it('records no hash when the upload path wrote none', async () => {
    const { fetchImpl } = stubFetch([json({ id: 'n1', properties: { file: { size: 3 } } })]);
    expect((await getArtifactNode(context(fetchImpl), 'functions', 'p'))?.contentHash).toBeNull();
  });
});

describe('uploadArtifact', () => {
  it('posts multipart with override_existing so a dev loop can repeat', async () => {
    const { fetchImpl, calls } = stubFetch([json({ storedKey: 'k' })]);
    await uploadArtifact(
      context(fetchImpl),
      'functions',
      'lib/demo/greet/main.wasm',
      new Uint8Array([0, 97, 115, 109]),
      'main.wasm'
    );
    expect(calls[0].url).toBe(
      'http://host:8081/api/repository/demo/main/head/functions/lib/demo/greet/main.wasm?override_existing=true'
    );
    expect(calls[0].init?.body).toBeInstanceOf(FormData);
  });

  it('throws with the server message', async () => {
    const { fetchImpl } = stubFetch([json({ message: 'not a component' }, 400)]);
    await expect(
      uploadArtifact(context(fetchImpl), 'functions', 'p', new Uint8Array(), 'main.wasm')
    ).rejects.toThrow('not a component');
  });
});

describe('invokeFunction', () => {
  it('asks for a synchronous run and maps the response', async () => {
    const { fetchImpl, calls } = stubFetch([
      json({ execution_id: 'e', sync: true, result: { ok: 1 }, duration_ms: 4, logs: ['hi'] }),
    ]);
    const outcome = await invokeFunction(context(fetchImpl), 'greet', { name: 'Ada' }, 500);
    expect(JSON.parse(String(calls[0].init?.body))).toEqual({
      input: { name: 'Ada' },
      sync: true,
      timeout_ms: 500,
    });
    expect(outcome).toEqual({
      success: true,
      result: { ok: 1 },
      error: undefined,
      durationMs: 4,
      logs: ['hi'],
    });
  });

  it('treats an error field as a failed run, not a transport error', async () => {
    const { fetchImpl } = stubFetch([json({ execution_id: 'e', sync: true, error: 'nope' })]);
    const outcome = await invokeFunction(context(fetchImpl), 'greet', {});
    expect(outcome.success).toBe(false);
    expect(outcome.error).toBe('nope');
  });
});

describe('parseSseFrames', () => {
  it('returns whole frames and keeps the partial tail', () => {
    const { frames, rest } = parseSseFrames(
      'event: started\ndata: {"execution_id":"e"}\n\nevent: log\ndata: {"message":"hi"'
    );
    expect(frames).toEqual([{ event: 'started', data: { execution_id: 'e' } }]);
    expect(rest).toBe('event: log\ndata: {"message":"hi"');
  });

  it('falls back to the payload type when no event name is sent', () => {
    const { frames } = parseSseFrames('data: {"type":"done"}\n\n');
    expect(frames[0].event).toBe('done');
  });

  it('skips a non-JSON payload rather than failing the run', () => {
    expect(parseSseFrames(': keep-alive\n\n').frames).toEqual([]);
  });
});

/** A Response whose body streams the given chunks, as the SSE route does. */
function sseResponse(chunks: string[]): Response {
  const encoder = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
      controller.close();
    },
  });
  return new Response(body, { status: 200, headers: { 'Content-Type': 'text/event-stream' } });
}

describe('streamRunFile', () => {
  it('collects logs and the result across chunk boundaries', async () => {
    const { fetchImpl, calls } = stubFetch([
      sseResponse([
        'event: started\ndata: {"execution_id":"e"}\n\nevent: log\ndata: {"level":"info","mes',
        'sage":"hello"}\n\nevent: result\ndata: {"success":true,"result":{"greeting":"hi"},"duration_ms":3}\n\nevent: done\ndata: {"type":"done"}\n\n',
      ]),
    ]);
    const seen: string[] = [];
    const outcome = await streamRunFile(
      context(fetchImpl),
      { node_id: 'n1', handler: 'default', input: {} },
      (frame) => seen.push(frame.event)
    );
    expect(calls[0].url).toBe('http://host:8081/api/files/demo/run');
    expect(seen).toEqual(['started', 'log', 'result', 'done']);
    expect(outcome.success).toBe(true);
    expect(outcome.result).toEqual({ greeting: 'hi' });
    expect(outcome.logs).toEqual(['[info] hello']);
  });

  it('reports a stream that ends without a result as a failure', async () => {
    const { fetchImpl } = stubFetch([sseResponse(['event: done\ndata: {"type":"done"}\n\n'])]);
    const outcome = await streamRunFile(
      context(fetchImpl),
      { node_id: 'n1', handler: 'default', input: {} },
      () => {}
    );
    expect(outcome.success).toBe(false);
    expect(outcome.error).toMatch(/without a result/);
  });
});
