/**
 * FlowsApi - WebSocket-based flow operations.
 *
 * Routes all flow operations through the existing WebSocket connection
 * instead of separate HTTP/SSE requests.
 */

import type { RequestContext, EventMessage } from './protocol';
import { RequestType } from './protocol';
import type { EventHandler } from './events';
import type {
  FlowRunResponse,
  FlowExecutionEvent,
  FlowInstanceStatusResponse,
} from './types/flow';

type SendRequestFn = (
  payload: unknown,
  requestType: string,
  contextOverride?: RequestContext,
) => Promise<unknown>;

export class FlowsApi {
  private context: RequestContext;
  private sendRequest: SendRequestFn;
  private eventHandler: EventHandler;

  constructor(
    repository: string,
    context: RequestContext,
    sendRequest: SendRequestFn,
    eventHandler: EventHandler,
  ) {
    this.context = { ...context, repository };
    this.sendRequest = sendRequest;
    this.eventHandler = eventHandler;
  }

  /** Run a flow by path */
  async run(
    flowPath: string,
    input: Record<string, unknown> = {},
  ): Promise<FlowRunResponse> {
    return (await this.sendRequest(
      { flow_path: flowPath, input },
      RequestType.FlowRun,
      this.context,
    )) as FlowRunResponse;
  }

  /** Get instance status */
  async getInstanceStatus(instanceId: string): Promise<FlowInstanceStatusResponse> {
    return (await this.sendRequest(
      { instance_id: instanceId },
      RequestType.FlowGetInstanceStatus,
      this.context,
    )) as FlowInstanceStatusResponse;
  }

  /** Resume a waiting flow */
  async resume(instanceId: string, data: unknown): Promise<FlowRunResponse> {
    return (await this.sendRequest(
      { instance_id: instanceId, resume_data: data },
      RequestType.FlowResume,
      this.context,
    )) as FlowRunResponse;
  }

  /** Cancel a running flow */
  async cancel(instanceId: string): Promise<void> {
    await this.sendRequest(
      { instance_id: instanceId },
      RequestType.FlowCancel,
      this.context,
    );
  }

  /**
   * Subscribe to flow execution events.
   * Returns an async iterable of FlowExecutionEvent + unsubscribe function.
   * Uses WS EventMessage subscription pattern.
   */
  async subscribeEvents(instanceId: string): Promise<{
    events: AsyncIterable<FlowExecutionEvent>;
    unsubscribe: () => Promise<void>;
  }> {
    // 1. Subscribe on server - get subscription_id back
    const response = (await this.sendRequest(
      { instance_id: instanceId },
      RequestType.FlowSubscribeEvents,
      this.context,
    )) as { subscription_id: string };

    const subscriptionId = response.subscription_id;

    // 2. Create async queue for events
    const queue = createAsyncQueue<FlowExecutionEvent>();

    // 3. Register callback for EventMessages with this subscription_id
    const callback = (event: EventMessage) => {
      const payload = event.payload as FlowExecutionEvent;
      queue.push(payload);
      if (
        payload.type === 'flow_completed' ||
        payload.type === 'flow_failed'
      ) {
        queue.end();
      }
    };

    // Register the subscription_id -> callback in the event handler
    this.eventHandler.addFlowEventListener(subscriptionId, callback);

    return {
      events: queue,
      unsubscribe: async () => {
        this.eventHandler.removeFlowEventListener(subscriptionId);
        queue.end();
        try {
          await this.sendRequest(
            { subscription_id: subscriptionId },
            RequestType.FlowUnsubscribeEvents,
            this.context,
          );
        } catch {
          // Best effort - server may have already cleaned up
        }
      },
    };
  }
}

/**
 * Simple async queue that implements AsyncIterable.
 * Push events from callbacks, consume them with for-await-of.
 */
function createAsyncQueue<T>(): AsyncIterableQueue<T> {
  const buffer: T[] = [];
  let resolve: ((value: IteratorResult<T>) => void) | null = null;
  let done = false;

  return {
    push(item: T) {
      if (done) return;
      if (resolve) {
        const r = resolve;
        resolve = null;
        r({ done: false, value: item });
      } else {
        buffer.push(item);
      }
    },
    end() {
      done = true;
      if (resolve) {
        const r = resolve;
        resolve = null;
        r({ done: true, value: undefined });
      }
    },
    [Symbol.asyncIterator]() {
      return {
        next(): Promise<IteratorResult<T>> {
          if (buffer.length > 0) {
            return Promise.resolve({ done: false, value: buffer.shift()! });
          }
          if (done) {
            return Promise.resolve({ done: true, value: undefined });
          }
          return new Promise<IteratorResult<T>>((r) => {
            resolve = r;
          });
        },
        return(): Promise<IteratorResult<T>> {
          done = true;
          resolve = null;
          buffer.length = 0;
          return Promise.resolve({ done: true, value: undefined });
        },
      };
    },
  };
}

interface AsyncIterableQueue<T> extends AsyncIterable<T> {
  push(item: T): void;
  end(): void;
}
