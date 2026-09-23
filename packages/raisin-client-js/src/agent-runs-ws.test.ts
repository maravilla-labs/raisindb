import { describe, expect, it } from 'vitest';
import { AgentRunsWsApi } from './agent-runs-ws';
import { EventHandler } from './events';
import type { AgentRunEvent } from './agent-runs';
import type { EventMessage } from './protocol';

const ev = (seq: number, type = 'status_changed'): AgentRunEvent => ({ run_id: 'r1', seq, at_ms: seq, kind: { type } });

function harness(log: AgentRunEvent[], status = 'running') {
  const calls: Array<{ type: string; payload: any }> = [];
  const handler = new EventHandler(async () => ({ subscription_id: 'x' }) as never);
  let emitBeforeAnswer: EventMessage[] = [];
  const send = async (payload: any, type: string) => {
    calls.push({ type, payload });
    switch (type) {
      case 'agent_run_subscribe':
        // Live events race the answer: this one reaches the socket first.
        for (const m of emitBeforeAnswer) handler.handleEvent(m);
        return { subscription_id: 'sub-1' };
      case 'agent_run_events':
        return log.filter((e) => e.seq > payload.after_seq);
      case 'agent_run_get':
        return { status, run: { run_id: 'r1', last_seq: log.length } };
      default:
        return { ok: true };
    }
  };
  const api = new AgentRunsWsApi({ tenant_id: 't', repository: 'studio' } as never, send as never, handler);
  const push = (type: string, payload: unknown) =>
    handler.handleEvent({ event_id: `e-${Math.random()}`, subscription_id: 'sub-1', event_type: type, payload, timestamp: '' } as EventMessage);
  return { api, calls, handler, push, setEarly: (m: EventMessage[]) => (emitBeforeAnswer = m) };
}

describe('AgentRunsWsApi', () => {
  it('speaks the agent_run_* request types', async () => {
    const { api, calls } = harness([]);
    await api.bySubject('ai:/agents/b/inbox/chats/c1');
    await api.stop('r1', 'enough', 'c-1');
    await api.children('r1');
    expect(calls.map((c) => c.type)).toEqual(['agent_run_by_subject', 'agent_run_control', 'agent_run_children']);
    expect(calls[0].payload).toMatchObject({ workspace: 'ai', path: '/agents/b/inbox/chats/c1', limit: 20 });
    expect(calls[1].payload).toEqual({ run_id: 'r1', control_id: 'c-1', command: { command: 'stop', reason: 'enough' }, capability: undefined });
  });

  it('replays from afterSeq, then delivers live events once and in order', async () => {
    const log = [ev(1), ev(2), ev(3)];
    const { api, push } = harness(log);
    const seen: number[] = [];
    const sub = await api.subscribe('r1', { afterSeq: 1, onEvent: (e) => seen.push(e.seq) });
    expect(seen).toEqual([2, 3]);
    push('agent_run_event', ev(3)); // a duplicate of the replay
    push('agent_run_event', ev(4, 'terminal'));
    push('agent_run_end', { type: 'end', last_seq: 4 });
    const got: number[] = [];
    for await (const e of sub) got.push(e.seq);
    expect(got).toEqual([2, 3, 4]);
    expect(sub.lastSeq).toBe(4);
    await sub.done;
  });

  it('an event that beat the subscribe answer is recovered from the log', async () => {
    const log = [ev(1), ev(2)];
    const h = harness(log);
    h.setEarly([{ event_id: 'early', subscription_id: 'sub-1', event_type: 'agent_run_event', payload: ev(2), timestamp: '' } as never]);
    const seen: number[] = [];
    await h.api.subscribe('r1', { onEvent: (e) => seen.push(e.seq) });
    expect(seen).toEqual([1, 2]);
  });

  it('ends a subscription to a run that already ended', async () => {
    const { api } = harness([ev(1), ev(2, 'terminal')], 'completed');
    let ended = -1;
    const sub = await api.subscribe('r1', { onEnd: (n) => (ended = n) });
    const got: number[] = [];
    for await (const e of sub) got.push(e.seq);
    expect(got).toEqual([1, 2]);
    expect(ended).toBe(2);
  });
});
