/**
 * Child runs, the durable mailbox, structured checkpoints and usage
 * accounting — the wire types of `/api/agent-runs/{repo}/{run}/children…`.
 *
 * A child is an ordinary durable run with a typed objective. Its terminal
 * hand-back lands in the parent's mailbox (and completes any tool or wait
 * parked on it); nothing polls a transient result.
 */

/** What context a child starts with. */
export type ChildContext =
  | { mode: 'none' }
  | { mode: 'recent_turns'; turns: number; items?: unknown[] }
  | { mode: 'snapshot'; checkpoint_no?: number; data?: unknown };

/** The typed objective of a child. */
export interface ChildObjective {
  title: string;
  instructions?: string;
  context?: ChildContext;
  /** Tool function paths (`*` suffix = prefix); empty = unrestricted. */
  allowed_tools?: string[];
  /** Write roots `{workspace, path, ops?}`, narrowed to the parent's grant. */
  allowed_writes?: Array<{ workspace: string; path?: string; ops?: string[] }>;
  expected_artifacts?: Array<{ kind: string; locator?: unknown; description?: string; required?: boolean }>;
  acceptance_checks?: Array<{ id: string; description?: string; check?: unknown; required?: boolean }>;
  hand_back?: { required_fields?: string[]; schema?: unknown; description?: string };
}

/** Spawn options. */
export interface SpawnChildOptions {
  objective: ChildObjective;
  budgets?: Record<string, unknown>;
  /** Default `fail`: a child paused on budget would stall its parent. */
  onExceeded?: 'pause' | 'fail';
  /** Idempotency key: a retry returns the same child. */
  spawnKey?: string;
  subject?: { workspace: string; path: string; node_id?: string };
  asAgent?: string;
  agentRef?: string;
  input?: unknown;
  executorConfig?: Record<string, unknown>;
  /** Server-driven child: its reducer function (any language). */
  reducer?: { function_path: string; handler?: string };
  /** Server-driven child on the parent's own reducer. */
  inheritReducer?: boolean;
  branch?: string;
}

/** The answer to a spawn. */
export interface SpawnChildResult {
  child_run_id: string;
  child_no: number;
  created: boolean;
  budgets: Record<string, unknown>;
  /** Resume key a tool answers `waiting` with to wait for the child. */
  resume_key: string;
}

/** What a parent does to a child. */
export type ChildAction =
  | { action: 'message'; message: unknown }
  | { action: 'steer'; input: unknown }
  | { action: 'interrupt'; mode?: 'stop' | 'pause'; reason?: string }
  | { action: 'resume' };

/** One mailbox item with its payload (a hand-back envelope or a message). */
export interface MailboxEntry {
  item: {
    mail_no: number;
    kind: 'completion' | 'message';
    from_run: string;
    seq: number;
    result_key: string;
    status?: string;
  };
  payload: unknown;
}

/** A structured checkpoint write (compaction). */
export interface CheckpointOptions {
  fence?: { owner: string; epoch: number };
  /** The in-flight operation doing the compaction (instead of a fence). */
  operationId?: string;
  reason?: 'compaction' | 'pause' | 'periodic' | 'before_terminal' | 'domain_requested' | 'delegation';
  summary?: string;
  transcriptCutoff?: { workspace: string; path: string; node_id?: string };
  /** Objective, constraints, decisions, pending questions, … */
  state?: unknown;
  largeRefs?: Array<{ key: string; bytes: number; content_type: string }>;
}

/** Wire body of a spawn. */
export function spawnBody(o: SpawnChildOptions): Record<string, unknown> {
  return {
    objective: o.objective,
    budgets: o.budgets,
    on_exceeded: o.onExceeded,
    spawn_key: o.spawnKey,
    subject: o.subject,
    as_agent: o.asAgent,
    agent_ref: o.agentRef,
    input: o.input ?? null,
    executor_config: o.executorConfig,
    reducer: o.reducer,
    inherit_reducer: o.inheritReducer ?? false,
  };
}

/** Wire body of a checkpoint write. */
export function checkpointBody(o: CheckpointOptions): Record<string, unknown> {
  return {
    fence: o.fence,
    operation_id: o.operationId,
    reason: o.reason,
    summary: o.summary,
    transcript_cutoff: o.transcriptCutoff,
    state: o.state,
    large_refs: o.largeRefs ?? [],
  };
}
