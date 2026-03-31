/**
 * FlowClient for executing and monitoring RaisinDB flows.
 *
 * Provides methods to:
 * - Run a flow by path and receive an instance ID
 * - Stream real-time execution events via SSE
 * - Resume a waiting flow with external data
 * - Respond to human tasks within a flow
 *
 * @example
 * ```typescript
 * import { RaisinHttpClient } from '@raisindb/client';
 * import { FlowClient } from '@raisindb/client/flow-client';
 *
 * const httpClient = new RaisinHttpClient('http://localhost:8081');
 * await httpClient.authenticate({ username: 'admin', password: 'admin' });
 *
 * const flows = new FlowClient(httpClient, 'my-repo');
 *
 * // Start a flow
 * const { instance_id } = await flows.run('/flows/my-flow', { key: 'value' });
 *
 * // Stream events
 * for await (const event of flows.streamEvents(instance_id)) {
 *   console.log(event.type, event);
 *   if (event.type === 'flow_completed') break;
 * }
 * ```
 */

import type { AuthManager } from './auth';
import type { FlowsApi } from './flows';
import type {
  FlowRunResponse,
  FlowExecutionEvent,
  FlowInstanceStatusResponse,
  FlowCompletedEvent,
  FlowFailedEvent,
} from './types/flow';
import { SSEClient, type SSEEvent } from './streaming/sse-client';
import {
  classifyHttpError,
  RaisinAbortError,
  RaisinTimeoutError,
} from './errors';

/** Options for creating a FlowClient */
export interface FlowClientOptions {
  /** Request timeout in milliseconds (default: 60000 for long-running flows) */
  requestTimeout?: number;
  /** Custom fetch implementation */
  fetch?: typeof fetch;
}

/** Result from `runAndWait()` */
export interface FlowRunResult {
  /** Flow instance ID */
  instanceId: string;
  /** Terminal status */
  status: 'completed' | 'failed';
  /** Flow output (when completed) */
  output?: unknown;
  /** Error message (when failed) */
  error?: string;
}

/** Result from `runAndCollect()` */
export interface FlowCollectResult {
  /** Flow instance ID */
  instanceId: string;
  /** All collected events */
  events: FlowExecutionEvent[];
}

/**
 * Minimal interface the FlowClient needs from the HTTP client.
 * This avoids a hard dependency on the full RaisinHttpClient class.
 */
interface HttpClientLike {
  /** Get the auth manager for token access */
  getAuthManager(): AuthManager;
}

/**
 * Client for executing and monitoring RaisinDB flows.
 *
 * Uses the HTTP REST API for flow operations and SSE for real-time event streaming.
 */
export class FlowClient {
  private baseUrl: string;
  private repository: string;
  private authManager: AuthManager;
  private fetchImpl: typeof fetch;
  private requestTimeout: number;
  private flowsApi?: FlowsApi;

  /**
   * Create a new FlowClient.
   *
   * @param baseUrl - Base HTTP URL of the RaisinDB server (e.g., "http://localhost:8081")
   * @param repository - Repository name
   * @param authManager - Auth manager for token access
   * @param options - Additional options
   * @param flowsApi - Optional FlowsApi for WebSocket-based flow operations
   */
  constructor(
    baseUrl: string,
    repository: string,
    authManager: AuthManager,
    options: FlowClientOptions = {},
    flowsApi?: FlowsApi,
  ) {
    this.baseUrl = baseUrl.replace(/\/$/, '');
    this.repository = repository;
    this.authManager = authManager;
    this.fetchImpl = options.fetch ?? fetch;
    this.requestTimeout = options.requestTimeout ?? 60000;
    this.flowsApi = flowsApi;
  }

  /**
   * Create a FlowClient from an existing RaisinHttpClient.
   *
   * @param httpClient - An authenticated RaisinHttpClient instance
   * @param baseUrl - Base HTTP URL (same as the one used for the HTTP client)
   * @param repository - Repository name
   * @param options - Additional options
   */
  static fromHttpClient(
    httpClient: HttpClientLike,
    baseUrl: string,
    repository: string,
    options: FlowClientOptions = {},
    flowsApi?: FlowsApi,
  ): FlowClient {
    return new FlowClient(
      baseUrl,
      repository,
      httpClient.getAuthManager(),
      options,
      flowsApi,
    );
  }

