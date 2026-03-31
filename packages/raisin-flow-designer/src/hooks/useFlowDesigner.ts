/**
 * useFlowDesigner Hook
 *
 * Convenience hook that wraps useFlowDesignerContext with additional
 * high-level operations for common flow designer actions.
 */

import { useCallback, useMemo } from 'react';
import { useFlowDesignerContext, type ValidationResult } from '../context/FlowDesignerContext';
import type { FlowNode, FlowStepProperties, ContainerType, RaisinReference, StepType } from '../types';
import { AddStepCommand, DeleteStepCommand, MoveStepCommand, UpdateStepCommand } from '../commands';
import type { InsertPosition } from '../types';

/**
 * Step creation options
 */
export interface CreateStepOptions {
  action?: string;
  function_ref?: RaisinReference;
  agent_ref?: RaisinReference;
  step_type?: 'default' | 'ai_agent' | 'human_task';
  payload_key?: string;
}

/**
 * Container creation options
 */
export interface CreateContainerOptions {
  container_type: ContainerType;
  children?: FlowNode[];
}

/**
 * Extended flow designer interface
 */
export interface UseFlowDesignerReturn {
  // Context passthrough
  flow: ReturnType<typeof useFlowDesignerContext>['flow'];
  selectedNodeIds: string[];
  selectedNodes: FlowNode[];
  isDirty: boolean;
  validation: ValidationResult;
  canUndo: boolean;
  canRedo: boolean;

  // Actions
  undo: () => void;
  redo: () => void;
  selectNode: (nodeId: string, addToSelection?: boolean) => void;
  clearSelection: () => void;

  // High-level operations
  addStep: (stepType: StepType, targetId?: string | null, insertPosition?: InsertPosition) => void;
  addContainer: (options: CreateContainerOptions, targetId?: string | null, insertPosition?: InsertPosition) => void;
  deleteNode: (nodeId: string, preserveChildren?: boolean) => void;
  deleteSelectedNodes: () => void;
  moveNode: (sourceId: string, targetId: string, insertPosition: InsertPosition) => void;
  updateStepProperties: (nodeId: string, properties: Partial<FlowStepProperties>) => void;

  // Utility
  getNodeById: (nodeId: string) => FlowNode | undefined;
  getNodePath: (nodeId: string) => number[];
  isNodeSelected: (nodeId: string) => boolean;
}

/**
 * useFlowDesigner - High-level hook for flow designer operations
 */
export function useFlowDesigner(): UseFlowDesignerReturn {
  const context = useFlowDesignerContext();

  // Add a new step node using existing AddStepCommand
  const addStep = useCallback(
    (stepType: StepType, targetId?: string | null, insertPosition: InsertPosition = 'after'): void => {
      const command = new AddStepCommand(context.commandContext, {
        type: stepType,
        targetId: targetId ?? null,
        insertPosition,
      });

      context.executeCommand(command);
    },
    [context]
  );

  // Create a new container node
  const addContainer = useCallback(
    (options: CreateContainerOptions, targetId?: string | null, insertPosition: InsertPosition = 'after'): void => {
      // Map container type to StepType
      const stepType: StepType = options.container_type === 'parallel' ? 'parallel' :
                                  options.container_type === 'ai_sequence' ? 'ai_sequence' :
                                  options.container_type === 'and' ? 'and' :
                                  options.container_type === 'or' ? 'or' : 'parallel';

      const command = new AddStepCommand(context.commandContext, {
        type: stepType,
        targetId: targetId ?? null,
        insertPosition,
      });

      context.executeCommand(command);
    },
    [context]
  );

  // Delete a node by ID
  const deleteNode = useCallback(
    (nodeId: string, preserveChildren?: boolean) => {
      const command = new DeleteStepCommand(context.commandContext, {
        nodeId,
        preserveChildren,
      });

      context.executeCommand(command);

      // Also deselect the node
      context.deselectNode(nodeId);
    },
    [context]
  );

  // Delete all selected nodes
  const deleteSelectedNodes = useCallback(() => {
    const nodeIds = [...context.selectedNodeIds];
    nodeIds.forEach((id) => deleteNode(id));
  }, [context.selectedNodeIds, deleteNode]);

  // Move a node to a new location
  const moveNode = useCallback(
    (sourceId: string, targetId: string, insertPosition: InsertPosition) => {
      const command = new MoveStepCommand(context.commandContext, {
        sourceId,
        targetId,
        insertPosition,
      });

      context.executeCommand(command);
    },
    [context]
  );

  // Update step properties
  const updateStepProperties = useCallback(
    (nodeId: string, properties: Partial<FlowStepProperties>) => {
      const command = new UpdateStepCommand(context.commandContext, {
        nodeId,
        updates: properties,
      });

      context.executeCommand(command);
    },
    [context]
  );

  return useMemo(
    () => ({
      // Context passthrough
      flow: context.flow,
      selectedNodeIds: context.selectedNodeIds,
      selectedNodes: context.selectedNodes,
      isDirty: context.isDirty,
      validation: context.validation,
      canUndo: context.canUndo,
      canRedo: context.canRedo,

      // Actions
      undo: context.undo,
      redo: context.redo,
      selectNode: context.selectNode,
      clearSelection: context.clearSelection,

      // High-level operations
      addStep,
      addContainer,
      deleteNode,
      deleteSelectedNodes,
      moveNode,
      updateStepProperties,

      // Utility
      getNodeById: context.findNode,
      getNodePath: context.findNodePath,
      isNodeSelected: context.isSelected,
    }),
    [
      context,
      addStep,
      addContainer,
      deleteNode,
      deleteSelectedNodes,
      moveNode,
      updateStepProperties,
    ]
  );
}

export default useFlowDesigner;
