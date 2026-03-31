/**
 * SSE (Server-Sent Events) client for streaming flow execution events.
 *
 * Uses fetch-based approach for better control over connection lifecycle,
 * headers, and reconnection compared to the native EventSource API.
 */

import { logger } from '../logger';
import { ReconnectManager, type ReconnectOptions } from '../utils/reconnect';
import { RaisinConnectionError, RaisinAuthError, RaisinAbortError } from '../errors';

/**
 * Parsed SSE event from the wire format
 */
export interface SSEEvent<T = unknown> {
  /** SSE event type (from "event:" field) */
  type: string;
  /** Parsed event data */
  data: T;
  /** SSE event ID (from "id:" field, if present) */
  id?: string;
}

/**
 * SSE connection state
 */
export type SSEConnectionState = 'disconnected' | 'connecting' | 'connected' | 'reconnecting';

/**
 * Callback for SSE connection state changes
 */
export type SSEStateCallback = (state: SSEConnectionState) => void;

/**
 * Options for creating an SSE client
 */
export interface SSEClientOptions {
  /** HTTP headers to include (e.g., Authorization) */
  headers?: Record<string, string>;
  /** Event types to filter for (if empty, all events are passed through) */
  eventTypes?: string[];
  /** Reconnection options (uses ReconnectManager) */
  reconnect?: ReconnectOptions & {
    /** Whether to automatically reconnect on disconnect (default: true) */
    enabled?: boolean;
  };
  /** Custom fetch implementation */
  fetch?: typeof fetch;
  /** Connection state change callback */
  onStateChange?: SSEStateCallback;
  /** Last-Event-ID for resuming streams */
  lastEventId?: string;
  /** External AbortSignal for cancellation */
  signal?: AbortSignal;
  /** HTTP method (default: 'GET'). Use 'POST' to avoid path in URL params. */
  method?: 'GET' | 'POST';
  /** Request body for POST requests (JSON-serialized automatically) */
  body?: unknown;
}

/**
 * Parse a single SSE message block into event type, data, and id.
 *
 * SSE wire format:
 *   event: <type>\n
 *   data: <json>\n
 *   id: <id>\n
 *   \n
 */
function parseSSEBlock(block: string): { event: string; data: string; id?: string } | null {
  let event = 'message';
  let data = '';
  let id: string | undefined;

  const lines = block.split('\n');
  for (const line of lines) {
    if (line.startsWith('event:')) {
      event = line.slice(6).trim();
    } else if (line.startsWith('data:')) {
      // Accumulate data lines (SSE spec allows multi-line data)
      if (data) data += '\n';
      data += line.slice(5).trim();
    } else if (line.startsWith('id:')) {
      id = line.slice(3).trim();
    }
    // Lines starting with ":" are comments (keep-alive), ignore them
    // Empty lines are block separators, handled by the caller
  }

  if (!data) return null;

  return { event, data, id };
}

/**
 * Fetch-based SSE client with reconnection, filtering, and AsyncIterable support.
 *
 * @example
 * ```typescript
 * const sse = new SSEClient('https://api.example.com/events/flow/123', {
 *   headers: { Authorization: `Bearer ${token}` },
 *   eventTypes: ['step_started', 'step_completed', 'flow_completed'],
 * });
 *
 * // AsyncIterable usage
 * for await (const event of sse) {
 *   console.log(event.type, event.data);
 * }
 *
 * // Callback usage
 * sse.connect((event) => {
 *   console.log(event.type, event.data);
 * });
 *
 * // Cleanup
 * sse.close();
 * ```
 */
export class SSEClient<T = unknown> implements AsyncIterable<SSEEvent<T>> {
  private url: string;
  private options: SSEClientOptions;
  private abortController: AbortController | null = null;
  private reconnectManager: ReconnectManager;
  private fetchImpl: typeof fetch;
  private _state: SSEConnectionState = 'disconnected';
  private _lastEventId?: string;
  private eventTypeSet: Set<string> | null;
  private autoReconnect: boolean;
  private externalSignal?: AbortSignal;

