/**
 * Flow Designer Component
 *
 * Main component that assembles the complete flow editor.
 * Matches the design specification.
 */

import { Fragment, useState, useCallback, useEffect, useMemo, useImperativeHandle, forwardRef } from 'react';
import { clsx } from 'clsx';
import type { FlowDefinition, FlowNode, StepType, InsertPosition, RaisinReference, CommandContext, ContainerType, ContainerRule, FlowStepProperties, AiContainerConfig } from '../types';
import { isFlowStep, isFlowContainer } from '../types';
import { useFlowState, useDragAndDrop, useCommandHistory, useSelection } from '../hooks';
import {
  AddStepCommand,
  DeleteStepCommand,
  MoveStepCommand,
  UpdateStepCommand,
  UpdateRulesCommand,
  AbstractCommand,
} from '../commands';
import { ThemeProvider, useThemeClasses, type FlowTheme } from '../context';
import { FlowToolbar } from './FlowToolbar';
import { FlowCanvas } from './FlowCanvas';
import { StartNode, EndNode, StepNode, ContainerNode, TriggerNode, AddTriggerButton, type TriggerInfo } from './nodes';
import { VerticalConnector, ConnectorWithButton } from './connections';
import { DropIndicator, GhostNode } from './dnd';
import { NodePalette } from './NodePalette';
import { findDropTargetFromPoint, calculateInsertPosition, calculateDropIndicator } from '../utils';

/** Execution state for a single node */
export type NodeExecutionStatus = 'idle' | 'running' | 'completed' | 'failed' | 'waiting';

/** Execution state for the entire flow */
export interface ExecutionState {
  /** Current node being executed */
  currentNodeId?: string;
  /** Nodes that have completed successfully */
  completedNodeIds: Set<string>;
  /** Nodes that have failed */
  failedNodeIds: Set<string>;
  /** Node currently waiting (e.g., for human input) */
  waitingNodeId?: string;
  /** Whether the flow is currently executing */
  isExecuting: boolean;
}

export interface FlowDesignerProps {
  /** Initial flow definition */
  flow: FlowDefinition;
  /** Called when flow changes */
  onChange?: (flow: FlowDefinition) => void;
  /** Called when a node is selected */
  onSelect?: (nodeId: string | null) => void;
  /** Currently selected node ID */
  selectedNodeId?: string | null;
  /** Called when save is requested */
  onSave?: () => void;
  /** Called when user wants to pick a function for a step */
  onOpenFunctionPicker?: (stepId: string) => void;
  /** Called when user wants to pick an agent for a step */
  onOpenAgentPicker?: (stepId: string) => void;
  /** Trigger type for StartNode display */
  triggerType?: 'node_event' | 'schedule' | 'http';
  /** Whether editing is disabled */
  disabled?: boolean;
  /** Whether to show the node palette */
  showPalette?: boolean;
  /** Whether to show the toolbar (default: true) */
  showToolbar?: boolean;
  /** Whether properties panel is visible */
  propertiesVisible?: boolean;
  /** Handler for toggling properties panel */
  onToggleProperties?: () => void;
  /** Handler for run action */
  onRun?: () => void;
  /** Theme for the designer ('light' or 'dark', default: 'light') */
  theme?: FlowTheme;
  /** Custom class name */
  className?: string;
  /** Triggers to display before start node */
  triggers?: TriggerInfo[];
  /** Currently selected trigger ID */
  selectedTriggerId?: string | null;
  /** Called when a trigger is selected */
  onTriggerSelect?: (triggerId: string | null) => void;
  /** Called when user wants to add a trigger */
  onAddTrigger?: () => void;
  /** Called when user wants to edit a trigger */
  onEditTrigger?: (triggerId: string) => void;
  /** Called when user wants to delete a trigger */
  onDeleteTrigger?: (triggerId: string) => void;
  /** Execution state for runtime visualization */
  executionState?: ExecutionState;
}

