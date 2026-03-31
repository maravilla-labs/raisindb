/**
 * Command History Hook
 *
 * React hook for managing command history with undo/redo.
 */

import { useState, useCallback, useEffect, useMemo } from 'react';
import { CommandHistory, type HistoryState } from '../commands';
import type { CommandContext, FlowDefinition } from '../types';

export interface UseCommandHistoryOptions {
  maxHistory?: number;
}

export interface UseCommandHistoryReturn {
  history: CommandHistory;
  historyState: HistoryState;
  undo: () => void;
  redo: () => void;
  canUndo: boolean;
  canRedo: boolean;
  createContext: (
    getState: () => FlowDefinition,
    setState: (updater: (prev: FlowDefinition) => FlowDefinition) => void
  ) => CommandContext;
}

export function useCommandHistory(
  options: UseCommandHistoryOptions = {}
): UseCommandHistoryReturn {
  const { maxHistory = 50 } = options;

  // Create stable history instance
  const history = useMemo(() => new CommandHistory(maxHistory), [maxHistory]);

  // Track history state for reactivity
  const [historyState, setHistoryState] = useState<HistoryState>(
    history.getState()
  );

  // Subscribe to history changes
  useEffect(() => {
    const unsubscribe = history.subscribe(setHistoryState);
    return unsubscribe;
  }, [history]);

  const undo = useCallback(() => {
    history.undo();
  }, [history]);

  const redo = useCallback(() => {
    history.redo();
  }, [history]);

  const createContext = useCallback(
    (
      getState: () => FlowDefinition,
      setState: (updater: (prev: FlowDefinition) => FlowDefinition) => void
    ): CommandContext => ({
      getState,
      setState,
    }),
    []
  );

  return {
    history,
    historyState,
    undo,
    redo,
    canUndo: historyState.canUndo,
    canRedo: historyState.canRedo,
    createContext,
  };
}
