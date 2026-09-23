/**
 * Durable agent runs over HTTP.
 *
 * A run is a server-side, crash-safe execution of an agent: a monotonic event
 * log, a lifecycle (`queued → running → waiting/paused → completed/failed/
 * stopped`), and atomic, idempotent controls (stop, pause, resume, steer,
 * approve, answer). Two ways to drive one:
 *
 * - **server-driven** — create it with a `reducer` (any function, in any
 *   supported language); the server's job queue drives it and you watch it
 *   with {@link AgentRunsApi.stream};
 * - **client-driven** — create it without a reducer and drive it yourself with
 *   the lease calls ({@link AgentRunsApi.acquire}, {@link AgentRunsApi.begin},
 *   {@link AgentRunsApi.finish}, ...). Your operations are still durable and
 *   fenced, and anyone authorized can still stop or steer the run.
 */

import {
  checkpointBody,
  spawnBody,
  type ChildAction,
  type CheckpointOptions,
  type MailboxEntry,
  type SpawnChildOptions,
  type SpawnChildResult,
} from './agent-run-children';

/** Where a run lives, what it is about, and what it runs as. */
export interface CreateAgentRunOptions {
  /** What the run is about (one live run per subject). */
  subject: { workspace: string; path: string; node_id?: string | null };
  /** Branch (default "main"). */
  branch?: string;
  /** Opaque agent reference, e.g. an agent node path. */
  agentRef?: string;
  /** Idempotency key for the create (a retried create returns the same run). */
  createKey?: string;
  /** Limits; exceeding one pauses (default) or fails the run. */
  budgets?: Record<string, unknown>;
  /** Input, delivered to the reducer as `run_started.data.input`. */
  input?: unknown;
  /** Server-driven: the reducer function. Omit for a client-driven run. */
  reducer?: { function_path: string; handler?: string };
  /** Opaque executor configuration (e.g. `model_turn_function`). */
  executorConfig?: Record<string, unknown>;
  /** A secret whose holder may control the run. */
  controlCapability?: string;
  /** Run as this agent (`"ws:/path"` or `"/path"`), on your behalf. */
  asAgent?: string;
}

/** The answer to a create. */
export interface CreateAgentRunResult {
  run_id: string;
  /** False when the subject's live run (or the create key's run) came back. */
  created: boolean;
  status: AgentRunStatus;
}

export type AgentRunStatus =
  | 'queued'
  | 'running'
  | 'waiting'
  | 'paused'
  | 'cancelling'
  | 'completed'
  | 'failed'
  | 'stopped';

/** A run as a reader sees it. */
export interface AgentRunView {
  run: Record<string, unknown> & { run_id: string; last_seq: number };
  status: AgentRunStatus;
  /** The plan to display, already overridden by the run status. */
  projection: { items?: Array<{ key: string; title: string; status: string }>; summary?: string } | null;
}

/** One durable event (`seq` is contiguous from 1). */
export interface AgentRunEvent {
  run_id: string;
  seq: number;
  at_ms: number;
  turn?: number | null;
  op_id?: string | null;
  kind: { type: string; [key: string]: unknown };
}

/** The answer to a control. */
export type AgentRunControlAck =
  | { ack: 'applied'; seq: number }
  | { ack: 'duplicate'; original_seq: number }
  | { ack: 'rejected'; reason: string; seq: number };

/** A control command, as the server's contract spells it. */
export type AgentRunCommand =
  | { command: 'stop'; reason?: string }
  | { command: 'pause' }
  | { command: 'resume'; budget_increase?: Record<string, unknown>; accept_reducer_change?: boolean }
  | { command: 'steer'; input: unknown }
  | {
      command: 'approve';
      request_id: string;
      decision: { decision: 'approve' } | { decision: 'reject'; reason?: string };
      subject_digest: string;
    }
  | { command: 'provide_input'; request_id: string; value: unknown };

/** A lease held by a client driver; present it on every driver call. */
export interface AgentRunFence {
  owner: string;
  epoch: number;
}

/** Begin an operation (client-driven runs). */
export interface BeginOperationOptions {
  kind: 'model_turn' | 'tool_call' | 'compaction' | string;
  input?: unknown;
  replay_safe?: boolean;
  non_interruptible?: boolean;
  for_call_id?: string;
  answers?: string[];
}