/** Imperative handle for FlowDesigner */
export interface FlowDesignerHandle {
  /** Set the function for a step (call after function picker selection) */
  setStepFunction: (stepId: string, functionRef: RaisinReference) => void;
  /** Set the agent for a step (call after agent picker selection) */
  setStepAgent: (stepId: string, agentRef: RaisinReference) => void;
  /** Update step properties (title, disabled, etc.) */
  updateStepProperty: (nodeId: string, updates: Partial<FlowStepProperties>) => void;
  /** Update container properties (type, rules, ai_config, timeout) */
  updateContainer: (containerId: string, updates: { container_type?: ContainerType; rules?: ContainerRule[]; ai_config?: AiContainerConfig; timeout_ms?: number }) => void;
  /** Get the current flow state */
  getFlow: () => FlowDefinition;
  /** Undo last action */
  undo: () => void;
  /** Redo last undone action */
  redo: () => void;
  /** Delete selected node */
  deleteSelected: () => void;
  /** Check if undo is available */
  canUndo: () => boolean;
  /** Check if redo is available */
  canRedo: () => boolean;
  /** Check if delete is available */
  canDelete: () => boolean;
  /** Get current zoom level (0-100) */
  getZoom: () => number;
  /** Zoom in */
  zoomIn: () => void;
  /** Zoom out */
  zoomOut: () => void;
  /** Get current tool mode */
  getToolMode: () => 'select' | 'pan';
  /** Set tool mode */
  setToolMode: (mode: 'select' | 'pan') => void;
}

interface NodeRenderContext {
  parentId: string | null;
  prevSiblingId: string | null;
  nextSiblingId: string | null;
  depth: number;
}