  // ==========================================================================
  // Flow Execution
  // ==========================================================================

  /**
   * Execute a flow by path.
   *
   * Creates a FlowInstance and queues it for execution. Returns immediately
   * with the instance ID - use `streamEvents()` to monitor progress.
   *
   * @param flowPath - Path to the raisin:Flow node (e.g., "/flows/my-flow")
   * @param input - Input data passed to the flow
   * @returns Flow run response with instance_id and job_id
   *
   * @example
   * ```typescript
   * const result = await flows.run('/flows/process-order', {
   *   orderId: 'ord-123',
   *   items: [{ sku: 'ABC', qty: 2 }],
   * });
   * console.log('Instance:', result.instance_id);
   * ```
   */
  async run(
    flowPath: string,
    input: Record<string, unknown> = {},
    options?: { signal?: AbortSignal },
  ): Promise<FlowRunResponse> {
    if (this.flowsApi) {
      return this.flowsApi.run(flowPath, input);
    }
    const response = await this.request<FlowRunResponse>({
      method: 'POST',
      path: `/api/flows/${this.repository}/run`,
      body: { flow_path: flowPath, input },
      signal: options?.signal,
    });
    return response;
  }

  // ==========================================================================
  // Instance Status
  // ==========================================================================

  /**
   * Get the current status of a flow instance.
   *
   * Returns the instance status, variables, and metadata. Useful for
   * polling-based approaches where SSE is not reliable (e.g., initial
   * flow execution where events may be missed).
   *
   * @param instanceId - Flow instance ID (from `run()`)
   * @returns Instance status including variables and flow path
   *
   * @example
   * ```typescript
   * const status = await flows.getInstanceStatus('instance-123');
   * console.log('Status:', status.status); // 'waiting', 'completed', etc.
   * ```
   */
  async getInstanceStatus(
    instanceId: string,
    options?: { signal?: AbortSignal },
  ): Promise<FlowInstanceStatusResponse> {
    if (this.flowsApi) {
      return this.flowsApi.getInstanceStatus(instanceId);
    }
    return this.request<FlowInstanceStatusResponse>({
      method: 'GET',
      path: `/api/flows/${this.repository}/instances/${instanceId}`,
      signal: options?.signal,
    });
  }

  // ==========================================================================
  // Event Streaming (SSE)
  // ==========================================================================

  /**
   * Stream real-time execution events for a flow instance via SSE.
   *
   * Returns an async iterable that yields FlowExecutionEvent objects.
   * The stream automatically closes when a terminal event is received
   * (flow_completed or flow_failed).
   *
   * @param instanceId - Flow instance ID (from `run()`)
   * @returns Async iterable of flow execution events
   *
   * @example
   * ```typescript
   * const { instance_id } = await flows.run('/flows/my-flow', { key: 'value' });
   *
   * for await (const event of flows.streamEvents(instance_id)) {
   *   switch (event.type) {
   *     case 'step_started':
   *       console.log(`Step ${event.node_id} started`);
   *       break;
   *     case 'step_completed':
   *       console.log(`Step ${event.node_id} completed in ${event.duration_ms}ms`);
   *       break;
   *     case 'text_chunk':
   *       process.stdout.write(event.text);
   *       break;
   *     case 'flow_completed':
   *       console.log('Flow done:', event.output);
   *       break;
   *     case 'flow_failed':
   *       console.error('Flow failed:', event.error);
   *       break;
   *   }
   * }
   * ```
   */
  async *streamEvents(
    instanceId: string,
    options?: { signal?: AbortSignal },
  ): AsyncIterable<FlowExecutionEvent> {
    if (this.flowsApi) {
      const sub = await this.flowsApi.subscribeEvents(instanceId);
      try {
        for await (const event of sub.events) {
          if (options?.signal?.aborted) throw new RaisinAbortError();
          yield event;
          if (event.type === 'flow_completed' || event.type === 'flow_failed') {
            return;
          }
        }
      } finally {
        await sub.unsubscribe();
      }
      return;
    }
    const url = `${this.baseUrl}/api/flows/${this.repository}/instances/${instanceId}/events`;
    const headers: Record<string, string> = {};

    const token = this.authManager.getAccessToken();
    if (token) {
      headers['Authorization'] = `Bearer ${token}`;
    }

    const sse = new SSEClient<FlowExecutionEvent>(url, {
      headers,
      eventTypes: ['flow-event', 'message'],
      reconnect: { enabled: false },
      fetch: this.fetchImpl,
      signal: options?.signal,
    });

    try {
      for await (const sseEvent of sse) {
        const event = sseEvent.data;
        yield event;

        // Stop on terminal events
        if (
          event.type === 'flow_completed' ||
          event.type === 'flow_failed'
        ) {
          return;
        }
      }
    } finally {
      sse.close();
    }
  }

