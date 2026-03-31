/**
 * Builder Command History Manager
 *
 * Manages the undo/redo stack for builder commands.
 * Generic implementation that works with any state type.
 */

import type { AbstractBuilderCommand } from './AbstractBuilderCommand'
import type { HistoryState } from './types'

/**
 * Command history manager implementing undo/redo functionality.
 *
 * @template TState - The state type managed by commands
 */
export class BuilderCommandHistory<TState> {
  private past: AbstractBuilderCommand<TState>[] = []
  private future: AbstractBuilderCommand<TState>[] = []
  private maxHistory: number
  private listeners: Set<(state: HistoryState) => void> = new Set()

  constructor(maxHistory: number = 50) {
    this.maxHistory = maxHistory
  }

  /**
   * Execute a command and add it to history.
   */
  execute(command: AbstractBuilderCommand<TState>): void {
    command.execute()
    this.past.push(command)
    this.future = [] // Clear redo stack on new command

    // Limit history size
    if (this.past.length > this.maxHistory) {
      this.past.shift()
    }

    this.notifyListeners()
  }

  /**
   * Undo the last command.
   * @returns true if undo was successful
   */
  undo(): boolean {
    const command = this.past.pop()
    if (!command) return false

    command.undo()
    this.future.unshift(command)
    this.notifyListeners()
    return true
  }

  /**
   * Redo the last undone command.
   * @returns true if redo was successful
   */
  redo(): boolean {
    const command = this.future.shift()
    if (!command) return false

    command.execute()
    this.past.push(command)
    this.notifyListeners()
    return true
  }

  /**
   * Check if undo is available.
   */
  get canUndo(): boolean {
    return this.past.length > 0
  }

  /**
   * Check if redo is available.
   */
  get canRedo(): boolean {
    return this.future.length > 0
  }

  /**
   * Get the current history state.
   */
  getState(): HistoryState {
    return {
      canUndo: this.canUndo,
      canRedo: this.canRedo,
      undoStack: this.past.map((cmd) => cmd.getMetadata()),
      redoStack: this.future.map((cmd) => cmd.getMetadata()),
    }
  }

  /**
   * Reset history (clear all undo/redo).
   */
  reset(): void {
    this.past = []
    this.future = []
    this.notifyListeners()
  }

  /**
   * Subscribe to history changes.
   */
  subscribe(listener: (state: HistoryState) => void): () => void {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  /**
   * Notify all listeners of state change.
   */
  private notifyListeners(): void {
    const state = this.getState()
    this.listeners.forEach((listener) => listener(state))
  }
}