export const FlowDesigner = forwardRef<FlowDesignerHandle, FlowDesignerProps>(
  function FlowDesigner(
    {
      flow: initialFlow,
      onChange,
      onSelect,
      selectedNodeId: externalSelectedId,
      onSave,
      onOpenFunctionPicker,
      onOpenAgentPicker,
      triggerType,
      disabled = false,
      showPalette = true,
      showToolbar = true,
      propertiesVisible = true,
      onToggleProperties,
      onRun,
      theme = 'light',
      className,
      triggers,
      selectedTriggerId,
      onTriggerSelect,
      onAddTrigger,
      // onEditTrigger and onDeleteTrigger available for future use
      onEditTrigger: _onEditTrigger,
      onDeleteTrigger: _onDeleteTrigger,
      executionState,
    },
    ref
  ) {
  // Flow state management
  const { flow, getState, setState } = useFlowState({
    initialFlow,
    onChange,
  });

  // Command history for undo/redo
  const { history, canUndo, canRedo, undo, redo, createContext } = useCommandHistory();
  const commandContext = useMemo(
    () => createContext(getState, setState),
    [createContext, getState, setState]
  );

  // Selection state
  const { selectedId, select, toggleSelect, clearSelection } = useSelection({
    initialSelection: externalSelectedId ? [externalSelectedId] : [],
    onSelectionChange: (selection) => onSelect?.(selection[0] ?? null),
  });

  // Zoom state
  const [zoom, setZoom] = useState(1);

  // Tool mode (select or pan)
  const [toolMode, setToolMode] = useState<'select' | 'pan'>('select');

  // External drag from palette
  const [externalDropIndicator, setExternalDropIndicator] = useState<{
    visible: boolean;
    targetId: string | null;
    insertPosition: InsertPosition;
    position: { x: number; y: number };
    size: number;
    orientation: 'horizontal' | 'vertical';
  }>({
    visible: false,
    targetId: null,
    insertPosition: 'after',
    position: { x: 0, y: 0 },
    size: 0,
    orientation: 'horizontal',
  });

  // Drag-and-drop
  const { dragState, dropIndicator, createDragHandlers, setScrollContainer } =
    useDragAndDrop({
      disabled: disabled || toolMode === 'pan',
      onDragEnd: (sourceId, targetId, insertPosition) => {
        if (targetId) {
          const command = new MoveStepCommand(commandContext, {
            sourceId,
            targetId,
            insertPosition,
          });
          history.execute(command);
        }
      },
    });

  // Keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (disabled) return;

      // Check if user is typing in an input or contentEditable element
      const isEditing =
        document.activeElement?.tagName === 'INPUT' ||
        document.activeElement?.tagName === 'TEXTAREA' ||
        (document.activeElement as HTMLElement)?.isContentEditable;

      // Undo: Ctrl/Cmd + Z
      if ((e.ctrlKey || e.metaKey) && e.key === 'z' && !e.shiftKey) {
        e.preventDefault();
        undo();
      }
      // Redo: Ctrl/Cmd + Shift + Z or Ctrl/Cmd + Y
      if ((e.ctrlKey || e.metaKey) && (e.key === 'y' || (e.key === 'z' && e.shiftKey))) {
        e.preventDefault();
        redo();
      }
      // Delete: Delete or Backspace (only when not editing)
      if ((e.key === 'Delete' || e.key === 'Backspace') && selectedId && !isEditing) {
        e.preventDefault();
        handleDelete();
      }
      // Escape: Clear selection
      if (e.key === 'Escape') {
        clearSelection();
      }
      // Space: Toggle pan mode (only when not editing)
      if (e.key === ' ' && !e.repeat && !isEditing) {
        e.preventDefault();
        setToolMode('pan');
      }
    };

    const handleKeyUp = (e: KeyboardEvent) => {
      // Check if user is typing
      const isEditing =
        document.activeElement?.tagName === 'INPUT' ||
        document.activeElement?.tagName === 'TEXTAREA' ||
        (document.activeElement as HTMLElement)?.isContentEditable;

      if (e.key === ' ' && !isEditing) {
        setToolMode('select');
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('keyup', handleKeyUp);
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('keyup', handleKeyUp);
    };
  }, [disabled, undo, redo, selectedId, clearSelection]);

  // Delete handler
  const handleDelete = useCallback(() => {
    if (!selectedId) return;
    const command = new DeleteStepCommand(commandContext, { nodeId: selectedId });
    history.execute(command);
    clearSelection();
  }, [selectedId, commandContext, history, clearSelection]);

  // Title update handler
  const handleUpdateTitle = useCallback(
    (nodeId: string, newTitle: string) => {
      const command = new UpdateStepCommand(commandContext, {
        nodeId,
        updates: { action: newTitle },
      });
      history.execute(command);
    },
    [commandContext, history]
  );

  // Unlink function handler
  const handleUnlinkFunction = useCallback(
    (nodeId: string) => {
      const command = new UpdateStepCommand(commandContext, {
        nodeId,
        updates: { function_ref: undefined },
      });
      history.execute(command);
    },
    [commandContext, history]
  );

  // Unlink agent handler
  const handleUnlinkAgent = useCallback(
    (nodeId: string) => {
      const command = new UpdateStepCommand(commandContext, {
        nodeId,
        updates: { agent_ref: undefined },
      });
      history.execute(command);
    },
    [commandContext, history]
  );

  // Set function on a step (called by parent after function picker selection)
  const setStepFunction = useCallback(
    (stepId: string, functionRef: RaisinReference) => {
      const command = new UpdateStepCommand(commandContext, {
        nodeId: stepId,
        updates: { function_ref: functionRef },
      });
      history.execute(command);
    },
    [commandContext, history]
  );

  // Set agent on a step (called by parent after agent picker selection)
  const setStepAgent = useCallback(
    (stepId: string, agentRef: RaisinReference) => {
      const command = new UpdateStepCommand(commandContext, {
        nodeId: stepId,
        updates: { agent_ref: agentRef },
      });
      history.execute(command);
    },
    [commandContext, history]
  );

  // Update step properties (title, disabled, etc.)
  const updateStepProperty = useCallback(
    (nodeId: string, updates: Partial<FlowStepProperties>) => {
      const command = new UpdateStepCommand(commandContext, {
        nodeId,
        updates,
      });
      history.execute(command);
    },
    [commandContext, history]
  );

  // Update container properties
  const updateContainer = useCallback(
    (containerId: string, updates: { container_type?: ContainerType; rules?: ContainerRule[]; ai_config?: AiContainerConfig; timeout_ms?: number }) => {
      const command = new UpdateRulesCommand(commandContext, {
        containerId,
        ...updates,
      });
      history.execute(command);
    },
    [commandContext, history]
  );

  // Zoom handlers
  const handleZoomIn = useCallback(() => {
    setZoom((z) => Math.min(3, z + 0.1));
  }, []);

  const handleZoomOut = useCallback(() => {
    setZoom((z) => Math.max(0.2, z - 0.1));
  }, []);

  // Expose imperative methods to parent
  useImperativeHandle(ref, () => ({
    setStepFunction,
    setStepAgent,
    updateStepProperty,
    updateContainer,
    getFlow: getState,
    undo,
    redo,
    deleteSelected: handleDelete,
    canUndo: () => canUndo,
    canRedo: () => canRedo,
    canDelete: () => !!selectedId,
    getZoom: () => Math.round(zoom * 100),
    zoomIn: handleZoomIn,
    zoomOut: handleZoomOut,
    getToolMode: () => toolMode,
    setToolMode,
  }), [setStepFunction, setStepAgent, updateStepProperty, getState, undo, redo, handleDelete, canUndo, canRedo, selectedId, zoom, handleZoomIn, handleZoomOut, toolMode]);

  // External drag handlers (for palette drops)
  const handleExternalDragOver = useCallback(
    (e: React.DragEvent) => {
      const nodeType = e.dataTransfer.types.includes('application/x-flow-node-type');
      if (!nodeType || disabled) return;

      e.preventDefault();
      e.dataTransfer.dropEffect = 'copy';

      const target = findDropTargetFromPoint(e.clientX, e.clientY);
      if (!target) {
        setExternalDropIndicator((prev) => ({ ...prev, visible: false, targetId: null }));
        return;
      }

      // For external drag, we can drop anywhere
      const insertPosition = calculateInsertPosition(e.clientX, e.clientY, target, null);
      if (!insertPosition) {
        setExternalDropIndicator((prev) => ({ ...prev, visible: false, targetId: null }));
        return;
      }

      const indicator = calculateDropIndicator(target.nodeId, insertPosition, target.rect);
      setExternalDropIndicator({
        visible: true,
        ...indicator,
      });
    },
    [disabled]
  );

  const handleExternalDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      const nodeType = e.dataTransfer.getData('application/x-flow-node-type') as StepType;
      if (!nodeType || disabled) return;

      const target = findDropTargetFromPoint(e.clientX, e.clientY);
      if (!target) return;

      const insertPosition = calculateInsertPosition(e.clientX, e.clientY, target, null);
      if (!insertPosition) return;

      // Create and execute AddStepCommand
      const command = new AddStepCommand(commandContext, {
        type: nodeType,
        targetId: target.nodeId,
        insertPosition,
      });
      history.execute(command);

      // Select the new node
      const newId = command.getNewNodeId();
      if (newId) {
        select(newId);
      }

      // Reset external drop indicator
      setExternalDropIndicator((prev) => ({ ...prev, visible: false, targetId: null }));
    },
    [disabled, commandContext, history, select]
  );

  const handleExternalDragLeave = useCallback(() => {
    setExternalDropIndicator((prev) => ({ ...prev, visible: false, targetId: null }));
  }, []);

  // Helper to compute execution status for a node
  const getNodeExecutionStatus = useCallback(
    (nodeId: string): NodeExecutionStatus => {
      if (!executionState?.isExecuting) return 'idle';
      if (executionState.currentNodeId === nodeId) return 'running';
      if (executionState.waitingNodeId === nodeId) return 'waiting';
      if (executionState.failedNodeIds.has(nodeId)) return 'failed';
      if (executionState.completedNodeIds.has(nodeId)) return 'completed';
      return 'idle';
    },
    [executionState]
  );

  // Render a flow node recursively
  const renderNode = useCallback(
    function renderNodeInternal(
      node: FlowNode,
      context: NodeRenderContext
    ): React.ReactNode {
      const isSelected = selectedId === node.id;
      const isDragging = dragState.draggedNodeId === node.id;
      const dragHandlers = toolMode === 'select' ? createDragHandlers(node.id, node) : undefined;
      const executionStatus = getNodeExecutionStatus(node.id);

      if (isFlowStep(node)) {
        return (
          <StepNode
            key={node.id}
            node={node}
            parentId={context.parentId}
            nextSiblingId={context.nextSiblingId}
            selected={isSelected}
            executionStatus={executionStatus}
            onClick={() => toggleSelect(node.id)}
            onOpenFunctionPicker={() => onOpenFunctionPicker?.(node.id)}
            onOpenAgentPicker={() => onOpenAgentPicker?.(node.id)}
            onUnlinkFunction={handleUnlinkFunction}
            onUnlinkAgent={handleUnlinkAgent}
            onUpdateTitle={handleUpdateTitle}
            onAddStep={(position) => {
              const command = new AddStepCommand(commandContext, {
                type: 'step',
                targetId: node.id,
                insertPosition: position,
              });
              history.execute(command);
              // Select the new node
              const newId = command.getNewNodeId();
              if (newId) {
                select(newId);
              }
            }}
            dragHandlers={dragHandlers}
            disabled={disabled || isDragging}
          />
        );
      }

      if (isFlowContainer(node)) {
        return (
          <ContainerNode
            key={node.id}
            node={node}
            parentId={context.parentId}
            nextSiblingId={context.nextSiblingId}
            selected={isSelected}
            onClick={() => select(node.id)}
            onAddChild={() => {
              const command = new AddStepCommand(commandContext, {
                type: 'step',
                targetId: node.id,
                insertPosition: 'inside',
              });
              history.execute(command);
            }}
            onDelete={() => {
              const command = new DeleteStepCommand(commandContext, { nodeId: node.id });
              history.execute(command);
              if (selectedId === node.id) {
                clearSelection();
              }
            }}
            dragHandlers={dragHandlers}
            renderChild={(child, index, siblings) =>
              renderNodeInternal(child, {
                parentId: node.id,
                prevSiblingId: index > 0 ? siblings[index - 1]?.id ?? null : null,
                nextSiblingId:
                  index < siblings.length - 1
                    ? siblings[index + 1]?.id ?? null
                    : null,
                depth: context.depth + 1,
              })
            }
            highlightDropTarget={
              dropIndicator.visible &&
              dropIndicator.targetId === node.id &&
              dropIndicator.insertPosition === 'inside'
            }
            isDragging={dragState.isDragging}
            disabled={disabled || isDragging}
          />
        );
      }

      return null;
    },
    [
      selectedId,
      dragState.draggedNodeId,
      dragState.isDragging,
      toolMode,
      createDragHandlers,
      select,
      toggleSelect,
      onOpenFunctionPicker,
      onOpenAgentPicker,
      handleUpdateTitle,
      handleUnlinkFunction,
      handleUnlinkAgent,
      disabled,
      commandContext,
      history,
      dropIndicator,
      clearSelection,
      getNodeExecutionStatus,
    ]
  );

  return (
    <ThemeProvider theme={theme}>
      <FlowDesignerContent
        className={className}
        onDragOver={handleExternalDragOver}
        onDrop={handleExternalDrop}
        onDragLeave={handleExternalDragLeave}
        disabled={disabled}
        showPalette={showPalette}
        showToolbar={showToolbar}
        onSave={onSave}
        undo={undo}
        redo={redo}
        canUndo={canUndo}
        canRedo={canRedo}
        handleDelete={handleDelete}
        selectedId={selectedId}
        toolMode={toolMode}
        setToolMode={setToolMode}
        handleZoomIn={handleZoomIn}
        handleZoomOut={handleZoomOut}
        zoom={zoom}
        setZoom={setZoom}
        propertiesVisible={propertiesVisible}
        onToggleProperties={onToggleProperties}
        onRun={onRun}
        setScrollContainer={setScrollContainer}
        clearSelection={clearSelection}
        triggerType={triggerType}
        flow={flow}
        commandContext={commandContext}
        history={history}
        select={select}
        renderNode={renderNode}
        dropIndicator={dropIndicator}
        externalDropIndicator={externalDropIndicator}
        dragState={dragState}
        triggers={triggers}
        selectedTriggerId={selectedTriggerId}
        onTriggerSelect={onTriggerSelect}
        onAddTrigger={onAddTrigger}
      />
    </ThemeProvider>
  );
});

