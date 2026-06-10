import { describe, expect, it, vi } from 'vitest';
import { SSEClient, type SSEEvent } from './sse-client';
import { RaisinTimeoutError } from '../errors';

const encoder = new TextEncoder();

function sseFrame(event: string, data: unknown): Uint8Array {
  return encoder.encode(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
}

/** SSE response that emits the given frames and then stalls forever. */
function stallingResponse(frames: Uint8Array[]): Response {
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const frame of frames) controller.enqueue(frame);
      // Never close — simulates a dead stream
    },
  });
  return new Response(stream, {
    status: 200,
    headers: { 'content-type': 'text/event-stream' },
  });
}

/** SSE response that emits the given frames and closes cleanly. */
function closingResponse(frames: Uint8Array[]): Response {
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const frame of frames) controller.enqueue(frame);
      controller.close();
    },
  });
  return new Response(stream, {
    status: 200,
    headers: { 'content-type': 'text/event-stream' },
  });
}

describe('SSEClient inactivity timeout', () => {
  it('ends the iterator with a timeout error when the stream stalls (reconnect disabled)', async () => {
    const fetchMock = vi.fn(async () =>
      stallingResponse([sseFrame('message', { n: 1 })]),
    );
    const sse = new SSEClient(
      'http://localhost:9999/events',
      {
        fetch: fetchMock as unknown as typeof fetch,
        reconnect: { enabled: false },
        inactivityTimeoutMs: 50,
      },
    );

    const seen: SSEEvent[] = [];
    let caught: unknown;
    try {
      for await (const event of sse) {
        seen.push(event);
      }
    } catch (error) {
      caught = error;
    }

    // The event before the stall is delivered, then the iterator errors
    expect(seen).toHaveLength(1);
    expect(seen[0].data).toEqual({ n: 1 });
    expect(caught).toBeInstanceOf(RaisinTimeoutError);
    expect((caught as RaisinTimeoutError).code).toBe('SSE_INACTIVITY_TIMEOUT');
  });

  it('ends with a timeout error when the stream never produces anything', async () => {
    const fetchMock = vi.fn(async () => stallingResponse([]));
    const sse = new SSEClient('http://localhost:9999/events', {
      fetch: fetchMock as unknown as typeof fetch,
      reconnect: { enabled: false },
      inactivityTimeoutMs: 40,
    });

    let caught: unknown;
    const start = Date.now();
    try {
      for await (const _event of sse) {
        // no events expected
      }
    } catch (error) {
      caught = error;
    }

    expect(caught).toBeInstanceOf(RaisinTimeoutError);
    // Should have ended around the timeout instead of hanging forever
    expect(Date.now() - start).toBeLessThan(2000);
  });

  it('ends with a timeout error when the fetch itself never resolves', async () => {
    const fetchMock = vi.fn(
      () => new Promise<Response>(() => undefined), // never resolves
    );
    const sse = new SSEClient('http://localhost:9999/events', {
      fetch: fetchMock as unknown as typeof fetch,
      reconnect: { enabled: false },
      inactivityTimeoutMs: 40,
    });

    let caught: unknown;
    try {
      for await (const _event of sse) {
        // no events expected
      }
    } catch (error) {
      caught = error;
    }

    expect(caught).toBeInstanceOf(RaisinTimeoutError);
    expect((caught as RaisinTimeoutError).code).toBe('SSE_INACTIVITY_TIMEOUT');
  });

  it('resets the timer on activity so slow-but-alive streams survive', async () => {
    // Emit 3 events spaced 25ms apart with a 70ms inactivity timeout,
    // then close. All events must be delivered without a timeout error.
    const stream = new ReadableStream<Uint8Array>({
      async start(controller) {
        for (let i = 0; i < 3; i++) {
          controller.enqueue(sseFrame('message', { n: i }));
          await new Promise((r) => setTimeout(r, 25));
        }
        controller.close();
      },
    });
    const fetchMock = vi.fn(async () =>
      new Response(stream, { status: 200, headers: { 'content-type': 'text/event-stream' } }),
    );
    const sse = new SSEClient('http://localhost:9999/events', {
      fetch: fetchMock as unknown as typeof fetch,
      reconnect: { enabled: false },
      inactivityTimeoutMs: 70,
    });

    const seen: unknown[] = [];
    for await (const event of sse) {
      seen.push(event.data);
    }

    expect(seen).toEqual([{ n: 0 }, { n: 1 }, { n: 2 }]);
  });

  it('reconnects after an inactivity timeout when reconnection is enabled', async () => {
    let call = 0;
    const fetchMock = vi.fn(async () => {
      call++;
      if (call === 1) {
        return stallingResponse([]); // first connection stalls
      }
      return closingResponse([sseFrame('message', { recovered: true })]);
    });

    const sse = new SSEClient('http://localhost:9999/events', {
      fetch: fetchMock as unknown as typeof fetch,
      reconnect: { enabled: true, initialDelay: 1, maxDelay: 5 },
      inactivityTimeoutMs: 40,
    });

    const seen: unknown[] = [];
    for await (const event of sse) {
      seen.push(event.data);
      break; // got the post-reconnect event, stop consuming
    }

    expect(seen).toEqual([{ recovered: true }]);
    expect(fetchMock.mock.calls.length).toBeGreaterThanOrEqual(2);
  });
});
