import { describe, expect, it, vi } from 'vitest';
import { ConversationManager } from './conversations';
import { ConversationStore } from './stores/conversation-store';
import type { AuthManager } from './auth';
import type { SqlResult } from './protocol';
import type { Database } from './database';
import type { ChatEvent } from './types/chat';
import type {
  ClientToolCallEvent,
  ClientToolResultBody,
  ClientToolsProvider,
} from './types/client-tools';

const fakeAuth = { getAccessToken: () => 'tok' } as unknown as AuthManager;
const encoder = new TextEncoder();
const sse = (event: string, data: unknown) =>
  encoder.encode(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);

function makeSql(inserts: Array<{ path: string; props: Record<string, any> }> = []) {
  return vi.fn(async (sql: string, params?: unknown[]): Promise<SqlResult> => {
    if (sql.includes('RAISIN_CURRENT_USER()')) {
      return { columns: ['home', 'user_id'], rows: [{ home: '/users/internal/alice', user_id: 'u1' }], row_count: 1 };
    }
    if (sql.includes('SELECT properties FROM')) {
      return {
        columns: ['properties'],
        rows: [{ properties: { stream_channel: 'chat:c1', conversation_id: 'c1', participants: ['u1', 'agent:support'] } }],
        row_count: 1,
      };
    }
    if (sql.includes('INSERT INTO')) {
      inserts.push({ path: params![0] as string, props: JSON.parse(params![1] as string) });
      return { columns: ['id'], rows: [{ id: 'm1' }], row_count: 1 };
    }
    return { columns: [], rows: [], row_count: 0 };
  });
}

const TOOL = { name: 'insert_blocks', description: 'd', inputSchema: { type: 'object' }, annotations: { readOnlyHint: false } };
const ATTACHMENT = {
  'raisin:ref': '/uploads/chat/p.jpg',
  'raisin:workspace': 'assets',
  mime_type: 'image/jpeg',
  name: 'p.jpg',
};

describe('createUserMessage metadata (contract §2)', () => {
  it('writes metadata.client and attachments with client_id equal to the message client_id', async () => {
    const inserts: Array<{ path: string; props: Record<string, any> }> = [];
    const manager = new ConversationManager('http://x', 'demo', fakeAuth, {}, makeSql(inserts));
    const id = await manager.createUserMessage('/users/internal/alice/inbox/chats/c1', 'hi', {
      client: { tools: [TOOL], context: { location: { path: '/a' } } },
      attachments: [ATTACHMENT],
    });
    const props = inserts[0].props;
    expect(props.client_id).toBe(id);
    expect(inserts[0].path).toBe(`/users/internal/alice/outbox/msg-${id}`);
    expect(props.metadata).toEqual({
      client: { client_id: id, tools: [TOOL], context: { location: { path: '/a' } } },
      attachments: [ATTACHMENT],
    });
  });

  it('omits metadata entirely for a plain message', async () => {
    const inserts: Array<{ path: string; props: Record<string, any> }> = [];
    const manager = new ConversationManager('http://x', 'demo', fakeAuth, {}, makeSql(inserts));
    await manager.createUserMessage('/users/internal/alice/inbox/chats/c1', 'hi');
    expect('metadata' in inserts[0].props).toBe(false);
  });

  it('passes metadata in resume_data on the flow-resume path', async () => {
    const sql = vi.fn(async (q: string): Promise<SqlResult> => ({
      columns: ['properties'],
      rows: [{ properties: { flow_instance_id: 'f1' } }],
      row_count: 1,
    }));
    const fetchMock = vi.fn(async () => new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } }));
    const manager = new ConversationManager('http://x', 'demo', fakeAuth, { fetch: fetchMock as unknown as typeof fetch }, sql);
    const id = await manager.createUserMessage('/c/f1', 'hi', { client: { tools: [TOOL] }, clientId: 'cid-1' });
    expect(id).toBe('cid-1');
    const body = JSON.parse((fetchMock.mock.calls[0] as any)[1].body);
    expect(body.resume_data).toEqual({
      message: 'hi',
      client_id: 'cid-1',
      metadata: { client: { client_id: 'cid-1', tools: [TOOL] } },
    });
  });

  it('sendMessage (SSE) attaches metadata and yields client_tool_call events', async () => {
    const inserts: Array<{ path: string; props: Record<string, any> }> = [];
    const call: ClientToolCallEvent = {
      type: 'client_tool_call', run_id: 'r1', call_id: 'c1', resume_key: 'client:c1',
      name: 'insert_blocks', args: {}, client_id: 'cid', expires_at_ms: Date.now() + 60000,
    };
    const fetchMock = vi.fn(async () =>
      new Response(
        new ReadableStream({
          start(c) {
            c.enqueue(sse('conversation-event', call));
            c.enqueue(sse('conversation-event', { type: 'waiting', timestamp: 't' }));
            c.close();
          },
        }),
        { status: 200, headers: { 'content-type': 'text/event-stream' } },
      ),
    );
    const manager = new ConversationManager('http://x', 'demo', fakeAuth, { fetch: fetchMock as unknown as typeof fetch }, makeSql(inserts));
    const seen: ChatEvent[] = [];
    for await (const e of manager.sendMessage('/users/internal/alice/inbox/chats/c1', 'hi', {
      client: { tools: [TOOL] },
      clientId: 'cid',
    })) seen.push(e);
    expect(seen.map((e) => e.type)).toEqual(['client_tool_call', 'waiting']);
    expect(inserts[0].props.metadata.client.client_id).toBe('cid');
  });
});