// Inner component that uses theme context
interface FlowDesignerContentProps {
  className?: string;
  onDragOver: (e: React.DragEvent) => void;
  onDrop: (e: React.DragEvent) => void;
  onDragLeave: () => void;
  disabled: boolean;
  showPalette: boolean;
  showToolbar: boolean;
  onSave?: () => void;
  undo: () => void;
  redo: () => void;
  canUndo: boolean;
  canRedo: boolean;
  handleDelete: () => void;
  selectedId: string | null;
  toolMode: 'select' | 'pan';
  setToolMode: (mode: 'select' | 'pan') => void;
  handleZoomIn: () => void;
  handleZoomOut: () => void;
  zoom: number;
  setZoom: (zoom: number) => void;
  propertiesVisible: boolean;
  onToggleProperties?: () => void;
  onRun?: () => void;
  setScrollContainer: (el: HTMLElement | null) => void;
  clearSelection: () => void;
  triggerType?: 'node_event' | 'schedule' | 'http';
  flow: FlowDefinition;
  commandContext: CommandContext;
  history: { execute: (command: AbstractCommand) => void };
  select: (id: string) => void;
  renderNode: (node: FlowNode, context: NodeRenderContext) => React.ReactNode;
  dropIndicator: { visible: boolean; targetId: string | null; insertPosition: InsertPosition; position: { x: number; y: number }; size: number; orientation: 'horizontal' | 'vertical' };
  externalDropIndicator: { visible: boolean; targetId: string | null; insertPosition: InsertPosition; position: { x: number; y: number }; size: number; orientation: 'horizontal' | 'vertical' };
  dragState: { isDragging: boolean; draggedNodeId: string | null; draggedNode: FlowNode | null; ghostPosition: { x: number; y: number } | null };
  triggers?: TriggerInfo[];
  selectedTriggerId?: string | null;
  onTriggerSelect?: (triggerId: string | null) => void;
  onAddTrigger?: () => void;
}

