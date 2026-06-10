/**
 * InboxApi for human-in-the-loop tasks.
 *
 * Inbox tasks are the primitive workflows use for approvals, input
 * requests, reviews, and generic actions. A `human_task` step creates a
 * task in the assignee's inbox and pauses the flow; completing the task
 * resumes it. Assignees can be users OR AI agents - agents decide tasks
 * automatically (with escalation back to humans), and both complete tasks
 * through the same API.
 *
 * @example
 * ```typescript
 * const inbox = new InboxApi('http://localhost:8081', 'my-repo', authManager);
 *
 * // List my pending tasks
 * const { tasks } = await inbox.listTasks({ status: 'pending' });
 *
 * // Approve the first one
 * await inbox.completeTask(tasks[0].id, {
 *   action: 'approve',
 *   comment: 'Looks good!',
 * });
 * ```
 */

import type { AuthManager } from './auth';
import type { InboxTask, TaskCompletionResult } from './types/flow-definition';
import {
  classifyHttpError,
  RaisinAbortError,
  RaisinTimeoutError,
} from './errors';

/** Options for creating an InboxApi */
export interface InboxApiOptions {
  /** Request timeout in milliseconds (default: 30000) */
  requestTimeout?: number;
  /** Custom fetch implementation */
  fetch?: typeof fetch;
}

/** Filters for listing inbox tasks */
export interface ListTasksOptions {
  /** Filter by status */
  status?: 'pending' | 'completed' | 'expired' | 'cancelled';
  /** List another principal's inbox (admins only) */
  assignee?: string;
  /** Abort signal */
  signal?: AbortSignal;
}

/** Response from listing inbox tasks */
export interface ListTasksResponse {
  /** The inbox owner the tasks belong to */
  assignee: string;
  /** Number of tasks returned */
  count: number;
  /** The tasks, pending first, then by priority and due time */
  tasks: InboxTask[];
}

/**
 * Client for the inbox task endpoints.
 */
export class InboxApi {
  private baseUrl: string;
  private repository: string;
  private authManager: AuthManager;
  private fetchImpl: typeof fetch;
  private requestTimeout: number;

  constructor(
    baseUrl: string,
    repository: string,
    authManager: AuthManager,
    options: InboxApiOptions = {},
  ) {
    this.baseUrl = baseUrl.replace(/\/$/, '');
    this.repository = repository;
    this.authManager = authManager;
    this.fetchImpl = options.fetch ?? fetch;
    this.requestTimeout = options.requestTimeout ?? 30000;
  }

  /**
   * List the caller's inbox tasks.
   *
   * Tasks are sorted pending-first, then by priority (highest first) and
   * due time (earliest first).
   */
  async listTasks(options: ListTasksOptions = {}): Promise<ListTasksResponse> {
    const params = new URLSearchParams();
    if (options.status) params.set('status', options.status);
    if (options.assignee) params.set('assignee', options.assignee);
    const query = params.size > 0 ? `?${params.toString()}` : '';

    return this.request<ListTasksResponse>({
      method: 'GET',
      path: `/api/inbox/${this.repository}${query}`,
      signal: options.signal,
    });
  }

  /**
   * Get a single inbox task by ID (or path).
   */
  async getTask(
    taskId: string,
    options?: { signal?: AbortSignal },
  ): Promise<InboxTask> {
    return this.request<InboxTask>({
      method: 'GET',
      path: `/api/inbox/${this.repository}/tasks/${encodeURIComponent(taskId)}`,
      signal: options?.signal,
    });
  }

  /**
   * Complete an inbox task.
   *
   * The caller must be the task's assignee (or an admin). Completion marks
   * the task done, records who decided, and resumes the owning flow.
   *
   * @param taskId - Task node ID (or path)
   * @param response - The response payload:
   *   - approval tasks: `{ action: '<option value>', comment?: string }`
   *   - input tasks: the input value(s) matching the task's input_schema
   *   - review/action tasks: any acknowledgement payload
   *
   * @example
   * ```typescript
   * await inbox.completeTask('task-123', { action: 'approve' });
   * ```
   */
  async completeTask(
    taskId: string,
    response: unknown,
    options?: { signal?: AbortSignal },
  ): Promise<TaskCompletionResult> {
    return this.request<TaskCompletionResult>({
      method: 'POST',
      path: `/api/inbox/${this.repository}/tasks/${encodeURIComponent(taskId)}/complete`,
      body: { response },
      signal: options?.signal,
    });
  }

  // ==========================================================================
  // Internal HTTP request helper
  // ==========================================================================

  private async request<T>(options: {
    method: string;
    path: string;
    body?: unknown;
    signal?: AbortSignal;
  }): Promise<T> {
    const url = `${this.baseUrl}${options.path}`;
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };

    const token = this.authManager.getAccessToken();
    if (token) {
      headers['Authorization'] = `Bearer ${token}`;
    }

    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), this.requestTimeout);

    if (options.signal) {
      if (options.signal.aborted) {
        clearTimeout(timeoutId);
        throw new RaisinAbortError();
      }
      options.signal.addEventListener('abort', () => controller.abort(), {
        once: true,
      });
    }

    try {
      const response = await this.fetchImpl(url, {
        method: options.method,
        headers,
        body: options.body ? JSON.stringify(options.body) : undefined,
        signal: controller.signal,
      });

      clearTimeout(timeoutId);

      if (!response.ok) {
        const errorText = await response.text();
        let errorMessage: string;
        try {
          const errorJson = JSON.parse(errorText);
          errorMessage = errorJson.message || errorJson.error || errorText;
        } catch {
          errorMessage =
            errorText || `HTTP ${response.status}: ${response.statusText}`;
        }
        throw classifyHttpError(response.status, errorMessage);
      }

      const contentType = response.headers.get('content-type');
      if (contentType?.includes('application/json')) {
        return (await response.json()) as T;
      }
      return undefined as unknown as T;
    } catch (error) {
      clearTimeout(timeoutId);
      if (
        error instanceof RaisinAbortError ||
        error instanceof RaisinTimeoutError
      ) {
        throw error;
      }
      if (error instanceof Error && error.name === 'AbortError') {
        if (options.signal?.aborted) {
          throw new RaisinAbortError();
        }
        throw new RaisinTimeoutError(
          `Request timeout after ${this.requestTimeout}ms`,
          'REQUEST_TIMEOUT',
          this.requestTimeout,
        );
      }
      throw error;
    }
  }
}