  constructor(url: string, options: SSEClientOptions = {}) {
    this.url = url;
    this.options = options;
    this.fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
    this._lastEventId = options.lastEventId;
    this.autoReconnect = options.reconnect?.enabled ?? true;
    this.externalSignal = options.signal;
    this.eventTypeSet = options.eventTypes?.length
      ? new Set(options.eventTypes)
      : null;

    this.reconnectManager = new ReconnectManager({
      initialDelay: options.reconnect?.initialDelay ?? 1000,
      maxDelay: options.reconnect?.maxDelay ?? 30000,
      backoffMultiplier: options.reconnect?.backoffMultiplier ?? 2,
      maxAttempts: options.reconnect?.maxAttempts ?? Infinity,
    });
  }

  /** Current connection state */
  get state(): SSEConnectionState {
    return this._state;
  }

  /**
   * Returns a promise that resolves once the SSE connection is established.
   *
   * Useful for ensuring the connection is ready before triggering events
   * on the server side (e.g., calling resume before consuming the stream).
   * If the client is already connected, resolves immediately.
   */
  waitUntilConnected(): Promise<void> {
    if (this._state === 'connected') return Promise.resolve();
    return new Promise<void>((resolve, reject) => {
      const originalOnState = this.options.onStateChange;
      this.options.onStateChange = (state) => {
        originalOnState?.(state);
        if (state === 'connected') {
          // Restore original callback and resolve
          this.options.onStateChange = originalOnState;
          resolve();
        } else if (state === 'disconnected') {
          this.options.onStateChange = originalOnState;
          reject(new Error('SSE connection failed'));
        }
      };
    });
  }

  /** Last received event ID */
  get lastEventId(): string | undefined {
    return this._lastEventId;
  }

  private setState(state: SSEConnectionState): void {
    if (this._state !== state) {
      this._state = state;
      this.options.onStateChange?.(state);
    }
  }

  /**
   * Connect and consume events via callback.
   *
   * @param onEvent - Called for each matching SSE event
   * @param onError - Called when an error occurs (optional)
   */
  async connect(
    onEvent: (event: SSEEvent<T>) => void,
    onError?: (error: Error) => void,
  ): Promise<void> {
    this.close();
    await this.startStream(onEvent, onError);
  }

  /**
   * Close the SSE connection and stop reconnection attempts.
   */
  close(): void {
    this.reconnectManager.cancelReconnect();
    if (this.abortController) {
      this.abortController.abort();
      this.abortController = null;
    }
    this.setState('disconnected');
  }

  /**
   * AsyncIterable interface: allows `for await (const event of sseClient)`.
   *
   * The iterator ends when close() is called or the server closes the stream
   * (and auto-reconnect is disabled or max attempts are exhausted).
   */
  async *[Symbol.asyncIterator](): AsyncIterator<SSEEvent<T>> {
    // Use a queue and resolve-based approach for backpressure
    type QueueItem =
      | { done: false; value: SSEEvent<T> }
      | { done: true; error?: Error };

    const queue: QueueItem[] = [];
    let resolve: ((item: QueueItem) => void) | null = null;
    let finished = false;

    const push = (item: QueueItem) => {
      if (finished) return;
      if (resolve) {
        const r = resolve;
        resolve = null;
        r(item);
      } else {
        queue.push(item);
      }
    };

    const pull = (): Promise<QueueItem> => {
      const queued = queue.shift();
      if (queued) return Promise.resolve(queued);
      return new Promise<QueueItem>((r) => {
        resolve = r;
      });
    };

    // Start streaming in the background
    this.startStream(
      (event) => push({ done: false, value: event }),
      (error) => push({ done: true, error }),
    ).then(() => {
      push({ done: true });
    }).catch((error) => {
      push({ done: true, error });
    });

    try {
      while (!finished) {
        const item = await pull();
        if (item.done) {
          finished = true;
          if (item.error) {
            throw item.error;
          }
          return;
        }
        yield item.value;
      }
    } finally {
      finished = true;
      this.close();
    }
  }

