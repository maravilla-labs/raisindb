/**
 * Delete Step Command
 *
 * Command to delete a step or container from the flow.
 */

import { AbstractCommand } from './AbstractCommand';
import type { CommandContext, DeleteStepParams } from '../types';
import { cloneFlow, removeNodeById, findNodeAndParent, isFlowContainer } from '../utils';

export class DeleteStepCommand extends AbstractCommand {
  private params: DeleteStepParams;

  constructor(context: CommandContext, params: DeleteStepParams) {
    super(context, 'DELETE_STEP', 'Delete step');
    this.params = params;
  }

  execute(): void {
    this.saveState();

    this.context.setState((currentState) => {
      const newState = cloneFlow(currentState);
      const { nodeId, preserveChildren } = this.params;

      if (preserveChildren) {
        // Find the node to delete
        const result = findNodeAndParent(newState, nodeId);
        if (result && isFlowContainer(result.node)) {
          // After type guard, result.node is FlowContainer
          const children = [...result.node.children];

          // Remove the container
          removeNodeById(newState.nodes, nodeId);

          // Insert children at the container's former position
          if (result.parent) {
            // Insert into parent container
            result.parent.children.splice(result.index, 0, ...children);
          } else {
            // Insert into root nodes
            newState.nodes.splice(result.index, 0, ...children);
          }
        } else {
          // Not a container, just delete
          removeNodeById(newState.nodes, nodeId);
        }
      } else {
        // Simple delete (including children)
        removeNodeById(newState.nodes, nodeId);
      }

      return newState;
    });
  }

  undo(): void {
    this.restoreState();
  }
}
