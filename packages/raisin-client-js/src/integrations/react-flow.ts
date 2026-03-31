/**
 * React flow adapter for RaisinDB.
 *
 * Provides a `useFlow` hook that wraps the FlowClient for executing
 * and monitoring RaisinDB flows in React applications. Returns reactive
 * state tracking flow events, status, and providing control actions.
 *
 * Since this SDK package does not depend on React, the hook accepts
 * React as a parameter via the `ReactLike` interface.
 *
 * @example
 * ```tsx
 * import React from 'react';
 * import { useFlow } from '@raisindb/client';
 *
 * function OrderProcessor() {
 *   const flow = useFlow(React, {
 *     baseUrl: 'http://localhost:8081',
 *     repository: 'my-repo',
 *     authManager,
 *   });
 *
 *   const handleRun = async () => {
 *     await flow.run('/flows/process-order', { orderId: 'ord-123' });
 *   };
 *
 *   return (
 *     <div>
 *       <button onClick={handleRun} disabled={flow.isRunning}>Run</button>
 *       <p>Status: {flow.status}</p>
 *       {flow.events.map((e, i) => <div key={i}>{e.type}</div>)}
 *       {flow.error && <p className="error">{flow.error}</p>}
 *     </div>
 *   );
 * }
 * ```
 */

import { FlowClient } from '../flow-client';
import type { FlowClientOptions } from '../flow-client';
import type { AuthManager } from '../auth';
import type { FlowsApi } from '../flows';
import type { Database } from '../database';
import type { FlowExecutionEvent } from '../types/flow';
import type { ReactLike } from './react-conversation';

/** Options for creating the FlowClient within useFlow */
export interface UseFlowOptions {
  /**
   * Pre-configured Database instance (from `client.database()`).
   * When provided, `baseUrl`, `repository`, `authManager`, and `flowsApi` are derived
   * automatically — no manual wiring needed.
   */
  database?: Database;
  /** Base HTTP URL of the RaisinDB server */
  baseUrl?: string;
  /** Repository name */
  repository?: string;
  /** Auth manager for token access */
  authManager?: AuthManager;
  /** Additional FlowClient options */
  clientOptions?: FlowClientOptions;
  /** Optional FlowsApi for WebSocket-based flow operations */
  flowsApi?: FlowsApi;
}

/** Flow execution status */
export type FlowStatus = 'idle' | 'running' | 'waiting' | 'completed' | 'failed';

/** Return value from the useFlow hook */
export interface UseFlowReturn {
  /** Collected flow execution events */
  events: FlowExecutionEvent[];
  /** Current flow status */
  status: FlowStatus;
  /** Whether a flow is currently running */
  isRunning: boolean;
  /** Current error, if any */
  error: string | null;
  /** Output from the completed flow */
  output: unknown | null;
  /** Current flow instance ID, if running */
  instanceId: string | null;
  /** Run a flow by path with optional input data */
  run: (flowPath: string, input?: Record<string, unknown>) => Promise<void>;
  /** Resume a waiting flow with data */
  resume: (data: unknown) => Promise<void>;
  /** Reset the hook state (clear events, status, error) */
  reset: () => void;
}

/**
 * React hook for executing and monitoring RaisinDB flows.
 *
 * Manages the full lifecycle: starting a flow, streaming events,
 * resuming waiting flows, and tracking completion status.
 * Auto-cleans up on component unmount.
 *
 * @param react - The React instance (pass `React` or `{ useState, useEffect, useRef, useCallback }`)
 * @param options - Flow client options (baseUrl, repository, authManager, etc.)
 * @returns Reactive flow state and action functions
 *
 * @example
 * ```tsx
 * import React from 'react';
 * import { useFlow } from '@raisindb/client';
 *
 * function DataPipeline() {
 *   const flow = useFlow(React, {
 *     baseUrl: 'http://localhost:8081',
 *     repository: 'my-repo',
 *     authManager,
 *   });
 *
 *   return (
 *     <div>
 *       <button onClick={() => flow.run('/flows/etl', { source: 'api' })}>
 *         Start Pipeline
 *       </button>
 *       <p>Status: {flow.status}</p>
 *       <p>Events: {flow.events.length}</p>
 *       {flow.status === 'waiting' && (
 *         <button onClick={() => flow.resume({ approved: true })}>
 *           Approve
 *         </button>
 *       )}
 *       {flow.output && <pre>{JSON.stringify(flow.output, null, 2)}</pre>}
 *     </div>
 *   );
 * }
 * ```
 */
export function useFlow(react: ReactLike, options: UseFlowOptions): UseFlowReturn {
  const { useState, useEffect, useRef, useCallback } = react;

  const [events, setEvents] = useState<FlowExecutionEvent[]>([]);
  const [status, setStatus] = useState<FlowStatus>('idle');
  const [error, setError] = useState<string | null>(null);
  const [output, setOutput] = useState<unknown | null>(null);
  const [instanceId, setInstanceId] = useState<string | null>(null);

  const clientRef = useRef<FlowClient | null>(null);
  const abortRef = useRef(false);

  // Create the FlowClient once
  useEffect(() => {
    if (options.database) {
      clientRef.current = options.database.flow;
    } else if (options.baseUrl && options.repository && options.authManager) {
      clientRef.current = new FlowClient(
        options.baseUrl,
        options.repository,
        options.authManager,
        options.clientOptions,
        options.flowsApi,
      );
    }

    return () => {
      abortRef.current = true;
      clientRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [options.database, options.baseUrl, options.repository]);

  const run = useCallback(async (flowPath: string, input: Record<string, unknown> = {}) => {
    const client = clientRef.current;
    if (!client) return;

    // Reset state
    abortRef.current = false;
    setEvents([]);
    setError(null);
    setOutput(null);
    setStatus('running');

    try {
      const result = await client.run(flowPath, input);
      setInstanceId(result.instance_id);

      for await (const event of client.streamEvents(result.instance_id)) {
        if (abortRef.current) break;

        setEvents((prev) => [...prev, event]);

        if (event.type === 'flow_waiting') {
          setStatus('waiting');
        } else if (event.type === 'flow_completed') {
          setOutput(event.output);
          setStatus('completed');
        } else if (event.type === 'flow_failed') {
          setError(event.error);
          setStatus('failed');
        }
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setStatus('failed');
    }
  }, []);

  const resume = useCallback(async (data: unknown) => {
    const client = clientRef.current;
    if (!client || !instanceId) return;

    setStatus('running');
    setError(null);

    try {
      // Open event stream before resuming to avoid missing events
      const stream = await client.createEventStream(instanceId);
      await client.resume(instanceId, data);

      for await (const event of stream.events) {
        if (abortRef.current) break;

        setEvents((prev) => [...prev, event]);

        if (event.type === 'flow_waiting') {
          setStatus('waiting');
          stream.close();
          return;
        } else if (event.type === 'flow_completed') {
          setOutput(event.output);
          setStatus('completed');
          stream.close();
          return;
        } else if (event.type === 'flow_failed') {
          setError(event.error);
          setStatus('failed');
          stream.close();
          return;
        }
      }

      stream.close();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setStatus('failed');
    }
  }, [instanceId]);

  const reset = useCallback(() => {
    abortRef.current = true;
    setEvents([]);
    setStatus('idle');
    setError(null);
    setOutput(null);
    setInstanceId(null);
  }, []);

  return {
    events,
    status,
    isRunning: status === 'running',
    error,
    output,
    instanceId,
    run,
    resume,
    reset,
  };
}
