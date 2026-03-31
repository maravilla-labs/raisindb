/**
 * Components Module
 *
 * Re-exports all components.
 */

// Main components
export { FlowDesigner, type FlowDesignerProps, type FlowDesignerHandle, type ExecutionState, type NodeExecutionStatus } from './FlowDesigner';
export { FlowToolbar, type FlowToolbarProps } from './FlowToolbar';
export { FlowCanvas, type FlowCanvasProps } from './FlowCanvas';

// Node components
export {
  StartNode,
  EndNode,
  StepNode,
  ContainerNode,
  EmptyDropZone,
  type StartNodeProps,
  type EndNodeProps,
  type StepNodeProps,
  type ContainerNodeProps,
  type EmptyDropZoneProps,
} from './nodes';

// Connection components
export {
  VerticalConnector,
  HorizontalConnector,
  ConnectorWithButton,
  ContainerTypeIcon,
  ErrorEdge,
  type VerticalConnectorProps,
  type HorizontalConnectorProps,
  type ConnectorWithButtonProps,
  type ContainerTypeIconProps,
  type ErrorEdgeProps,
} from './connections';

// DnD components
export {
  DropIndicator,
  GhostNode,
  DraggableNode,
  type DropIndicatorProps,
  type GhostNodeProps,
  type DraggableNodeProps,
} from './dnd';

// Node Palette
export { NodePalette, type NodePaletteProps } from './NodePalette';

// Properties Panel
export { PropertiesPanel, type PropertiesPanelProps } from './PropertiesPanel';
export {
  StepPropertiesEditor,
  ErrorHandlingEditor,
  type StepPropertiesEditorProps,
  type ErrorHandlingEditorProps,
} from './properties';

// UI components
export {
  Tooltip,
  ValidationBadge,
  ValidationIssueBadge,
  ProblemsPanel,
  type TooltipProps,
  type TooltipPosition,
  type ValidationBadgeProps,
  type ValidationIssueBadgeProps,
  type ProblemsPanelProps,
} from './ui';
