/**
 * Abstract Command Base Class
 *
 * Base class for all commands implementing the Command pattern.
 * Provides execute/undo capability for flow modifications.
 */

import type { CommandContext, CommandMetadata, CommandType } from '../types';
import type { FlowDefinition } from '../types';

/**
 * Abstract base class for flow commands.
 * Subclasses must implement execute() and undo() methods.
 */
export abstract class AbstractCommand {
  protected context: CommandContext;
  protected previousState: FlowDefinition | null = null;
  protected metadata: CommandMetadata;

  constructor(context: CommandContext, type: CommandType, description: string) {
    this.context = context;
    this.metadata = {
      type,
      description,
      timestamp: Date.now(),
    };
  }

  /**
   * Execute the command.
   * Should store previous state for undo capability.
   */
  abstract execute(): void;

  /**
   * Undo the command.
   * Should restore the previous state.
   */
  abstract undo(): void;

  /**
   * Get command metadata for history display.
   */
  getMetadata(): CommandMetadata {
    return this.metadata;
  }

  /**
   * Deep clone to ensure immutability.
   */
  protected cloneState<T>(data: T): T {
    return JSON.parse(JSON.stringify(data));
  }

  /**
   * Store current state for undo.
   */
  protected saveState(): void {
    this.previousState = this.cloneState(this.context.getState());
  }

  /**
   * Restore previously saved state.
   */
  protected restoreState(): void {
    if (this.previousState) {
      this.context.setState(() => this.cloneState(this.previousState!));
    }
  }
}
