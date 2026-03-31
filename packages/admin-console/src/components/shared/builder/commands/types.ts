/**
 * Command Pattern Types for Visual Builders
 *
 * Generic types for implementing undo/redo in archetype-builder and nodetype-builder.
 */

/** Command types for tracking operations */
export type CommandType =
  | 'ADD_ITEM'
  | 'DELETE_ITEM'
  | 'MOVE_ITEM'
  | 'UPDATE_ITEM'
  | 'REORDER_ITEMS'
  | 'UPDATE_STATE' // Generic state update

/** Metadata for history display */
export interface CommandMetadata {
  type: CommandType
  description: string
  timestamp: number
}

/** Context provided to commands for state access */
export interface CommandContext<TState> {
  getState: () => TState
  setState: (updater: (prev: TState) => TState) => void
}

/** History state for external consumption */
export interface HistoryState {
  canUndo: boolean
  canRedo: boolean
  undoStack: CommandMetadata[]
  redoStack: CommandMetadata[]
}
