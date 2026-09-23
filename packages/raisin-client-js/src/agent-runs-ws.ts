/**
 * Durable agent runs over the WebSocket connection you already hold.
 *
 * The same contract as {@link AgentRunsApi} (HTTP), without a second
 * transport: find the run of a conversation, follow it live, control it and
 * read its children. `subscribe` is gap-free and resumable — it replays the
 * durable log after `afterSeq`, then follows it, delivering every event once
 * and in `seq` order even across a reconnect (pass the last `seq` you saw).
 *
 * ```ts
 * const runs = db.runs();
 * const [live] = await runs.bySubject('ai:/agents/builder/inbox/chats/c1');
 * const sub = await runs.subscribe(live.run.run_id, { afterSeq: 0 });
 * for await (const ev of sub) console.log(ev.seq, ev.kind.type);
 * await runs.stop(live.run.run_id, 'user asked');
 * ```
 */

import {
  createRunBody,
  newControlId,
  subjectOf,
  type AgentRunCommand,
  type AgentRunControlAck,
  type AgentRunEvent,
  type AgentRunStatus,
  type AgentRunSubject,
  type AgentRunView,
  type CreateAgentRunOptions,
  type CreateAgentRunResult,
} from './agent-runs';
import type { EventHandler } from './events';
import { RequestType, type EventMessage, type RequestContext } from './protocol';

type SendRequest = (payload: unknown, requestType: string, context?: RequestContext) => Promise<unknown>;

/** A child as its parent lists it. */
export interface AgentRunChildView {
  link: Record<string, unknown> & { run_id: string; child_no: number };
  status: AgentRunStatus | null;
  usage: Record<string, unknown> | null;
}

/** Options of {@link AgentRunsWsApi.subscribe}. */
export interface AgentRunSubscribeOptions {
  /** Replay events after this seq (0 = from the start). */
  afterSeq?: number;
  /** Called for every event, in seq order (in addition to iteration). */
  onEvent?: (event: AgentRunEvent) => void;
  /** Called once when the run ended and its log is complete. */
  onEnd?: (lastSeq: number) => void;
  /** Stop following. */
  signal?: AbortSignal;
}

/** A live subscription: iterate it, or use the callbacks. */
export interface AgentRunSubscription extends AsyncIterable<AgentRunEvent> {
  /** The last seq delivered (resume from it after a reconnect). */
  readonly lastSeq: number;
  /** Resolves when the run ended or the subscription was closed. */
  readonly done: Promise<void>;
  /** Stop following. */
  unsubscribe(): Promise<void>;
}

const TERMINAL: AgentRunStatus[] = ['completed', 'failed', 'stopped'];

export class AgentRunsWsApi {
  constructor(
    private context: RequestContext,
    private sendRequest: SendRequest,
    private eventHandler: EventHandler,
  ) {}

  private call<T>(type: RequestType, payload: unknown): Promise<T> {
    return this.sendRequest(payload, type, this.context) as Promise<T>;
  }

  /** Create a run, or get back the subject's live run. */
  create(options: CreateAgentRunOptions): Promise<CreateAgentRunResult> {
    return this.call(RequestType.AgentRunCreate, createRunBody(options));
  }

  /** Read a run (record + effective projection). */
  get(runId: string): Promise<AgentRunView> {
    return this.call(RequestType.AgentRunGet, { run_id: runId });
  }

  /**
   * Every run about `subject` (a conversation, a document) you may see,
   * newest first — the live run, if there is one, comes first.
   */
  bySubject(subject: AgentRunSubject, limit = 20): Promise<AgentRunView[]> {
    return this.call(RequestType.AgentRunBySubject, { ...subjectOf(subject), limit });
  }

  /** The newest run of `subject`, or null. */
  async latest(subject: AgentRunSubject): Promise<AgentRunView | null> {
    const [first] = await this.bySubject(subject, 1);
    return first ?? null;
  }

  /** Your runs in one status (default "running"). */
  list(status: AgentRunStatus = 'running', limit = 50): Promise<AgentRunView[]> {
    return this.call(RequestType.AgentRunList, { status, limit });
  }

  /** Durable events after `afterSeq`. */
  events(runId: string, afterSeq = 0, limit = 500): Promise<AgentRunEvent[]> {
    return this.call(RequestType.AgentRunEvents, { run_id: runId, after_seq: afterSeq, limit });
  }

  /** The children a run spawned, with their current status. */
  children(runId: string): Promise<AgentRunChildView[]> {
    return this.call(RequestType.AgentRunChildren, { run_id: runId });
  }

  /** Any control. Retrying with the same `controlId` is a no-op. */
  control(
    runId: string,
    command: AgentRunCommand,
    controlId: string = newControlId(command.command),
    capability?: string,
  ): Promise<AgentRunControlAck> {
    return this.call(RequestType.AgentRunControl, { run_id: runId, control_id: controlId, command, capability });
  }