  /**
   * Create an eagerly-connected event stream for a flow instance.
   *
   * Unlike `streamEvents()` (a lazy async generator), this method opens the
   * SSE connection immediately and waits until it is established before
   * returning. This is critical for resume flows: you must subscribe to
   * events BEFORE calling `resume()`, otherwise events emitted between
   * the resume and the SSE connection are lost.
   *
   * @param instanceId - Flow instance ID
   * @returns Object with an async iterable `events` and a `close()` method
   */
  async createEventStream(
    instanceId: string,
    options?: { signal?: AbortSignal },
  ): Promise<{
    events: AsyncIterable<FlowExecutionEvent>;
    close: () => void;
  }> {
    if (this.flowsApi) {
      const sub = await this.flowsApi.subscribeEvents(instanceId);
      return {
        events: sub.events,
        close: () => { sub.unsubscribe(); },
      };
    }
    const url = `${this.baseUrl}/api/flows/${this.repository}/instances/${instanceId}/events`;
    const headers: Record<string, string> = {};

    const token = this.authManager.getAccessToken();
    if (token) {
      headers['Authorization'] = `Bearer ${token}`;
    }

    const sse = new SSEClient<FlowExecutionEvent>(url, {
      headers,
      eventTypes: ['flow-event', 'message'],
      reconnect: { enabled: false },
      fetch: this.fetchImpl,
      signal: options?.signal,
    });

    // Kick-start the async generator to initiate the SSE connection.
    // Async generators are lazy — the body (which calls startStream)
    // only executes when .next() is called.
    const iterator = sse[Symbol.asyncIterator]();
    let pendingFirst: Promise<IteratorResult<SSEEvent<FlowExecutionEvent>>> | null = iterator.next();

    // Now that startStream is running, wait for the HTTP response
    await sse.waitUntilConnected();

    // Wrap in a terminal-event-aware iterable
    const events: AsyncIterable<FlowExecutionEvent> = {
      [Symbol.asyncIterator]() {
        return {
          async next() {
            let result;
            if (pendingFirst) {
              result = await pendingFirst;
              pendingFirst = null;
            } else {
              result = await iterator.next();
            }
            if (result.done) return { done: true as const, value: undefined };
            const event = result.value.data;
            return { done: false, value: event };
          },
          async return() {
            await iterator.return?.();
            sse.close();
            return { done: true as const, value: undefined };
          },
        };
      },
    };

    return { events, close: () => sse.close() };
  }

  // ==========================================================================
  // Flow Resume & Human Task
  // ==========================================================================

  /**
   * Resume a flow instance that is in a waiting state.
   *
   * Use this when a flow is waiting for external input (tool results,
   * external events, etc.). The resume data is passed to the waiting step.
   *
   * @param instanceId - Flow instance ID
   * @param data - Resume data to pass to the waiting step
   *
   * @example
   * ```typescript
   * await flows.resume('instance-123', {
   *   tool_name: 'get_weather',
   *   result: { temperature: 72, conditions: 'sunny' },
   * });
   * ```
   */
  async resume(
    instanceId: string,
    data: unknown,
    options?: { signal?: AbortSignal },
  ): Promise<void> {
    if (this.flowsApi) {
      await this.flowsApi.resume(instanceId, data);
      return;
    }
    await this.request<void>({
      method: 'POST',
      path: `/api/flows/${this.repository}/instances/${instanceId}/resume`,
      body: { resume_data: data },
      signal: options?.signal,
    });
  }

