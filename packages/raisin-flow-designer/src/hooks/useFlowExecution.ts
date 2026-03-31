/**
 * useFlowExecution Hook
 *
 * Manages flow execution state and SSE event subscription for real-time
 * step-level visualization in the flow designer canvas.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type {
  FlowExecutionState,
  FlowExecutionEvent,
  StepExecutionInfo,
  ExecutionLogEntry,
} from '../types/flow';
import { INITIAL_EXECUTION_STATE } from '../types/flow';

export interface UseFlowExecutionOptions {
  /** Repository ID */
  repoId: string;
  /** Base API URL (defaults to current origin) */
  baseUrl?: string;
  /** Auto-reconnect on disconnect */
  autoReconnect?: boolean;
  /** Reconnect delay in ms */
  reconnectDelay?: number;
}

export interface UseFlowExecutionResult {
  /** Current execution state */
  executionState: FlowExecutionState;
  /** Whether currently connected to SSE stream */
  isConnected: boolean;
  /** Start executing a flow */
  executeFlow: (flowPath: string, input?: Record<string, unknown>) => Promise<string>;
  /** Start a test run with mocking */
  executeTestRun: (
    flowPath: string,
    input?: Record<string, unknown>,
    testConfig?: TestRunConfig
  ) => Promise<string>;
  /** Subscribe to an existing flow instance */
  subscribeToInstance: (instanceId: string) => void;
  /** Disconnect from SSE stream */
  disconnect: () => void;
  /** Reset execution state */
  reset: () => void;
  /** Get execution state for a specific node */
  getNodeState: (nodeId: string) => StepExecutionInfo | undefined;
}

export interface TestRunConfig {
  is_test_run: boolean;
  isolated_branch?: boolean;
  auto_discard?: boolean;
  mock_functions?: Record<string, MockFunctionConfig>;
}

export interface MockFunctionConfig {
  behavior: 'passthrough' | 'mock_output' | 'error';
  mock_response?: unknown;
  error_message?: string;
}

/**
 * Hook for managing flow execution and real-time event streaming
 */
