/**
 * Update State Command
 *
 * Generic command for any state update with undo/redo support.
 * Captures the before/after state snapshot.
 */

import { AbstractBuilderCommand } from './AbstractBuilderCommand'
import type { CommandContext } from './types'

/**
 * Generic command that captures a state update.
 * Stores the old state on execute for undo capability.
 */
export class UpdateStateCommand<TState> extends AbstractBuilderCommand<TState> {
  private newState: TState

  constructor(
    context: CommandContext<TState>,
    newState: TState,
    description: string = 'Update state'
  ) {
    super(context, 'UPDATE_STATE', description)
    this.newState = this.cloneState(newState)
  }

  execute(): void {
    // Save current state for undo
    this.saveState()
    // Apply new state
    this.context.setState(() => this.cloneState(this.newState))
  }

  undo(): void {
    // Restore previous state
    this.restoreState()
  }
}