  /**
   * Respond to a human task within a flow.
   *
   * When a flow is waiting on a human_task step (approval, input, review),
   * use this method to submit the response and resume the flow.
   *
   * @param instanceId - Flow instance ID
   * @param taskId - The human task node ID
   * @param response - The human's response data
   *
   * @example
   * ```typescript
   * // Approve a task
   * await flows.respondToHumanTask('instance-123', 'step-5', {
   *   action: 'approve',
   *   comment: 'Looks good!',
   * });
   *
   * // Provide input for an input task
   * await flows.respondToHumanTask('instance-123', 'step-3', {
   *   name: 'John Doe',
   *   email: 'john@example.com',
   * });
   * ```
   */
  async respondToHumanTask(
    instanceId: string,
    taskId: string,
    response: unknown,
    options?: { signal?: AbortSignal },
  ): Promise<void> {
    await this.request<void>({
      method: 'POST',
      path: `/api/flows/${this.repository}/instances/${instanceId}/tasks/${taskId}/respond`,
      body: { response },
      signal: options?.signal,
    });
  }

  // ==========================================================================
  // Convenience Methods
  // ==========================================================================

  /**
   * Run a flow and wait for the final result.
   *
   * Starts the flow, streams all events, and returns the terminal outcome.
   * This is a one-shot helper for fire-and-forget flows where you only
   * care about the final output.
   *
   * @param flowPath - Path to the raisin:Flow node
   * @param input - Input data passed to the flow
   * @param options - Optional abort signal
   * @returns The flow outcome including output or error
   *
   * @example
   * ```typescript
   * const result = await flows.runAndWait('/flows/process-order', { orderId: '123' });
   * if (result.status === 'completed') {
   *   console.log('Output:', result.output);
   * }
   * ```
   */
  async runAndWait(
    flowPath: string,
    input: Record<string, unknown> = {},
    options?: { signal?: AbortSignal },
  ): Promise<FlowRunResult> {
    const { instance_id } = await this.run(flowPath, input, options);

    for await (const event of this.streamEvents(instance_id, options)) {
      if (event.type === 'flow_completed') {
        return {
          instanceId: instance_id,
          status: 'completed',
          output: (event as FlowCompletedEvent).output,
        };
      }
      if (event.type === 'flow_failed') {
        return {
          instanceId: instance_id,
          status: 'failed',
          error: (event as FlowFailedEvent).error,
        };
      }
    }

    return { instanceId: instance_id, status: 'failed', error: 'Stream ended without terminal event' };
  }

  /**
   * Run a flow and collect all events into an array.
   *
   * Starts the flow, streams all events, and returns them as a collected array.
   * Useful for testing, logging, or post-processing all flow events.
   *
   * @param flowPath - Path to the raisin:Flow node
   * @param input - Input data passed to the flow
   * @param options - Optional abort signal
   * @returns The instance ID and all collected events
   *
   * @example
   * ```typescript
   * const { events } = await flows.runAndCollect('/flows/etl-pipeline', { source: 'db' });
   * const steps = events.filter(e => e.type === 'step_completed');
   * console.log(`Completed ${steps.length} steps`);
   * ```
   */
  async runAndCollect(
    flowPath: string,
    input: Record<string, unknown> = {},
    options?: { signal?: AbortSignal },
  ): Promise<FlowCollectResult> {
    const { instance_id } = await this.run(flowPath, input, options);
    const events: FlowExecutionEvent[] = [];

    for await (const event of this.streamEvents(instance_id, options)) {
      events.push(event);
    }

    return { instanceId: instance_id, events };
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

    // Forward external signal to our internal controller
    if (options.signal) {
      if (options.signal.aborted) {
        clearTimeout(timeoutId);
        throw new RaisinAbortError();
      }
      options.signal.addEventListener('abort', () => controller.abort(), { once: true });
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
      // Re-throw our own errors as-is
      if (error instanceof RaisinAbortError || error instanceof RaisinTimeoutError) {
        throw error;
      }
      if (error instanceof Error && error.name === 'AbortError') {
        // Distinguish external abort from internal timeout
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
