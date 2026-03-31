/**
 * Flow Designer Context
 *
 * Provides centralized state management for the visual workflow designer.
 * Combines flow state, selection, command history, and validation.
 */

import { createContext, useContext, useMemo, useCallback, ReactNode } from 'react';
import type { FlowDefinition, FlowNode, CommandContext } from '../types';
import { useFlowState } from '../hooks/useFlowState';
import { useCommandHistory } from '../hooks/useCommandHistory';
import { useSelection } from '../hooks/useSelection';
import type { AbstractCommand } from '../commands/AbstractCommand';
import type { CommandHistory } from '../commands';

/**
 * Validation issue for inline feedback
 */
export interface ValidationIssue {
  /** Node ID where issue was found */
  nodeId: string;
  /** Field with issue (optional) */
  field?: string;
  /** Issue code for i18n */
  code: string;
  /** Human-readable message */
  message: string;
  /** Issue severity */
  severity: 'error' | 'warning' | 'suggestion';
}

/**
 * Validation result for a flow definition
 */
export interface ValidationResult {
  valid: boolean;
  errors: ValidationIssue[];
  warnings: ValidationIssue[];
  suggestions: ValidationIssue[];
}

/**
 * Flow designer context value
 */
export interface FlowDesignerContextValue {
  // Flow state
  flow: FlowDefinition;
  setFlow: (flow: FlowDefinition) => void;
  updateFlow: (updater: (prev: FlowDefinition) => FlowDefinition) => void;
  resetFlow: (newFlow?: FlowDefinition) => void;
  isDirty: boolean;

  // Selection
  selectedNodeIds: string[];
  selectNode: (nodeId: string, addToSelection?: boolean) => void;
  deselectNode: (nodeId: string) => void;
  clearSelection: () => void;
  isSelected: (nodeId: string) => boolean;
  selectMultiple: (nodeIds: string[]) => void;
  selectedNodes: FlowNode[];

  // Command history (undo/redo)
  canUndo: boolean;
  canRedo: boolean;
  undo: () => void;
  redo: () => void;
  executeCommand: (command: AbstractCommand) => void;
  commandHistory: CommandHistory;
  commandContext: CommandContext;

  // Validation
  validation: ValidationResult;
  validateFlow: () => ValidationResult;

  // Utility
  findNode: (nodeId: string) => FlowNode | undefined;
  findNodePath: (nodeId: string) => number[];
}

const FlowDesignerContext = createContext<FlowDesignerContextValue | null>(null);

/**
 * Props for FlowDesignerProvider
 */
export interface FlowDesignerProviderProps {
  children: ReactNode;
  initialFlow?: FlowDefinition;
  onChange?: (flow: FlowDefinition) => void;
  onValidate?: (flow: FlowDefinition) => ValidationResult;
}

/**
 * Find a node by ID recursively in the flow
 */
function findNodeInFlow(nodes: FlowNode[], nodeId: string): FlowNode | undefined {
  for (const node of nodes) {
    if (node.id === nodeId) {
      return node;
    }
    if (node.node_type === 'raisin:FlowContainer' && node.children) {
      const found = findNodeInFlow(node.children, nodeId);
      if (found) return found;
    }
  }
  return undefined;
}

/**
 * Find the path (indices) to a node in the flow
 */
function findNodePathInFlow(nodes: FlowNode[], nodeId: string, path: number[] = []): number[] | undefined {
  for (let i = 0; i < nodes.length; i++) {
    const node = nodes[i];
    if (node.id === nodeId) {
      return [...path, i];
    }
    if (node.node_type === 'raisin:FlowContainer' && node.children) {
      const childPath = findNodePathInFlow(node.children, nodeId, [...path, i]);
      if (childPath) return childPath;
    }
  }
  return undefined;
}

/**
 * FlowDesignerProvider - Provides flow designer context to children
 */
