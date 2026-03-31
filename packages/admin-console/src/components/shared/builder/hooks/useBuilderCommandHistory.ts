/**
 * useBuilderCommandHistory Hook
 *
 * React hook for managing command history with undo/redo support.
 */

import { useState, useEffect, useCallback, useRef, useMemo } from 'react'
import { BuilderCommandHistory } from '../commands/BuilderCommandHistory'
import type { AbstractBuilderCommand } from '../commands/AbstractBuilderCommand'
import type { CommandContext, HistoryState } from '../commands/types'

export interface UseBuilderCommandHistoryOptions {
  /** Maximum number of commands to keep in history */
  maxHistory?: number
}

export interface UseBuilderCommandHistoryReturn<TState> {
  /** The command history instance */
  history: BuilderCommandHistory<TState>
  /** Current history state */
  historyState: HistoryState
  /** Execute a command */
  execute: (command: AbstractBuilderCommand<TState>) => void
  /** Undo the last command */
  undo: () => boolean
  /** Redo the last undone command */
  redo: () => boolean
  /** Create a command context for the current state */
  createContext: (
    getState: () => TState,
    setState: (updater: (prev: TState) => TState) => void
  ) => CommandContext<TState>
  /** Reset the history */
  reset: () => void
}

export function useBuilderCommandHistory<TState>({
  maxHistory = 50,
}: UseBuilderCommandHistoryOptions = {}): UseBuilderCommandHistoryReturn<TState> {
  // Create history instance once
  const historyRef = useRef<BuilderCommandHistory<TState> | null>(null)
  if (!historyRef.current) {
    historyRef.current = new BuilderCommandHistory<TState>(maxHistory)
  }
  const history = historyRef.current

  // Track history state for React
  const [historyState, setHistoryState] = useState<HistoryState>(
    history.getState()
  )

  // Subscribe to history changes
  useEffect(() => {
    const unsubscribe = history.subscribe(setHistoryState)
    return unsubscribe
  }, [history])

  // Execute a command
  const execute = useCallback(
    (command: AbstractBuilderCommand<TState>) => {
      history.execute(command)
    },
    [history]
  )

  // Undo
  const undo = useCallback(() => {
    return history.undo()
  }, [history])

  // Redo
  const redo = useCallback(() => {
    return history.redo()
  }, [history])

  // Reset
  const reset = useCallback(() => {
    history.reset()
  }, [history])

  // Create a command context
  const createContext = useCallback(
    (
      getState: () => TState,
      setState: (updater: (prev: TState) => TState) => void
    ): CommandContext<TState> => {
      return { getState, setState }
    },
    []
  )

  return useMemo(
    () => ({
      history,
      historyState,
      execute,
      undo,
      redo,
      createContext,
      reset,
    }),
    [history, historyState, execute, undo, redo, createContext, reset]
  )
}