describe('AgentRunsApi via the manager', () => {
  it('POSTs the result to /api/agent-runs/{repo}/{run}/client-result', async () => {
    const fetchMock = vi.fn(async () => new Response('{"status":"applied"}', { status: 200, headers: { 'content-type': 'application/json' } }));
    const manager = new ConversationManager('http://x', 'demo', fakeAuth, { fetch: fetchMock as unknown as typeof fetch });
    const body: ClientToolResultBody = { resume_key: 'client:c1', delivery_id: 'd1', result: { ok: true } };
    const ack = await manager.agentRuns().submitClientToolResult('run 1', body);
    expect(ack).toEqual({ status: 'applied' });
    const [url, req] = fetchMock.mock.calls[0] as any;
    expect(url).toBe('http://x/api/agent-runs/demo/run%201/client-result');
    expect(req.method).toBe('POST');
    expect(JSON.parse(req.body)).toEqual(body);
  });
});

// ---------------------------------------------------------------------------
// ConversationStore
// ---------------------------------------------------------------------------

function harness(provider: Partial<ClientToolsProvider> & { execute: ClientToolsProvider['execute'] }) {
  let emit: (e: ChatEvent) => void = () => {};
  const created: any[] = [];
  const submitted: Array<{ runId: string; body: ClientToolResultBody }> = [];
  const manager = {
    subscribe: (_p: string, on: (e: ChatEvent) => void) => {
      emit = on;
      return { unsubscribe() {}, waitUntilConnected: async () => {} };
    },
    createUserMessage: vi.fn(async (_p: string, content: string, opts: any) => {
      created.push({ content, opts });
      return opts.clientId;
    }),
    agentRuns: () => ({
      submitClientToolResult: async (runId: string, body: ClientToolResultBody) => {
        submitted.push({ runId, body });
        return { status: 'applied' as const };
      },
    }),
    getMessages: async () => [],
  };
  const tools: ClientToolsProvider = {
    getTools: async () => [TOOL],
    getContext: async () => ({ location: { path: '/p' } }),
    ...provider,
  };
  const store = new ConversationStore({
    database: { conversations: manager } as unknown as Database,
    conversationPath: '/c',
    clientTools: tools,
  });
  const callEvent = (over: Partial<ClientToolCallEvent> = {}): ClientToolCallEvent => ({
    type: 'client_tool_call', run_id: 'r1', call_id: 'call-1', resume_key: 'client:call-1',
    name: 'insert_blocks', args: { n: 1 }, client_id: created[0]?.opts.clientId ?? 'x',
    expires_at_ms: Date.now() + 60000, ...over,
  });
  const flush = () => new Promise((r) => setTimeout(r, 0));
  return { store, created, submitted, emit: (e: ChatEvent) => emit(e), callEvent, flush };
}

