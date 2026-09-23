import { describe, expect, it } from 'vitest';
import { AgentRunsApi, type AgentRunsTransport } from './agent-runs';

type Call = { method: string; path: string; body?: unknown };

function transport(calls: Call[]): AgentRunsTransport {
  return {
    request: async <T>(method: string, path: string, body?: unknown) => {
      calls.push({ method, path, body });
      return { ok: true } as T;
    },
    url: (path) => `http://x${path}`,
    headers: () => ({}),
    fetch: fetch,
  };
}

describe('AgentRunsApi child runs', () => {
  it('maps a spawn onto the wire contract', async () => {
    const calls: Call[] = [];
    const api = new AgentRunsApi('studio', transport(calls));
    await api.spawnChild('p1', {
      objective: { title: 'schema', allowed_tools: ['/lib/raisin/node-dev/*'], context: { mode: 'snapshot' } },
      budgets: { max_operations: 5 },
      spawnKey: 'k1',
      reducer: { function_path: '/lib/r' },
    });
    expect(calls[0]).toMatchObject({
      method: 'POST',
      path: '/api/agent-runs/studio/p1/children',
      body: {
        objective: { title: 'schema', context: { mode: 'snapshot' } },
        budgets: { max_operations: 5 },
        spawn_key: 'k1',
        reducer: { function_path: '/lib/r' },
        inherit_reducer: false,
      },
    });
  });

  it('controls, waits, reads the mailbox and checkpoints', async () => {
    const calls: Call[] = [];
    const api = new AgentRunsApi('studio', transport(calls));
    await api.interruptChild('p1', 'c1', 'plan changed', 'stop', 'i-1');
    await api.messageChild('p1', 'c1', { text: 'hi' }, 'm-1');
    await api.waitChild('p1', 'c1', { owner: 'o', epoch: 2 });
    await api.ackMailbox('p1', 3);
    await api.postToParent('c1', 'q1', { question: 'which?' });
    await api.checkpoint('p1', { operationId: 'p1/op/4', state: { phase: 'apply' } });
    await api.readCheckpoint('p1');
    await api.usage('p1');
    expect(calls.map((c) => `${c.method} ${c.path}`)).toEqual([
      'POST /api/agent-runs/studio/p1/children/c1/control',
      'POST /api/agent-runs/studio/p1/children/c1/control',
      'POST /api/agent-runs/studio/p1/children/c1/wait',
      'POST /api/agent-runs/studio/p1/mailbox/ack',
      'POST /api/agent-runs/studio/c1/post-to-parent',
      'POST /api/agent-runs/studio/p1/checkpoints',
      'GET /api/agent-runs/studio/p1/checkpoints/latest',
      'GET /api/agent-runs/studio/p1/usage',
    ]);
    expect(calls[0].body).toEqual({ control_id: 'i-1', action: 'interrupt', mode: 'stop', reason: 'plan changed' });
    expect(calls[1].body).toEqual({ control_id: 'm-1', action: 'message', message: { text: 'hi' } });
    expect(calls[5].body).toMatchObject({ operation_id: 'p1/op/4', state: { phase: 'apply' }, large_refs: [] });
  });
});