function FlowDesignerContent({
  className,
  onDragOver,
  onDrop,
  onDragLeave,
  disabled,
  showPalette,
  showToolbar,
  onSave,
  undo,
  redo,
  canUndo,
  canRedo,
  handleDelete,
  selectedId,
  toolMode,
  setToolMode,
  handleZoomIn,
  handleZoomOut,
  zoom,
  setZoom,
  propertiesVisible,
  onToggleProperties,
  onRun,
  setScrollContainer,
  clearSelection,
  triggerType,
  flow,
  commandContext,
  history,
  select,
  renderNode,
  dropIndicator,
  externalDropIndicator,
  dragState,
  triggers,
  selectedTriggerId,
  onTriggerSelect,
  onAddTrigger,
}: FlowDesignerContentProps) {
  const themeClasses = useThemeClasses();

  return (
    <div
      className={clsx('flex flex-col h-full bg-transparent relative', className)}
      onDragOver={onDragOver}
      onDrop={onDrop}
      onDragLeave={onDragLeave}
    >
      {/* Node Palette */}
      {showPalette && (
        <NodePalette disabled={disabled} />
      )}

      {/* Toolbar */}
      {showToolbar && (
        <FlowToolbar
          onSave={onSave}
          onUndo={undo}
          onRedo={redo}
          canUndo={canUndo}
          canRedo={canRedo}
          onDelete={handleDelete}
          canDelete={!!selectedId}
          toolMode={toolMode}
          onToolModeChange={setToolMode}
          onZoomIn={handleZoomIn}
          onZoomOut={handleZoomOut}
          currentZoom={Math.round(zoom * 100)}
          propertiesVisible={propertiesVisible}
          onToggleProperties={onToggleProperties}
          onRun={onRun}
        />
      )}

      {/* Canvas */}
      <FlowCanvas
        zoom={zoom}
        onZoomChange={setZoom}
        setScrollContainer={setScrollContainer}
        onBackgroundClick={clearSelection}
        panMode={toolMode === 'pan'}
        className="flex-1"
      >
        <div className="flex flex-col items-center min-w-max">
          {/* Trigger nodes (displayed before start) */}
          {(triggers && triggers.length > 0) || onAddTrigger ? (
            <>
              <div className="flex flex-wrap gap-3 justify-center mb-2">
                {triggers?.map((trigger) => (
                  <TriggerNode
                    key={trigger.id}
                    id={trigger.id}
                    name={trigger.name}
                    triggerType={trigger.triggerType}
                    enabled={trigger.enabled}
                    webhookId={trigger.webhookId}
                    selected={selectedTriggerId === trigger.id}
                    onClick={() => onTriggerSelect?.(
                      selectedTriggerId === trigger.id ? null : trigger.id
                    )}
                  />
                ))}
                {onAddTrigger && !disabled && (
                  <AddTriggerButton onClick={onAddTrigger} disabled={disabled} />
                )}
              </div>
              <VerticalConnector height={24} />
            </>
          ) : null}

          {/* Start node */}
          <StartNode triggerType={triggerType} />

          {/* Connector to first node with add button */}
          <ConnectorWithButton
            height={48}
            onAdd={() => {
              const command = new AddStepCommand(commandContext, {
                type: 'step',
                targetId: flow.nodes[0]?.id ?? null,
                insertPosition: flow.nodes.length > 0 ? 'before' : 'after',
              });
              history.execute(command);
              const newId = command.getNewNodeId();
              if (newId) select(newId);
            }}
          />

          {/* Flow nodes */}
          {flow.nodes.length > 0 ? (
            flow.nodes.map((node, index, siblings) => (
              <Fragment key={node.id}>
                {renderNode(node, {
                  parentId: null,
                  prevSiblingId: index > 0 ? siblings[index - 1]?.id ?? null : null,
                  nextSiblingId:
                    index < siblings.length - 1
                      ? siblings[index + 1]?.id ?? null
                      : null,
                  depth: 0,
                })}
                <ConnectorWithButton
                  height={48}
                  onAdd={() => {
                    const command = new AddStepCommand(commandContext, {
                      type: 'step',
                      targetId: node.id,
                      insertPosition: 'after',
                    });
                    history.execute(command);
                    const newId = command.getNewNodeId();
                    if (newId) select(newId);
                  }}
                />
              </Fragment>
            ))
          ) : (
            <div className="flex flex-col items-center">
              <div className={clsx(
                'text-center p-6 border-2 border-dashed rounded-lg',
                themeClasses.emptyBorder,
                themeClasses.emptyBg
              )}>
                <p className={clsx('text-sm mb-2', themeClasses.emptyText)}>No steps yet</p>
                <button
                  onClick={() => {
                    const command = new AddStepCommand(commandContext, {
                      type: 'step',
                      targetId: null,
                      insertPosition: 'after',
                    });
                    history.execute(command);
                    const newId = command.getNewNodeId();
                    if (newId) select(newId);
                  }}
                  className="text-blue-500 hover:text-blue-700 font-medium text-sm"
                  disabled={disabled}
                >
                  Add your first step
                </button>
              </div>
              <VerticalConnector height={40} />
            </div>
          )}

          {/* End node */}
          <EndNode />
        </div>
      </FlowCanvas>

      {/* Drop indicator overlay (internal drag) */}
      <DropIndicator {...dropIndicator} />

      {/* External drop indicator (palette drag) */}
      {externalDropIndicator.visible && (
        <DropIndicator {...externalDropIndicator} />
      )}

      {/* Ghost node during drag */}
      {dragState.isDragging && dragState.draggedNode && dragState.ghostPosition && (
        <GhostNode
          node={dragState.draggedNode}
          position={dragState.ghostPosition}
        />
      )}
    </div>
  );
}