  stop(runId: string, reason?: string, controlId?: string): Promise<AgentRunControlAck> {
    return this.control(runId, { command: 'stop', reason }, controlId);
  }
  pause(runId: string, controlId?: string): Promise<AgentRunControlAck> {
    return this.control(runId, { command: 'pause' }, controlId);
  }
  resume(runId: string, controlId?: string): Promise<AgentRunControlAck> {
    return this.control(runId, { command: 'resume' }, controlId);
  }
  /** Queue input for the run's next safe boundary. */
  steer(runId: string, input: unknown, controlId?: string): Promise<AgentRunControlAck> {
    return this.control(runId, { command: 'steer', input }, controlId);
  }
  /** Decide an approval request; the digest ties it to one changeset. */
  approve(runId: string, requestId: string, subjectDigest: string, reject?: { reason?: string }, controlId?: string) {
    const decision = reject ? { decision: 'reject' as const, reason: reject.reason } : { decision: 'approve' as const };
    return this.control(runId, { command: 'approve', request_id: requestId, decision, subject_digest: subjectDigest }, controlId);
  }
  /** Answer an input request. */
  answer(runId: string, requestId: string, value: unknown, controlId?: string): Promise<AgentRunControlAck> {
    return this.control(runId, { command: 'provide_input', request_id: requestId, value }, controlId);
  }

  /**
   * Follow a run: replay after `afterSeq`, then live, until the run ends.
   *
   * Events that reach the socket before the subscription is known here are
   * not lost: the durable log is read once the listener is in place, live
   * events are held until that read is merged, and every seq is delivered
   * exactly once, in order.
   */
  async subscribe(runId: string, options: AgentRunSubscribeOptions = {}): Promise<AgentRunSubscription> {
    let last = options.afterSeq ?? 0;
    let ready = false;
    let closed = false;
    const held: AgentRunEvent[] = [];
    const buffer: AgentRunEvent[] = [];
    const waiters: Array<(r: IteratorResult<AgentRunEvent>) => void> = [];
    let resolveDone!: () => void;
    const done = new Promise<void>((r) => (resolveDone = r));

    const close = (ended: boolean) => {
      if (closed) return;
      closed = true;
      if (ended) options.onEnd?.(last);
      for (const w of waiters.splice(0)) w({ value: undefined, done: true });
      resolveDone();
    };
    const deliver = (ev: AgentRunEvent) => {
      if (closed || ev.seq <= last) return;
      last = ev.seq;
      options.onEvent?.(ev);
      const w = waiters.shift();
      if (w) w({ value: ev, done: false });
      else buffer.push(ev);
    };
    const listener = (message: EventMessage) => {
      if (message.event_type === 'agent_run_end') {
        if (ready) close(true);
        else held.push({ seq: Number.MAX_SAFE_INTEGER } as AgentRunEvent);
        return;
      }
      const ev = message.payload as unknown as AgentRunEvent;
      if (ready) deliver(ev);
      else held.push(ev);
    };

    const response = await this.call<{ subscription_id: string }>(RequestType.AgentRunSubscribe, {
      run_id: runId,
      after_seq: last,
    });
    const subscriptionId = response.subscription_id;
    this.eventHandler.addFlowEventListener(subscriptionId, listener);

    const unsubscribe = async () => {
      this.eventHandler.removeFlowEventListener(subscriptionId);
      close(false);
      await this.call(RequestType.AgentRunUnsubscribe, { subscription_id: subscriptionId }).catch(() => undefined);
    };
    options.signal?.addEventListener('abort', () => void unsubscribe(), { once: true });

    // Backfill from the log, then release what arrived meanwhile.
    for (;;) {
      const page = await this.events(runId, last, 500);
      page.forEach(deliver);
      if (page.length < 500) break;
    }
    ready = true;
    const endSeen = held.some((e) => e.seq === Number.MAX_SAFE_INTEGER);
    held.filter((e) => e.seq !== Number.MAX_SAFE_INTEGER).sort((a, b) => a.seq - b.seq).forEach(deliver);
    if (endSeen) close(true);
    else {
      // An `end` sent before the listener existed: the run itself says so.
      const view = await this.get(runId).catch(() => null);
      const finalized = !view?.run?.domain || (view.run.domain as { finalized?: boolean }).finalized !== false;
      if (view && TERMINAL.includes(view.status) && finalized) {
        const tail = await this.events(runId, last, 500).catch(() => []);
        tail.forEach(deliver);
        this.eventHandler.removeFlowEventListener(subscriptionId);
        close(true);
      }
    }
    void done.then(() => this.eventHandler.removeFlowEventListener(subscriptionId));

    return {
      get lastSeq() {
        return last;
      },
      done,
      unsubscribe,
      [Symbol.asyncIterator]() {
        return {
          next: () => {
            const ev = buffer.shift();
            if (ev) return Promise.resolve({ value: ev, done: false });
            if (closed) return Promise.resolve({ value: undefined, done: true });
            return new Promise<IteratorResult<AgentRunEvent>>((r) => waiters.push(r));
          },
          return: async () => {
            await unsubscribe();
            return { value: undefined, done: true };
          },
        };
      },
    };
  }
}