export function useFlowExecution(options: UseFlowExecutionOptions): UseFlowExecutionResult {
  const { repoId, baseUrl = '', autoReconnect = true, reconnectDelay = 3000 } = options;

  const [executionState, setExecutionState] = useState<FlowExecutionState>(INITIAL_EXECUTION_STATE);
  const [isConnected, setIsConnected] = useState(false);

  const eventSourceRef = useRef<EventSource | null>(null);
  const reconnectTimeoutRef = useRef<number | null>(null);
  const currentInstanceIdRef = useRef<string | null>(null);

  // Clean up on unmount
  useEffect(() => {
    return () => {
      disconnect();
    };
  }, []);

  /**
   * Process incoming SSE event and update execution state
   */
  const processEvent = useCallback((event: FlowExecutionEvent) => {
    setExecutionState((prev) => {
      const now = new Date();

      switch (event.type) {
        case 'step_started': {
          const stepInfo: StepExecutionInfo = {
            state: 'running',
            startedAt: new Date(event.timestamp),
          };
          return {
            ...prev,
            status: 'running',
            currentNodeId: event.node_id,
            steps: {
              ...prev.steps,
              [event.node_id]: stepInfo,
            },
            logs: [
              ...prev.logs,
              {
                timestamp: now,
                level: 'info',
                message: `Step started: ${event.step_name || event.node_id}`,
                nodeId: event.node_id,
              },
            ],
          };
        }

        case 'step_completed': {
          const existingStep = prev.steps[event.node_id];
          const stepInfo: StepExecutionInfo = {
            state: 'completed',
            startedAt: existingStep?.startedAt,
            endedAt: new Date(event.timestamp),
            durationMs: event.duration_ms,
            output: event.output,
          };
          return {
            ...prev,
            currentNodeId: undefined,
            steps: {
              ...prev.steps,
              [event.node_id]: stepInfo,
            },
            logs: [
              ...prev.logs,
              {
                timestamp: now,
                level: 'info',
                message: `Step completed in ${event.duration_ms}ms`,
                nodeId: event.node_id,
              },
            ],
          };
        }

        case 'step_failed': {
          const existingStep = prev.steps[event.node_id];
          const stepInfo: StepExecutionInfo = {
            state: 'failed',
            startedAt: existingStep?.startedAt,
            endedAt: new Date(event.timestamp),
            durationMs: event.duration_ms,
            error: event.error,
          };
          return {
            ...prev,
            currentNodeId: undefined,
            steps: {
              ...prev.steps,
              [event.node_id]: stepInfo,
            },
            logs: [
              ...prev.logs,
              {
                timestamp: now,
                level: 'error',
                message: `Step failed: ${event.error}`,
                nodeId: event.node_id,
              },
            ],
          };
        }

        case 'flow_waiting': {
          const existingStep = prev.steps[event.node_id];
          const stepInfo: StepExecutionInfo = {
            ...existingStep,
            state: 'waiting',
            waitReason: event.reason,
          };
          return {
            ...prev,
            status: 'waiting',
            currentNodeId: event.node_id,
            steps: {
              ...prev.steps,
              [event.node_id]: stepInfo,
            },
            logs: [
              ...prev.logs,
              {
                timestamp: now,
                level: 'info',
                message: `Waiting: ${event.reason}`,
                nodeId: event.node_id,
              },
            ],
          };
        }

        case 'flow_resumed': {
          const existingStep = prev.steps[event.node_id];
          const stepInfo: StepExecutionInfo = {
            ...existingStep,
            state: 'running',
            waitReason: undefined,
          };
          return {
            ...prev,
            status: 'running',
            steps: {
              ...prev.steps,
              [event.node_id]: stepInfo,
            },
            logs: [
              ...prev.logs,
              {
                timestamp: now,
                level: 'info',
                message: `Resumed after ${event.wait_duration_ms}ms`,
                nodeId: event.node_id,
              },
            ],
          };
        }

        case 'flow_completed': {
          return {
            ...prev,
            status: 'completed',
            currentNodeId: undefined,
            endedAt: new Date(event.timestamp),
            totalDurationMs: event.total_duration_ms,
            output: event.output,
            logs: [
              ...prev.logs,
              {
                timestamp: now,
                level: 'info',
                message: `Flow completed in ${event.total_duration_ms}ms`,
              },
            ],
          };
        }

        case 'flow_failed': {
          return {
            ...prev,
            status: 'failed',
            currentNodeId: event.failed_at_node,
            endedAt: new Date(event.timestamp),
            totalDurationMs: event.total_duration_ms,
            error: event.error,
            logs: [
              ...prev.logs,
              {
                timestamp: now,
                level: 'error',
                message: `Flow failed: ${event.error}`,
                nodeId: event.failed_at_node,
              },
            ],
          };
        }

        case 'log': {
          const logEntry: ExecutionLogEntry = {
            timestamp: new Date(event.timestamp),
            level: event.level as ExecutionLogEntry['level'],
            message: event.message,
            nodeId: event.node_id,
          };
          return {
            ...prev,
            logs: [...prev.logs, logEntry],
          };
        }

        default:
          return prev;
      }
    });
  }, []);

  /**
   * Subscribe to SSE events for a flow instance
   */
  const subscribeToInstance = useCallback(
    (instanceId: string) => {
      // Disconnect any existing connection
      disconnect();

      currentInstanceIdRef.current = instanceId;
      const url = `${baseUrl}/api/flows/${repoId}/instances/${instanceId}/events`;

      console.log('[FlowExecution] Subscribing to:', url);

      const eventSource = new EventSource(url);
      eventSourceRef.current = eventSource;

      eventSource.onopen = () => {
        console.log('[FlowExecution] SSE connected');
        setIsConnected(true);
      };

      eventSource.addEventListener('flow-event', (e: MessageEvent) => {
        try {
          const event = JSON.parse(e.data) as FlowExecutionEvent;
          console.log('[FlowExecution] Event:', event.type, event);
          processEvent(event);
        } catch (err) {
          console.error('[FlowExecution] Failed to parse event:', err);
        }
      });

      eventSource.onerror = (err) => {
        console.error('[FlowExecution] SSE error:', err);
        setIsConnected(false);

        // Check if flow completed/failed (SSE closes on completion)
        if (eventSource.readyState === EventSource.CLOSED) {
          console.log('[FlowExecution] SSE closed');
          eventSourceRef.current = null;

          // Auto-reconnect if enabled and flow still running
          if (autoReconnect && currentInstanceIdRef.current) {
            setExecutionState((prev) => {
              if (prev.status === 'running' || prev.status === 'waiting') {
                reconnectTimeoutRef.current = window.setTimeout(() => {
                  if (currentInstanceIdRef.current) {
                    subscribeToInstance(currentInstanceIdRef.current);
                  }
                }, reconnectDelay);
              }
              return prev;
            });
          }
        }
      };
    },
    [baseUrl, repoId, processEvent, autoReconnect, reconnectDelay]
  );

  /**
   * Execute a flow and subscribe to events
   */
  const executeFlow = useCallback(
    async (flowPath: string, input: Record<string, unknown> = {}): Promise<string> => {
      // Reset state
      setExecutionState({
        ...INITIAL_EXECUTION_STATE,
        status: 'running',
        startedAt: new Date(),
      });

      const response = await fetch(`${baseUrl}/api/flows/${repoId}/run`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ flow_path: flowPath, input }),
      });

      if (!response.ok) {
        const error = await response.text();
        throw new Error(`Failed to execute flow: ${error}`);
      }

      const result = await response.json();
      const instanceId = result.instance_id;

      setExecutionState((prev) => ({
        ...prev,
        instanceId,
      }));

      // Subscribe to events
      subscribeToInstance(instanceId);

      return instanceId;
    },
    [baseUrl, repoId, subscribeToInstance]
  );

  /**
   * Execute a test run with mocking
   */
  const executeTestRun = useCallback(
    async (
      flowPath: string,
      input: Record<string, unknown> = {},
      testConfig: TestRunConfig = { is_test_run: true }
    ): Promise<string> => {
      // Reset state
      setExecutionState({
        ...INITIAL_EXECUTION_STATE,
        status: 'running',
        startedAt: new Date(),
      });

      const response = await fetch(`${baseUrl}/api/flows/${repoId}/test`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          flow_path: flowPath,
          input,
          test_config: testConfig,
        }),
      });

      if (!response.ok) {
        const error = await response.text();
        throw new Error(`Failed to execute test run: ${error}`);
      }

      const result = await response.json();
      const instanceId = result.instance_id;

      setExecutionState((prev) => ({
        ...prev,
        instanceId,
      }));

      // Subscribe to events
      subscribeToInstance(instanceId);

      return instanceId;
    },
    [baseUrl, repoId, subscribeToInstance]
  );

  /**
   * Disconnect from SSE stream
   */
  const disconnect = useCallback(() => {
    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current);
      reconnectTimeoutRef.current = null;
    }

    if (eventSourceRef.current) {
      eventSourceRef.current.close();
      eventSourceRef.current = null;
    }

    currentInstanceIdRef.current = null;
    setIsConnected(false);
  }, []);

  /**
   * Reset execution state
   */
  const reset = useCallback(() => {
    disconnect();
    setExecutionState(INITIAL_EXECUTION_STATE);
  }, [disconnect]);

  /**
   * Get execution state for a specific node
   */
  const getNodeState = useCallback(
    (nodeId: string): StepExecutionInfo | undefined => {
      return executionState.steps[nodeId];
    },
    [executionState.steps]
  );

  return {
    executionState,
    isConnected,
    executeFlow,
    executeTestRun,
    subscribeToInstance,
    disconnect,
    reset,
    getNodeState,
  };
}
