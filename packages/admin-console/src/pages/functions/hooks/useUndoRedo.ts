/**
 * useUndoRedo Hook
 *
 * Custom hook for managing undo/redo state with a history stack.
 * Provides a simple API for form editors without Monaco.
 */

import { useState, useCallback, useRef } from 'react'

interface UndoRedoOptions {
  maxHistory?: number
}

export interface UndoRedoResult<T> {
  /** Current value */
  value: T
  /** Set a new value (pushes to history) */
  setValue: (newValue: T | ((prev: T) => T)) => void
  /** Undo to previous state */
  undo: () => void
  /** Redo to next state */
  redo: () => void
  /** Whether undo is available */
  canUndo: boolean
  /** Whether redo is available */
  canRedo: boolean
  /** Reset history with a new initial value (e.g., after save) */
  reset: (initialValue: T) => void
  /** Whether current value differs from the saved value */
  isDirty: boolean
}

/**
 * Deep equality check for objects
 */
function deepEqual<T>(a: T, b: T): boolean {
  if (a === b) return true
  if (a === null || b === null) return false
  if (typeof a !== 'object' || typeof b !== 'object') return false

  const keysA = Object.keys(a as object)
  const keysB = Object.keys(b as object)

  if (keysA.length !== keysB.length) return false

  for (const key of keysA) {
    if (!keysB.includes(key)) return false
    if (!deepEqual((a as Record<string, unknown>)[key], (b as Record<string, unknown>)[key])) {
      return false
    }
  }

  return true
}

/**
 * Hook for managing undo/redo state
 *
 * @param initialValue - Initial value for the state
 * @param options - Configuration options
 * @returns UndoRedoResult with state and control functions
 */
export function useUndoRedo<T>(
  initialValue: T,
  options: UndoRedoOptions = {}
): UndoRedoResult<T> {
  const { maxHistory = 50 } = options

  // Current value
  const [value, setValueInternal] = useState<T>(initialValue)

  // History stacks
  const [past, setPast] = useState<T[]>([])
  const [future, setFuture] = useState<T[]>([])

  // Saved value reference (for isDirty check)
  const savedValueRef = useRef<T>(initialValue)

  /**
   * Set a new value and push current to history
   */
  const setValue = useCallback(
    (newValue: T | ((prev: T) => T)) => {
      setValueInternal((currentValue) => {
        const resolvedValue = typeof newValue === 'function'
          ? (newValue as (prev: T) => T)(currentValue)
          : newValue

        // Don't push to history if value hasn't changed
        if (deepEqual(resolvedValue, currentValue)) {
          return currentValue
        }

        // Push current value to past
        setPast((prevPast) => {
          const newPast = [...prevPast, currentValue]
          // Limit history size
          if (newPast.length > maxHistory) {
            return newPast.slice(newPast.length - maxHistory)
          }
          return newPast
        })

        // Clear future when new value is set
        setFuture([])

        return resolvedValue
      })
    },
    [maxHistory]
  )

  /**
   * Undo to previous state
   */
  const undo = useCallback(() => {
    setPast((prevPast) => {
      if (prevPast.length === 0) return prevPast

      const newPast = [...prevPast]
      const previous = newPast.pop()!

      // Push current to future
      setFuture((prevFuture) => [value, ...prevFuture])

      // Set previous as current
      setValueInternal(previous)

      return newPast
    })
  }, [value])

  /**
   * Redo to next state
   */
  const redo = useCallback(() => {
    setFuture((prevFuture) => {
      if (prevFuture.length === 0) return prevFuture

      const newFuture = [...prevFuture]
      const next = newFuture.shift()!

      // Push current to past
      setPast((prevPast) => [...prevPast, value])

      // Set next as current
      setValueInternal(next)

      return newFuture
    })
  }, [value])

  /**
   * Reset history with a new initial value
   */
  const reset = useCallback((newInitialValue: T) => {
    setValueInternal(newInitialValue)
    setPast([])
    setFuture([])
    savedValueRef.current = newInitialValue
  }, [])

  return {
    value,
    setValue,
    undo,
    redo,
    canUndo: past.length > 0,
    canRedo: future.length > 0,
    reset,
    isDirty: !deepEqual(value, savedValueRef.current),
  }
}