describe('ConversationStore client tools', () => {
  it('attaches tools/context and attachments on send', async () => {
    const h = harness({ execute: async () => ({ ok: true }) });
    await h.store.sendMessage('hello', { attachments: [ATTACHMENT] });
    expect(h.created[0].opts.client).toEqual({ tools: [TOOL], context: { location: { path: '/p' } } });
    expect(h.created[0].opts.attachments).toEqual([ATTACHMENT]);
    expect(typeof h.created[0].opts.clientId).toBe('string');
  });

  it('sends without client when the provider fails', async () => {
    const h = harness({ execute: async () => ({ ok: true }), getTools: async () => { throw new Error('boom'); } });
    await h.store.sendMessage('hello');
    expect(h.created[0].opts.client).toBeUndefined();
  });

  it('executes calls for its own client_id and posts the result with fresh tools/context', async () => {
    const execute = vi.fn(async () => ({ ok: true, content: { done: 1 }, untrusted: true }));
    const h = harness({ execute });
    await h.store.sendMessage('hello');
    h.emit(h.callEvent());
    expect(h.store.getSnapshot().activeToolCalls[0]).toMatchObject({ id: 'call-1', functionName: 'insert_blocks', status: 'running' });
    await h.flush();
    expect(execute).toHaveBeenCalledWith('insert_blocks', { n: 1 });
    expect(h.submitted).toHaveLength(1);
    expect(h.submitted[0].runId).toBe('r1');
    const body = h.submitted[0].body;
    expect(body.resume_key).toBe('client:call-1');
    expect(typeof body.delivery_id).toBe('string');
    expect(body.result).toEqual({ ok: true, content: { done: 1 }, untrusted: true });
    expect(body.client_tools).toEqual([TOOL]);
    expect(body.client_context).toEqual({ location: { path: '/p' } });
    expect(h.store.getSnapshot().activeToolCalls[0].status).toBe('completed');
  });

  it("ignores calls addressed to another tab's client_id", async () => {
    const execute = vi.fn(async () => ({ ok: true }));
    const h = harness({ execute });
    await h.store.sendMessage('hello');
    h.emit(h.callEvent({ client_id: 'someone-else' }));
    await h.flush();
    expect(execute).not.toHaveBeenCalled();
    expect(h.submitted).toHaveLength(0);
    expect(h.store.getSnapshot().activeToolCalls).toHaveLength(0);
  });

  it('never executes a replayed call_id twice', async () => {
    const execute = vi.fn(async () => ({ ok: true }));
    const h = harness({ execute });
    await h.store.sendMessage('hello');
    h.emit(h.callEvent());
    h.emit(h.callEvent());
    await h.flush();
    h.emit(h.callEvent());
    await h.flush();
    expect(execute).toHaveBeenCalledTimes(1);
    expect(h.submitted).toHaveLength(1);
  });

  it('answers a thrown tool as ok:false with the message', async () => {
    const h = harness({ execute: async () => { throw new Error('nope'); } });
    await h.store.sendMessage('hello');
    h.emit(h.callEvent());
    await h.flush();
    expect(h.submitted[0].body.result).toEqual({ ok: false, error: 'nope' });
    expect(h.store.getSnapshot().activeToolCalls[0].status).toBe('failed');
  });

  it('answers {ok:false,error:"timeout"} before expires_at_ms when a tool hangs', async () => {
    const h = harness({ execute: () => new Promise(() => {}) });
    await h.store.sendMessage('hello');
    const expires = Date.now() + 1600; // margin 1500ms -> fires after ~100ms
    h.emit(h.callEvent({ expires_at_ms: expires }));
    await new Promise((r) => setTimeout(r, 400));
    expect(h.submitted).toHaveLength(1);
    expect(h.submitted[0].body.result).toEqual({ ok: false, error: 'timeout' });
    expect(Date.now()).toBeLessThan(expires);
  });
});
