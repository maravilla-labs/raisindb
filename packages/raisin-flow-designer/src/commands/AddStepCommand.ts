/**
 * Add Step Command
 *
 * Command to add a new step or container to the flow.
 * Supports left/right positioning by wrapping in a parallel container.
 */

import { AbstractCommand } from './AbstractCommand';
import type {
  CommandContext,
  StepType,
  AddStepParams,
  FlowNode,
  FlowStep,
  FlowContainer,
  InsertPosition,
} from '../types';
import { STEP_TEMPLATES, isFlowContainer } from '../types';
import {
  generateStepId,
  generateContainerId,
  findNodeById,
  cloneFlow,
} from '../utils';

export class AddStepCommand extends AbstractCommand {
  private params: AddStepParams;
  private newNodeId: string | null = null;

  constructor(context: CommandContext, params: AddStepParams) {
    const typeLabel = params.type === 'step' ? 'step' : `${params.type} container`;
    super(context, 'ADD_STEP', `Add ${typeLabel}`);
    this.params = params;
  }

  execute(): void {
    this.saveState();

    const { type, targetId, insertPosition } = this.params;

    // Handle left/right - wrap target in parallel container
    if ((insertPosition === 'left' || insertPosition === 'right') && targetId) {
      this.wrapInParallelContainer(targetId, insertPosition);
      return;
    }

    // Normal add (before/after/inside)
    const newNode = this.createNode(type);
    this.newNodeId = newNode.id;

    this.context.setState((currentState) => {
      const newState = cloneFlow(currentState);

      if (!targetId) {
        // Append to root
        newState.nodes.push(newNode);
      } else {
        // Insert relative to target
        const inserted = this.insertNodeRelative(
          newState.nodes,
          newNode,
          targetId,
          insertPosition
        );

        if (!inserted) {
          // Target not found, append to root
          newState.nodes.push(newNode);
        }
      }

      return newState;
    });
  }

  undo(): void {
    this.restoreState();
  }

  /**
   * Get the ID of the newly created node.
   */
  getNewNodeId(): string | null {
    return this.newNodeId;
  }

  /**
   * Wrap target node in a parallel container and add new step alongside
   */
  private wrapInParallelContainer(
    targetId: string,
    position: 'left' | 'right'
  ): void {
    const newStep = this.createNode('step');
    this.newNodeId = newStep.id;

    this.context.setState((currentState) => {
      const newState = cloneFlow(currentState);

      // Find the target node and its location
      const result = findNodeById(newState.nodes, targetId);
      if (!result) return currentState;

      const { node: targetNode, parent, index } = result;

      // If target is already inside a parallel container, just add to it
      if (parent && isFlowContainer(parent) && parent.container_type === 'parallel') {
        // Insert at appropriate position
        if (position === 'left') {
          parent.children.splice(index, 0, newStep);
        } else {
          parent.children.splice(index + 1, 0, newStep);
        }
        return newState;
      }

      // Create a new parallel container
      const containerId = generateContainerId();
      const container: FlowContainer = {
        id: containerId,
        node_type: 'raisin:FlowContainer',
        container_type: 'parallel',
        children:
          position === 'left'
            ? [newStep, targetNode]
            : [targetNode, newStep],
      };

      // Replace target node with container
      if (parent) {
        // Target is inside a container
        parent.children[index] = container;
      } else {
        // Target is at root level
        newState.nodes[index] = container;
      }

      return newState;
    });
  }

  /**
   * Insert a node relative to a target node
   */
  private insertNodeRelative(
    nodes: FlowNode[],
    newNode: FlowNode,
    targetId: string,
    position: InsertPosition
  ): boolean {
    for (let i = 0; i < nodes.length; i++) {
      const node = nodes[i];

      if (node.id === targetId) {
        if (position === 'inside' && isFlowContainer(node)) {
          node.children.push(newNode);
          return true;
        } else if (position === 'before') {
          nodes.splice(i, 0, newNode);
          return true;
        } else if (position === 'after') {
          nodes.splice(i + 1, 0, newNode);
          return true;
        }
      }

      if (isFlowContainer(node)) {
        if (this.insertNodeRelative(node.children, newNode, targetId, position)) {
          return true;
        }
      }
    }
    return false;
  }

  /**
   * Create a new node from template.
   */
  private createNode(type: StepType): FlowNode {
    const template = STEP_TEMPLATES[type];
    const id =
      template.node_type === 'raisin:FlowStep'
        ? generateStepId()
        : generateContainerId();

    if (template.node_type === 'raisin:FlowStep') {
      return {
        id,
        node_type: 'raisin:FlowStep',
        properties: { ...template.properties },
      } as FlowStep;
    }

    return {
      id,
      node_type: 'raisin:FlowContainer',
      container_type: template.container_type!,
      children: [],
    } as FlowContainer;
  }
}