  /**
   * Core streaming loop: connects, reads the SSE stream, and handles reconnection.
   */
  private async startStream(
    onEvent: (event: SSEEvent<T>) => void,
    onError?: (error: Error) => void,
  ): Promise<void> {
    while (true) {
      try {
        await this.readStream(onEvent);

        // Stream ended cleanly (server closed connection)
        if (!this.autoReconnect || this._state === 'disconnected') {
          return;
        }

        // Schedule reconnection
        this.setState('reconnecting');
        const scheduled = await this.waitForReconnect();
        if (!scheduled) {
          // Max attempts reached
          logger.warn('SSE max reconnection attempts reached');
          this.setState('disconnected');
          return;
        }
      } catch (error) {
        if (this._state === 'disconnected') {
          // close() was called, exit cleanly
          return;
        }

        const err = error instanceof Error ? error : new Error(String(error));

        // AbortError means close() was called or external AbortSignal fired
        if (err.name === 'AbortError' || err instanceof RaisinAbortError) {
          return;
        }

        logger.error('SSE connection error:', err.message);
        onError?.(err);

        if (!this.autoReconnect) {
          this.setState('disconnected');
          throw err;
        }

        // Schedule reconnection
        this.setState('reconnecting');
        const scheduled = await this.waitForReconnect();
        if (!scheduled) {
          logger.warn('SSE max reconnection attempts reached');
          this.setState('disconnected');
          throw err;
        }
      }
    }
  }

  /**
   * Perform a single fetch and read the response body as SSE stream.
   */
  private async readStream(onEvent: (event: SSEEvent<T>) => void): Promise<void> {
    this.abortController = new AbortController();
    this.setState('connecting');

    // If an external signal is provided, forward its abort to our internal controller
    if (this.externalSignal) {
      if (this.externalSignal.aborted) {
        this.abortController.abort();
      } else {
        const ctrl = this.abortController;
        this.externalSignal.addEventListener('abort', () => ctrl.abort(), { once: true });
      }
    }

    const headers: Record<string, string> = {
      Accept: 'text/event-stream',
      'Cache-Control': 'no-cache',
      ...this.options.headers,
    };

    // Resume from last event ID if available
    if (this._lastEventId) {
      headers['Last-Event-ID'] = this._lastEventId;
    }

    const method = this.options.method ?? 'GET';
    const fetchOptions: RequestInit = {
      method,
      headers,
      signal: this.abortController.signal,
    };
    if (method === 'POST' && this.options.body !== undefined) {
      (fetchOptions.headers as Record<string, string>)['Content-Type'] = 'application/json';
      fetchOptions.body = JSON.stringify(this.options.body);
    }

    const response = await this.fetchImpl(this.url, fetchOptions);

    if (!response.ok) {
      const text = await response.text().catch(() => '');
      const msg = `SSE connection failed: ${response.status} ${response.statusText}${text ? ` - ${text}` : ''}`;
      if (response.status === 401 || response.status === 403) {
        throw new RaisinAuthError(
          msg,
          response.status === 401 ? 'AUTH_UNAUTHORIZED' : 'AUTH_FORBIDDEN',
          response.status,
        );
      }
      throw new RaisinConnectionError(msg, 'SSE_STREAM_ERROR', { status: response.status });
    }

    if (!response.body) {
      throw new RaisinConnectionError('SSE response has no body', 'SSE_STREAM_ERROR');
    }

    this.setState('connected');
    this.reconnectManager.reset();

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });

        // SSE events are separated by double newlines
        const blocks = buffer.split('\n\n');
        // Last element is incomplete, keep it in the buffer
        buffer = blocks.pop() ?? '';

        for (const block of blocks) {
          const trimmed = block.trim();
          if (!trimmed) continue;

          const parsed = parseSSEBlock(trimmed);
          if (!parsed) continue;

          // Track event ID for reconnection
          if (parsed.id) {
            this._lastEventId = parsed.id;
          }

          // Apply event type filter
          if (this.eventTypeSet && !this.eventTypeSet.has(parsed.event)) {
            continue;
          }

          // Parse JSON data
          let data: T;
          try {
            data = JSON.parse(parsed.data) as T;
          } catch {
            // If data isn't valid JSON, pass it as-is
            data = parsed.data as unknown as T;
          }

          const sseEvent = {
            type: parsed.event,
            data,
            id: parsed.id,
          };
          onEvent(sseEvent);
        }
      }
    } finally {
      reader.releaseLock();
    }
  }

  /**
   * Wait for the reconnect manager to schedule a reconnection attempt.
   * Returns true if reconnection was scheduled, false if max attempts reached.
   */
  private waitForReconnect(): Promise<boolean> {
    return new Promise<boolean>((resolve) => {
      const scheduled = this.reconnectManager.scheduleReconnect(() => {
        resolve(true);
      });
      if (!scheduled) {
        resolve(false);
      }
    });
  }
}
