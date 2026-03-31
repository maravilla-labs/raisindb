/**
 * Flow State Hook
 *
 * Manages the flow definition state with change tracking.
 */

import { useState, useCallback, useRef, useEffect } from 'react';
import type { FlowDefinition } from '../types';
import { createEmptyFlow, cloneFlow } from '../utils';

export interface UseFlowStateOptions {
  initialFlow?: FlowDefinition;
  onChange?: (flow: FlowDefinition) => void;
}

export interface UseFlowStateReturn {
  flow: FlowDefinition;
  setFlow: (flow: FlowDefinition) => void;
  updateFlow: (updater: (prev: FlowDefinition) => FlowDefinition) => void;
  resetFlow: (newFlow?: FlowDefinition) => void;
  isDirty: boolean;
  getState: () => FlowDefinition;
  setState: (updater: (prev: FlowDefinition) => FlowDefinition) => void;
}

export function useFlowState(
  options: UseFlowStateOptions = {}
): UseFlowStateReturn {
  const { initialFlow, onChange } = options;

  // Initialize flow state
  const [flow, setFlowInternal] = useState<FlowDefinition>(() =>
    initialFlow ? cloneFlow(initialFlow) : createEmptyFlow()
  );

  // Track original flow for dirty detection
  const originalFlowRef = useRef<string>(JSON.stringify(flow));

  // Track if flow has been modified
  const [isDirty, setIsDirty] = useState(false);

  /**
   * Set flow with dirty tracking
   */
  const setFlow = useCallback(
    (newFlow: FlowDefinition) => {
      setFlowInternal(newFlow);
      setIsDirty(JSON.stringify(newFlow) !== originalFlowRef.current);
      onChange?.(newFlow);
    },
    [onChange]
  );

  /**
   * Update flow with a reducer function
   */
  const updateFlow = useCallback(
    (updater: (prev: FlowDefinition) => FlowDefinition) => {
      setFlowInternal((prev) => {
        const newFlow = updater(prev);
        setIsDirty(JSON.stringify(newFlow) !== originalFlowRef.current);
        onChange?.(newFlow);
        return newFlow;
      });
    },
    [onChange]
  );

  /**
   * Reset flow to initial or provided state
   */
  const resetFlow = useCallback(
    (newFlow?: FlowDefinition) => {
      const flowToSet = newFlow
        ? cloneFlow(newFlow)
        : initialFlow
        ? cloneFlow(initialFlow)
        : createEmptyFlow();
      originalFlowRef.current = JSON.stringify(flowToSet);
      setFlowInternal(flowToSet);
      setIsDirty(false);
    },
    [initialFlow]
  );

  /**
   * Get current state (for command context)
   */
  const getState = useCallback(() => flow, [flow]);

  /**
   * Set state with updater (for command context)
   */
  const setState = useCallback(
    (updater: (prev: FlowDefinition) => FlowDefinition) => {
      updateFlow(updater);
    },
    [updateFlow]
  );

  // Update original ref when initialFlow changes
  useEffect(() => {
    if (initialFlow) {
      originalFlowRef.current = JSON.stringify(initialFlow);
      setFlowInternal(cloneFlow(initialFlow));
      setIsDirty(false);
    }
  }, [initialFlow]);

  return {
    flow,
    setFlow,
    updateFlow,
    resetFlow,
    isDirty,
    getState,
    setState,
  };
}
