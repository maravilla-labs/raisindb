import { describe, expect, it, vi } from 'vitest';
import { FlowClient } from './flow-client';
import type { AuthManager } from './auth';

const fakeAuth = {
  getAccessToken: () => 'test-token',
} as unknown as AuthManager;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

/** Build an SSE response streaming the given events as `flow-event` frames. */
function sseResponse(events: unknown[]): Response {
  const encoder = new TextEncoder();
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const event of events) {
        controller.enqueue(
          encoder.encode(`event: flow-event\ndata: ${JSON.stringify(event)}\n\n`),
        );
      }
      controller.close();
    },
  });
  return new Response(stream, {
    status: 200,
    headers: { 'content-type': 'text/event-stream' },
  });
}

describe('FlowClient.run', () => {
  it('posts flow_path and input to the run endpoint', async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ instance_id: 'inst-1', job_id: 'job-1', status: 'queued' }),
    );
    const flows = new FlowClient('http://localhost:8081', 'demo', fakeAuth, {
      fetch: fetchMock as unknown as typeof fetch,
    });

    const result = await flows.run('/flows/my-flow', { orderId: '123' });

    expect(result.instance_id).toBe('inst-1');
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('http://localhost:8081/api/flows/demo/run');
    expect(init.method).toBe('POST');
    expect(JSON.parse(init.body as string)).toEqual({
      flow_path: '/flows/my-flow',
      input: { orderId: '123' },
    });
    expect((init.headers as Record<string, string>)['Authorization']).toBe(
      'Bearer test-token',
    );
  });
});

describe('FlowClient.resume', () => {
  it('posts resume_data to the resume endpoint', async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ status: 'resumed' }));
    const flows = new FlowClient('http://localhost:8081', 'demo', fakeAuth, {
      fetch: fetchMock as unknown as typeof fetch,
    });

    await flows.resume('inst-1', { tool_result: 42 });

    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe(
      'http://localhost:8081/api/flows/demo/instances/inst-1/resume',
    );
    expect(JSON.parse(init.body as string)).toEqual({
      resume_data: { tool_result: 42 },
    });
  });
});

describe('FlowClient.respondToHumanTask', () => {
  it('completes the task via the inbox endpoint', async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ status: 'completed' }));
    const flows = new FlowClient('http://localhost:8081', 'demo', fakeAuth, {
      fetch: fetchMock as unknown as typeof fetch,
    });

    await flows.respondToHumanTask('inst-1', 'task-abc', { action: 'approve' });

    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe(
      'http://localhost:8081/api/inbox/demo/tasks/task-abc/complete',
    );
    expect(JSON.parse(init.body as string)).toEqual({
      response: { action: 'approve' },
    });
  });
});

describe('FlowClient.streamEvents', () => {
  it('yields events and stops at the terminal event', async () => {
    const fetchMock = vi.fn(async () =>
      sseResponse([
        { type: 'step_started', node_id: 'start', step_type: 'start', timestamp: 't' },
        {
          type: 'step_completed',
          node_id: 'start',
          output: null,
          duration_ms: 1,
          timestamp: 't',
        },
        { type: 'flow_completed', output: { ok: true }, total_duration_ms: 5, timestamp: 't' },
        // Should never be yielded - stream stops at the terminal event
        { type: 'log', level: 'info', message: 'after terminal', timestamp: 't' },
      ]),
    );
    const flows = new FlowClient('http://localhost:8081', 'demo', fakeAuth, {
      fetch: fetchMock as unknown as typeof fetch,
    });

    const seen: string[] = [];
    for await (const event of flows.streamEvents('inst-1')) {
      seen.push(event.type);
    }

    expect(seen).toEqual(['step_started', 'step_completed', 'flow_completed']);
    const [url] = fetchMock.mock.calls[0] as unknown as [string];
    expect(url).toBe(
      'http://localhost:8081/api/flows/demo/instances/inst-1/events',
    );
  });
});

describe('FlowClient.runAndWait', () => {
  it('returns the completed output', async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url.endsWith('/run')) {
        return jsonResponse({ instance_id: 'inst-9', job_id: 'j', status: 'queued' });
      }
      return sseResponse([
        { type: 'flow_completed', output: { answer: 42 }, total_duration_ms: 3, timestamp: 't' },
      ]);
    });
    const flows = new FlowClient('http://localhost:8081', 'demo', fakeAuth, {
      fetch: fetchMock as unknown as typeof fetch,
    });

    const result = await flows.runAndWait('/flows/calc', {});

    expect(result.status).toBe('completed');
    expect(result.instanceId).toBe('inst-9');
    expect(result.output).toEqual({ answer: 42 });
  });

  it('returns the error when the flow fails', async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url.endsWith('/run')) {
        return jsonResponse({ instance_id: 'inst-9', job_id: 'j', status: 'queued' });
      }
      return sseResponse([
        { type: 'flow_failed', error: 'boom', total_duration_ms: 3, timestamp: 't' },
      ]);
    });
    const flows = new FlowClient('http://localhost:8081', 'demo', fakeAuth, {
      fetch: fetchMock as unknown as typeof fetch,
    });

    const result = await flows.runAndWait('/flows/calc', {});

    expect(result.status).toBe('failed');
    expect(result.error).toBe('boom');
  });
});