export function FlowDesignerProvider({
  children,
  initialFlow,
  onChange,
  onValidate,
}: FlowDesignerProviderProps) {
  // Flow state management
  const flowState = useFlowState({ initialFlow, onChange });

  // Command history for undo/redo
  const { history: commandHistory, undo, redo, canUndo, canRedo, createContext: createCommandContext } = useCommandHistory();

  // Node selection (with multi-select enabled)
  const selection = useSelection({ multiSelect: true });

  // Create command context for executing commands
  const commandContext = useMemo<CommandContext>(
    () => createCommandContext(flowState.getState, flowState.setState),
    [createCommandContext, flowState.getState, flowState.setState]
  );

  // Execute a command and add to history
  const executeCommand = useCallback(
    (command: AbstractCommand): void => {
      commandHistory.execute(command);
    },
    [commandHistory]
  );

  // Validation
  const validateFlow = useCallback((): ValidationResult => {
    if (onValidate) {
      return onValidate(flowState.flow);
    }
    // Default basic validation
    const errors: ValidationIssue[] = [];
    const warnings: ValidationIssue[] = [];

    // Check for empty flow
    if (flowState.flow.nodes.length === 0) {
      warnings.push({
        nodeId: '',
        code: 'EMPTY_FLOW',
        message: 'Flow has no nodes',
        severity: 'warning',
      });
    }

    return {
      valid: errors.length === 0,
      errors,
      warnings,
      suggestions: [],
    };
  }, [flowState.flow, onValidate]);

  // Compute validation result
  const validation = useMemo(() => validateFlow(), [validateFlow]);

  // Find node by ID
  const findNode = useCallback(
    (nodeId: string): FlowNode | undefined => {
      return findNodeInFlow(flowState.flow.nodes, nodeId);
    },
    [flowState.flow.nodes]
  );

  // Find node path
  const findNodePath = useCallback(
    (nodeId: string): number[] => {
      return findNodePathInFlow(flowState.flow.nodes, nodeId) || [];
    },
    [flowState.flow.nodes]
  );

  // Get selected nodes
  const selectedNodes = useMemo(() => {
    return selection.selection
      .map((id: string) => findNode(id))
      .filter((node): node is FlowNode => node !== undefined);
  }, [selection.selection, findNode]);

  // Context value
  const contextValue = useMemo<FlowDesignerContextValue>(
    () => ({
      // Flow state
      flow: flowState.flow,
      setFlow: flowState.setFlow,
      updateFlow: flowState.updateFlow,
      resetFlow: flowState.resetFlow,
      isDirty: flowState.isDirty,

      // Selection (map to expected interface)
      selectedNodeIds: selection.selection,
      selectNode: selection.select,
      deselectNode: selection.deselect,
      clearSelection: selection.clearSelection,
      isSelected: selection.isSelected,
      selectMultiple: selection.selectAll,
      selectedNodes,

      // Command history
      canUndo,
      canRedo,
      undo,
      redo,
      executeCommand,
      commandHistory,
      commandContext,

      // Validation
      validation,
      validateFlow,

      // Utility
      findNode,
      findNodePath,
    }),
    [
      flowState,
      selection,
      selectedNodes,
      canUndo,
      canRedo,
      undo,
      redo,
      executeCommand,
      commandHistory,
      commandContext,
      validation,
      validateFlow,
      findNode,
      findNodePath,
    ]
  );

  return (
    <FlowDesignerContext.Provider value={contextValue}>
      {children}
    </FlowDesignerContext.Provider>
  );
}

/**
 * Hook to access the flow designer context
 * @throws Error if used outside FlowDesignerProvider
 */
export function useFlowDesignerContext(): FlowDesignerContextValue {
  const context = useContext(FlowDesignerContext);
  if (!context) {
    throw new Error('useFlowDesignerContext must be used within a FlowDesignerProvider');
  }
  return context;
}

export default FlowDesignerContext;