/** Finish an operation (client-driven runs). */
export interface FinishOperationOptions {
  outcome: 'succeeded' | 'waiting' | 'retryable' | 'blocked' | 'failed' | 'cancelled';
  payload?: unknown;
  tool_calls?: string[];
  usage?: { input_tokens: number; output_tokens: number };
  resume_key?: string;
}

/** How the API reaches the server (supplied by the HTTP client). */
export interface AgentRunsTransport {
  request<T>(method: string, path: string, body?: unknown): Promise<T>;
  /** Absolute URL for `path`. */
  url(path: string): string;
  /** Headers for a raw fetch (auth, tenant). */
  headers(): Record<string, string>;
  fetch: typeof fetch;
}

/** The wire body of a create (shared by the HTTP and WebSocket APIs). */
export function createRunBody(options: CreateAgentRunOptions): Record<string, unknown> {
  return {
    subject: options.subject,
    branch: options.branch,
    agent_ref: options.agentRef,
    create_key: options.createKey,
    budgets: options.budgets,
    input: options.input ?? null,
    reducer: options.reducer,
    executor_config: options.executorConfig,
    control_capability: options.controlCapability,
    as_agent: options.asAgent,
  };
}

/** What a run is about: `{workspace, path, node_id?}` or `"workspace:/path"`. */
export type AgentRunSubject = { workspace: string; path?: string; node_id?: string | null } | string;

/** `"ws:/path"` → `{workspace, path}`. */
export function subjectOf(subject: AgentRunSubject): { workspace: string; path?: string; node_id?: string | null } {
  if (typeof subject !== 'string') return subject;
  const cut = subject.indexOf(':/');
  if (cut <= 0) throw new Error(`subject must be "workspace:/path", got "${subject}"`);
  return { workspace: subject.slice(0, cut), path: subject.slice(cut + 1) };
}

let controlCounter = 0;

/** A fresh control id (controls are idempotent per id; retry with the SAME id). */
export function newControlId(prefix = 'ctl'): string {
  controlCounter += 1;
  const rand = Math.random().toString(36).slice(2, 10);
  return `${prefix}-${Date.now().toString(36)}-${controlCounter}-${rand}`;
}

export class AgentRunsApi {
  constructor(
    private repository: string,
    private transport: AgentRunsTransport,
  ) {}

  private base(runId?: string, rest = ''): string {
    const repo = encodeURIComponent(this.repository);
    const run = runId ? `/${encodeURIComponent(runId)}` : '';
    return `/api/agent-runs/${repo}${run}${rest}`;
  }

  /** Create a run, or get back the subject's live run. */
  create(options: CreateAgentRunOptions): Promise<CreateAgentRunResult> {
    return this.transport.request('POST', this.base(), createRunBody(options));
  }

  /** Every run about `subject` you may see, newest (the live one) first. */
  bySubject(subject: AgentRunSubject, limit = 20): Promise<AgentRunView[]> {
    const s = subjectOf(subject);
    const q = new URLSearchParams({ subject: `${s.workspace}:${s.path ?? ''}`, limit: String(limit) });
    if (s.node_id) q.set('subject_node_id', s.node_id);
    return this.transport.request('GET', `${this.base()}?${q.toString()}`);
  }

  /** Read a run (record + effective projection). */
  get(runId: string): Promise<AgentRunView> {
    return this.transport.request('GET', this.base(runId));
  }

  /** Your runs in one status (default "running"). */
  list(status: AgentRunStatus = 'running', limit = 50): Promise<AgentRunView[]> {
    return this.transport.request('GET', `${this.base()}?status=${status}&limit=${limit}`);
  }

  /** Durable events after `afterSeq` (gap-free; page with the last seq). */
  events(runId: string, afterSeq = 0, limit = 500): Promise<AgentRunEvent[]> {
    return this.transport.request('GET', this.base(runId, `/events?after_seq=${afterSeq}&limit=${limit}`));
  }

