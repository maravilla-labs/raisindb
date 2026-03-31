/**
 * Move Step Command
 *
 * Command to move a step via drag-and-drop.
 */

import { AbstractCommand } from './AbstractCommand';
import type { CommandContext, MoveStepParams } from '../types';
import {
  cloneFlow,
  findNodeAndParent,
  removeNodeById,
  insertNode,
  isAncestorOf,
  cloneNode,
  isFlowStep,
} from '../utils';

export class MoveStepCommand extends AbstractCommand {
  private params: MoveStepParams;

  constructor(context: CommandContext, params: MoveStepParams) {
    super(context, 'MOVE_STEP', 'Move step');
    this.params = params;
  }

  execute(): void {
    const { sourceId, targetId, insertPosition } = this.params;
    const currentState = this.context.getState();
    const normalizedPosition =
      insertPosition === 'left'
        ? 'before'
        : insertPosition === 'right'
        ? 'after'
        : insertPosition;

    // Validate: cannot move a node into its own descendant
    if (isAncestorOf(currentState, sourceId, targetId)) {
      console.warn('Cannot move a node into its own descendant');
      return;
    }

    // Validate: cannot move to same position
    if (sourceId === targetId) {
      return;
    }

    this.saveState();

    this.context.setState((state) => {
      const newState = cloneFlow(state);

      // Find and clone the source node
      const sourceResult = findNodeAndParent(newState, sourceId);
      if (!sourceResult) return state;

      // Find target to determine new parent
      const targetResult = findNodeAndParent(newState, targetId);
      if (!targetResult) return state;

      const nodeCopy = cloneNode(sourceResult.node);

      // Determine new parent ID
      let newParentId: string | null = null;
      if (normalizedPosition === 'inside') {
        newParentId = targetResult.node.id;
      } else {
        newParentId = targetResult.parent ? targetResult.parent.id : null;
      }

      // Check if parent changed
      const oldParentId = sourceResult.parent ? sourceResult.parent.id : null;

      if (oldParentId !== newParentId) {
        // Parent changed, clear condition
        if (isFlowStep(nodeCopy) && nodeCopy.properties.condition) {
          delete nodeCopy.properties.condition;
        }
      }

      // Remove from original position
      removeNodeById(newState.nodes, sourceId);

      // Insert at new position
          const inserted = insertNode(
            newState.nodes,
            nodeCopy,
            targetId,
            normalizedPosition
          );
      if (!inserted) {
        // If insert failed, restore original state
        return state;
      }

      return newState;
    });
  }

  undo(): void {
    this.restoreState();
  }
}
