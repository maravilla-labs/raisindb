/**
 * Abstract Builder Command Base Class
 *
 * Generic base class for all builder commands implementing the Command pattern.
 * Provides execute/undo capability with state snapshotting.
 */

import type { CommandContext, CommandMetadata, CommandType } from './types'

/**
 * Abstract base class for builder commands.
 * Subclasses must implement execute() and undo() methods.
 *
 * @template TState - The state type (ArchetypeDefinition or NodeTypeDefinition)
 */
export abstract class AbstractBuilderCommand<TState> {
  protected context: CommandContext<TState>
  protected previousState: TState | null = null
  protected metadata: CommandMetadata

  constructor(
    context: CommandContext<TState>,
    type: CommandType,
    description: string
  ) {
    this.context = context
    this.metadata = {
      type,
      description,
      timestamp: Date.now(),
    }
  }

  /**
   * Execute the command.
   * Should store previous state for undo capability.
   */
  abstract execute(): void

  /**
   * Undo the command.
   * Should restore the previous state.
   */
  abstract undo(): void

  /**
   * Get command metadata for history display.
   */
  getMetadata(): CommandMetadata {
    return this.metadata
  }

  /**
   * Deep clone to ensure immutability.
   */
  protected cloneState<T>(data: T): T {
    return JSON.parse(JSON.stringify(data))
  }

  /**
   * Store current state for undo.
   */
  protected saveState(): void {
    this.previousState = this.cloneState(this.context.getState())
  }

  /**
   * Restore previously saved state.
   */
  protected restoreState(): void {
    if (this.previousState) {
      this.context.setState(() => this.cloneState(this.previousState!))
    }
  }
}
