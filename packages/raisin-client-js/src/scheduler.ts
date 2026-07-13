/**
 * One-shot scheduled invocation operations.
 *
 * Schedule a single future run of a server-side function or flow, cancel it
 * before it fires (by job id or a caller-supplied external key), and inspect
 * pending/completed invocations. Scheduling is restart-safe: the invocation
 * is persisted server-side and fires even if the server restarts before
 * `runAt`.
 */

import { RequestContext, RequestType } from './protocol';

/** What a scheduled invocation targets. */
export type ScheduledInvocationTargetKind = 'function' | 'flow';

/** Options for {@link SchedulerApi.schedule}. */
export interface ScheduleInvocationOptions {
  /** "function" or "flow" */
  targetKind: ScheduledInvocationTargetKind;
  /** Path of the target function or flow node */
  targetPath: string;
  /** Input passed to the target when it fires */
  input?: unknown;
  /** When to fire — RFC3339 string or Date (a past time fires immediately) */
  runAt: string | Date;
  /** Caller-supplied idempotency/lookup key (find or cancel by this later) */
  externalKey?: string;
  /** Branch the invocation executes against (default "main") */
  branch?: string;
  /** Workspace the invocation executes against (default "functions") */
  workspace?: string;
  /** Retry attempts if the invocation fails (default 0 — one-shot) */
  maxRetries?: number;
}

/** Result of scheduling an invocation. */
export interface ScheduledInvocationRef {
  job_id: string;
  invocation_id: string;
  status: string;
  run_at: string;
}

/** A scheduled invocation as returned by get/list. */
export interface ScheduledInvocation {
  job_id: string;
  invocation_id: string;
  target_kind: string;
  target_path: string | null;
  input: unknown;
  actor: string | null;
  external_key: string | null;
  run_at: string | null;
  /** "scheduled" | "running" | "completed" | "cancelled" | "failed" */
  status: string;
  error: string | null;
  result: unknown;
}

/** Filters for {@link SchedulerApi.list}. */
export interface ListInvocationsFilter {
  externalKey?: string;
  status?: string;
}

export class SchedulerApi {
  private sendRequest: (payload: unknown, requestType: string) => Promise<unknown>;

  constructor(
    _context: RequestContext,
    sendRequest: (payload: unknown, requestType: string) => Promise<unknown>
  ) {
    this.sendRequest = sendRequest;
  }

  /**
   * Schedule a one-shot invocation of a function or flow at a fixed time.
   */
  async schedule(options: ScheduleInvocationOptions): Promise<ScheduledInvocationRef> {
    const runAt =
      options.runAt instanceof Date ? options.runAt.toISOString() : options.runAt;
    return this.sendRequest(
      {
        target_kind: options.targetKind,
        target_path: options.targetPath,
        input: options.input ?? {},
        run_at: runAt,
        external_key: options.externalKey,
        branch: options.branch,
        workspace: options.workspace,
        max_retries: options.maxRetries
      },
      RequestType.ScheduledInvocationCreate
    ) as Promise<ScheduledInvocationRef>;
  }

  /**
   * Cancel a pending invocation before it fires, by job id or external key.
   */
  async cancel(
    ref: string | { jobId?: string; externalKey?: string }
  ): Promise<{ job_id: string; status: string }> {
    const payload =
      typeof ref === 'string'
        ? { job_id: ref }
        : { job_id: ref.jobId, external_key: ref.externalKey };
    return this.sendRequest(
      payload,
      RequestType.ScheduledInvocationCancel
    ) as Promise<{ job_id: string; status: string }>;
  }

  /**
   * List this repository's scheduled invocations.
   */
  async list(filter?: ListInvocationsFilter): Promise<ScheduledInvocation[]> {
    const result = (await this.sendRequest(
      {
        external_key: filter?.externalKey,
        status: filter?.status
      },
      RequestType.ScheduledInvocationList
    )) as { invocations?: ScheduledInvocation[] };
    return result?.invocations ?? [];
  }

  /**
   * Fetch a single scheduled invocation, by job id or external key.
   */
  async get(
    ref: string | { jobId?: string; externalKey?: string }
  ): Promise<ScheduledInvocation> {
    const payload =
      typeof ref === 'string'
        ? { job_id: ref }
        : { job_id: ref.jobId, external_key: ref.externalKey };
    return this.sendRequest(
      payload,
      RequestType.ScheduledInvocationGet
    ) as Promise<ScheduledInvocation>;
  }
}