  /** Any control. Retrying with the same `controlId` is a no-op. */
  control(
    runId: string,
    command: AgentRunCommand,
    controlId: string = newControlId(command.command),
    capability?: string,
  ): Promise<AgentRunControlAck> {
    return this.transport.request('POST', this.base(runId, '/control'), {
      control_id: controlId,
      command,
      capability,
    });
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
  approve(
    runId: string,
    requestId: string,
    subjectDigest: string,
    reject?: { reason?: string },
    controlId?: string,
  ): Promise<AgentRunControlAck> {
    const decision = reject ? { decision: 'reject' as const, reason: reject.reason } : { decision: 'approve' as const };
    return this.control(
      runId,
      { command: 'approve', request_id: requestId, decision, subject_digest: subjectDigest },
      controlId,
    );
  }
  /** Answer an input request. */
  answer(runId: string, requestId: string, value: unknown, controlId?: string): Promise<AgentRunControlAck> {
    return this.control(runId, { command: 'provide_input', request_id: requestId, value }, controlId);
  }

  /**
   * Stream the run's events: replay after `afterSeq`, then live. Ends after
   * the run's terminal event. Resume after a disconnect with the last `seq`.
   */
  async *stream(runId: string, afterSeq = 0, signal?: AbortSignal): AsyncGenerator<AgentRunEvent> {
    const response = await this.transport.fetch(
      this.transport.url(this.base(runId, `/stream?after_seq=${afterSeq}`)),
      { headers: { ...this.transport.headers(), Accept: 'text/event-stream' }, signal },
    );
    if (!response.ok || !response.body) {
      throw new Error(`agent run stream failed: ${response.status} ${await response.text()}`);
    }
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    for (;;) {
      const { value, done } = await reader.read();
      if (done) return;
      buffer += decoder.decode(value, { stream: true });
      let cut: number;
      while ((cut = buffer.indexOf('\n\n')) >= 0) {
        const block = buffer.slice(0, cut);
        buffer = buffer.slice(cut + 2);
        let event = 'message';
        let data = '';
        for (const line of block.split('\n')) {
          if (line.startsWith('event:')) event = line.slice(6).trim();
          else if (line.startsWith('data:')) data += line.slice(5).trim();
        }
        if (event === 'end') return;
        if (event === 'error') throw new Error(`agent run stream error: ${data}`);
        if (event === 'run-event' && data) yield JSON.parse(data) as AgentRunEvent;
      }
    }
  }

  // ---- client-driven runs -------------------------------------------------

  /** Take the lease of a queued client-driven run. */
  acquire(runId: string): Promise<AgentRunFence> {
    return this.transport.request('POST', this.base(runId, '/lease/acquire'), {});
  }
  renew(runId: string, fence: AgentRunFence): Promise<AgentRunStatus> {
    return this.transport.request('POST', this.base(runId, '/lease/renew'), { fence });
  }
  release(runId: string, fence: AgentRunFence): Promise<AgentRunStatus> {
    return this.transport.request('POST', this.base(runId, '/lease/release'), { fence });
  }
  /** Begin an operation; returns the active operation (its `op_id`). */
  begin(runId: string, fence: AgentRunFence, op: BeginOperationOptions): Promise<{ op_id: string } & Record<string, unknown>> {
    return this.transport.request('POST', this.base(runId, '/operations'), { fence, ...op });
  }
  /** Record an operation's result. */
  finish(runId: string, fence: AgentRunFence, opId: string, result: FinishOperationOptions): Promise<AgentRunStatus> {
    return this.transport.request('POST', this.base(runId, '/operations/finish'), { fence, op_id: opId, ...result });
  }
  /** Open approval/input requests and wait (`kind: "approval" | "input"`). */
  wait(runId: string, fence: AgentRunFence, requests: Array<Record<string, unknown>>): Promise<AgentRunStatus> {
    return this.transport.request('POST', this.base(runId, '/wait'), { fence, requests });
  }
  /** End a client-driven run. */
  complete(
    runId: string,
    fence: AgentRunFence,
    status: 'completed' | 'failed',
    outcome: { kind: string; code?: string; message?: string; detail?: unknown },
  ): Promise<AgentRunStatus> {
    return this.transport.request('POST', this.base(runId, '/complete'), { fence, status, outcome });
  }

  // ── Child runs, mailbox, checkpoints, usage ──────────────────────────────

  /** Spawn a child with a typed objective; its budgets come out of this run's. */
  spawnChild(runId: string, options: SpawnChildOptions): Promise<SpawnChildResult> {
    const q = options.branch ? `?branch=${encodeURIComponent(options.branch)}` : '';
    return this.transport.request('POST', this.base(runId, `/children${q}`), spawnBody(options));
  }
  /** Every child of a run: the parent's link plus the child's live status. */
  children(runId: string): Promise<Array<{ link: Record<string, unknown>; status?: AgentRunStatus; usage?: unknown }>> {
    return this.transport.request('GET', this.base(runId, '/children'));
  }
  /** Inspect one child: record, events after `afterSeq`, usage, checkpoint. */
  inspectChild(runId: string, childId: string, afterSeq = 0, limit = 200): Promise<Record<string, unknown>> {
    const rest = `/children/${encodeURIComponent(childId)}?after_seq=${afterSeq}&limit=${limit}`;
    return this.transport.request('GET', this.base(runId, rest));
  }
  /** Message, steer, interrupt or resume a child (idempotent per `controlId`). */
  controlChild(
    runId: string,
    childId: string,
    action: ChildAction,
    controlId: string = newControlId(action.action),
  ): Promise<AgentRunControlAck> {
    const rest = `/children/${encodeURIComponent(childId)}/control`;
    return this.transport.request('POST', this.base(runId, rest), { control_id: controlId, ...action });
  }
  messageChild(runId: string, childId: string, message: unknown, controlId?: string): Promise<AgentRunControlAck> {
    return this.controlChild(runId, childId, { action: 'message', message }, controlId);
  }
  steerChild(runId: string, childId: string, input: unknown, controlId?: string): Promise<AgentRunControlAck> {
    return this.controlChild(runId, childId, { action: 'steer', input }, controlId);
  }
  interruptChild(runId: string, childId: string, reason?: string, mode: 'stop' | 'pause' = 'stop', controlId?: string): Promise<AgentRunControlAck> {
    return this.controlChild(runId, childId, { action: 'interrupt', mode, reason }, controlId);
  }
  /** A client-driven parent waits (under its lease) for a child's hand-back. */
  waitChild(runId: string, childId: string, fence: AgentRunFence, expiresAtMs?: number): Promise<AgentRunStatus> {
    const rest = `/children/${encodeURIComponent(childId)}/wait`;
    return this.transport.request('POST', this.base(runId, rest), { fence, expires_at_ms: expiresAtMs });
  }
  /** Unacknowledged mailbox items (hand-backs and child messages). */
  mailbox(runId: string): Promise<MailboxEntry[]> {
    return this.transport.request('GET', this.base(runId, '/mailbox'));
  }
  /** Acknowledge mailbox items up to `upTo`. */
  ackMailbox(runId: string, upTo: number): Promise<{ remaining: number }> {
    return this.transport.request('POST', this.base(runId, '/mailbox/ack'), { up_to: upTo });
  }
  /** A child posts a message to its parent's mailbox (idempotent per id). */
  postToParent(runId: string, messageId: string, message: unknown): Promise<{ mail_no: number }> {
    return this.transport.request('POST', this.base(runId, '/post-to-parent'), { message_id: messageId, message });
  }
  /** Write a structured checkpoint (compaction keeps state, not prose). */
  checkpoint(runId: string, options: CheckpointOptions): Promise<Record<string, unknown>> {
    return this.transport.request('POST', this.base(runId, '/checkpoints'), checkpointBody(options));
  }
  /** Read a checkpoint (default: latest) with its structured state. */
  readCheckpoint(runId: string, checkpointNo?: number): Promise<{ checkpoint: Record<string, unknown>; state: unknown }> {
    return this.transport.request('GET', this.base(runId, `/checkpoints/${checkpointNo ?? 'latest'}`));
  }
  /** Usage accounting: own, children, reserved, spare. */
  usage(runId: string): Promise<Record<string, unknown>> {
    return this.transport.request('GET', this.base(runId, '/usage'));
  }
}
