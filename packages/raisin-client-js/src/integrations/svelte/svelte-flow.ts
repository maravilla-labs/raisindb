import { FlowClient } from '../../flow-client';
import type { FlowClientOptions } from '../../flow-client';
import type { AuthManager } from '../../auth';
import type { FlowsApi } from '../../flows';
import type { Database } from '../../database';
import type { FlowExecutionEvent } from '../../types/flow';

export type FlowStatus = 'idle' | 'running' | 'waiting' | 'completed' | 'failed';

export interface FlowSnapshot {
  events: FlowExecutionEvent[];
  status: FlowStatus;
  isRunning: boolean;
  error: string | null;
  output: unknown | null;
  instanceId: string | null;
}

export interface FlowAdapterOptions {
  database?: Database;
  baseUrl?: string;
  repository?: string;
  authManager?: AuthManager;
  clientOptions?: FlowClientOptions;
  flowsApi?: FlowsApi;
}

export interface FlowAdapter {
  subscribe: (cb: (snapshot: FlowSnapshot) => void) => () => void;
  getSnapshot: () => FlowSnapshot;
  run: (flowPath: string, input?: Record<string, unknown>) => Promise<void>;
  resume: (data: unknown) => Promise<void>;
  reset: () => void;
  destroy: () => void;
}

/**
 * Create a flow execution adapter for Svelte 5.
 *
 * Manages the full flow lifecycle: starting, streaming events, resuming
 * waiting flows, and tracking completion. Bind snapshot to `$state` for reactivity.
 *
 * @example
 * ```typescript
 * // lib/flow.svelte.ts
 * import { createFlowAdapter } from '@raisindb/client/svelte';
 * import { db } from '$lib/raisin';
 *
 * const adapter = createFlowAdapter({ database: db });
 * let snapshot = $state(adapter.getSnapshot());
 * adapter.subscribe(s => { snapshot = s; });
 *
 * export const flow = {
 *   get events() { return snapshot.events; },
 *   get status() { return snapshot.status; },
 *   get isRunning() { return snapshot.isRunning; },
 *   get error() { return snapshot.error; },
 *   get output() { return snapshot.output; },
 *   run: adapter.run,
 *   resume: adapter.resume,
 *   reset: adapter.reset,
 * };
 * ```
 */
export function createFlowAdapter(options: FlowAdapterOptions): FlowAdapter {
  let snapshot: FlowSnapshot = {
    events: [],
    status: 'idle',
    isRunning: false,
    error: null,
    output: null,
    instanceId: null,
  };

  const listeners = new Set<(s: FlowSnapshot) => void>();
  let aborted = false;

  function emit() {
    for (const cb of listeners) cb(snapshot);
  }

  function update(partial: Partial<FlowSnapshot>) {
    snapshot = { ...snapshot, ...partial };
    if ('status' in partial) {
      snapshot.isRunning = snapshot.status === 'running';
    }
    emit();
  }

  // Resolve FlowClient
  let flowClient: FlowClient | null = null;
  if (options.database) {
    flowClient = options.database.flow;
  } else if (options.baseUrl && options.repository && options.authManager) {
    flowClient = new FlowClient(
      options.baseUrl,
      options.repository,
      options.authManager,
      options.clientOptions,
      options.flowsApi,
    );
  }

  return {
    subscribe(cb: (s: FlowSnapshot) => void) {
      listeners.add(cb);
      return () => { listeners.delete(cb); };
    },

    getSnapshot: () => snapshot,

    async run(flowPath: string, input: Record<string, unknown> = {}) {
      if (!flowClient) return;

      aborted = false;
      update({
        events: [],
        error: null,
        output: null,
        status: 'running',
      });

      try {
        const result = await flowClient.run(flowPath, input);
        update({ instanceId: result.instance_id });

        for await (const event of flowClient.streamEvents(result.instance_id)) {
          if (aborted) break;

          update({ events: [...snapshot.events, event] });

          if (event.type === 'flow_waiting') {
            update({ status: 'waiting' });
          } else if (event.type === 'flow_completed') {
            update({ output: event.output, status: 'completed' });
          } else if (event.type === 'flow_failed') {
            update({ error: event.error, status: 'failed' });
          }
        }
      } catch (err) {
        update({
          error: err instanceof Error ? err.message : String(err),
          status: 'failed',
        });
      }
    },

    async resume(data: unknown) {
      if (!flowClient || !snapshot.instanceId) return;

      update({ status: 'running', error: null });

      try {
        const stream = await flowClient.createEventStream(snapshot.instanceId);
        await flowClient.resume(snapshot.instanceId, data);

        for await (const event of stream.events) {
          if (aborted) break;

          update({ events: [...snapshot.events, event] });

          if (event.type === 'flow_waiting') {
            update({ status: 'waiting' });
            stream.close();
            return;
          } else if (event.type === 'flow_completed') {
            update({ output: event.output, status: 'completed' });
            stream.close();
            return;
          } else if (event.type === 'flow_failed') {
            update({ error: event.error, status: 'failed' });
            stream.close();
            return;
          }
        }

        stream.close();
      } catch (err) {
        update({
          error: err instanceof Error ? err.message : String(err),
          status: 'failed',
        });
      }
    },

    reset() {
      aborted = true;
      update({
        events: [],
        status: 'idle',
        error: null,
        output: null,
        instanceId: null,
      });
    },

    destroy() {
      aborted = true;
      listeners.clear();
    },
  };
}
