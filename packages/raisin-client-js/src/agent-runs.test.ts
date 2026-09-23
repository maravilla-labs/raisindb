import { describe, expect, it } from 'vitest';
import { AgentRunsApi, type AgentRunsTransport } from './agent-runs';

function transport(calls: Array<{ method: string; path: string; body?: unknown }>, sse = ''): AgentRunsTransport {
  return {
    request: async <T>(method: string, path: string, body?: unknown) => {
      calls.push({ method, path, body });
      return { ok: true } as T;
    },
    url: (path) => `http://x${path}`,
    headers: () => ({ Authorization: 'Bearer t' }),
    fetch: (async () =>
      new Response(
        new ReadableStream({
          start(c) {
            c.enqueue(new TextEncoder().encode(sse));
            c.close();
          },
        }),
        { status: 200 },
      )) as unknown as typeof fetch,
  };
}

describe('AgentRunsApi', () => {
  it('maps create options onto the wire contract', async () => {
    const calls: Array<{ method: string; path: string; body?: unknown }> = [];
    const api = new AgentRunsApi('studio', transport(calls));
    await api.create({
      subject: { workspace: 'ws', path: '/chat/1' },
      reducer: { function_path: '/lib/r' },
      asAgent: '/agents/builder',
      input: { text: 'hi' },
    });
    expect(calls[0].method).toBe('POST');
    expect(calls[0].path).toBe('/api/agent-runs/studio');
    expect(calls[0].body).toMatchObject({
      subject: { workspace: 'ws', path: '/chat/1' },
      reducer: { function_path: '/lib/r' },
      as_agent: '/agents/builder',
      input: { text: 'hi' },
    });
  });

  it('sends controls with a stable control id', async () => {
    const calls: Array<{ method: string; path: string; body?: unknown }> = [];
    const api = new AgentRunsApi('studio', transport(calls));
    await api.stop('run-1', 'enough', 'stop-1');
    await api.answer('run-1', 'run-1/req/1', 'yes', 'ans-1');
    expect(calls[0]).toMatchObject({
      path: '/api/agent-runs/studio/run-1/control',
      body: { control_id: 'stop-1', command: { command: 'stop', reason: 'enough' } },
    });
    expect(calls[1].body).toMatchObject({
      command: { command: 'provide_input', request_id: 'run-1/req/1', value: 'yes' },
    });
  });

  it('streams run events until the end marker', async () => {
    const ev = (seq: number) =>
      `id: ${seq}\nevent: run-event\ndata: ${JSON.stringify({ run_id: 'r', seq, at_ms: 1, kind: { type: 'x' } })}\n\n`;
    const api = new AgentRunsApi('studio', transport([], `${ev(1)}${ev(2)}event: end\ndata: {}\n\n${ev(3)}`));
    const seen: number[] = [];
    for await (const e of api.stream('r')) seen.push(e.seq);
    expect(seen).toEqual([1, 2]);
  });
  it('lists the runs of a subject with ?subject=', async () => {
    const calls: Array<{ method: string; path: string; body?: unknown }> = [];
    await new AgentRunsApi('studio', transport(calls)).bySubject({ workspace: 'ai', path: '/c/1', node_id: 'n1' }, 5);
    expect(calls[0].path).toBe('/api/agent-runs/studio?subject=ai%3A%2Fc%2F1&limit=5&subject_node_id=n1');
  });
});
