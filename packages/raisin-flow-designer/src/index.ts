/**
 * @raisindb/flow-designer
 *
 * Visual workflow designer component for RaisinDB.
 */

// Main components
export { FlowDesigner, type FlowDesignerProps, type FlowDesignerHandle, type ExecutionState, type NodeExecutionStatus } from './components';

// All components for customization
export {
  FlowToolbar,
  FlowCanvas,
  StartNode,
  EndNode,
  StepNode,
  ContainerNode,
  EmptyDropZone,
  VerticalConnector,
  HorizontalConnector,
  ConnectorWithButton,
  ContainerTypeIcon,
  DropIndicator,
  GhostNode,
  DraggableNode,
  NodePalette,
  Tooltip,
} from './components';

// Hooks for custom implementations
export {
  useDragAndDrop,
  useCommandHistory,
  useFlowState,
  useSelection,
  useAutoScroll,
  useFlowExecution,
  useFlowValidation,
} from './hooks';

// Flow execution types
export type {
  UseFlowExecutionOptions,
  UseFlowExecutionResult,
  TestRunConfig,
  MockFunctionConfig,
} from './hooks';

// Flow validation types
export type {
  UseFlowValidationOptions,
  UseFlowValidationReturn,
  FlowValidator,
} from './hooks';

// Validation types from context
export type {
  ValidationResult,
  ValidationIssue,
} from './context';

// Commands for custom integrations
export {
  AbstractCommand,
  CommandHistory,
  AddStepCommand,
  DeleteStepCommand,
  MoveStepCommand,
  UpdateStepCommand,
  UpdateRulesCommand,
} from './commands';

// Types
export type {
  // Flow types
  FlowDefinition,
  FlowNode,
  FlowStep,
  FlowContainer,
  FlowStepProperties,
  RaisinReference,
  ContainerType,
  ContainerRule,
  StepErrorBehavior,
  FlowErrorStrategy,
  // Retry types
  RetryConfig,
  RetryStrategy,
  // AI Container types
  AiToolMode,
  AiErrorBehavior,
  AiContainerConfig,
  // Execution state types
  StepExecutionState,
  StepExecutionInfo,
  FlowExecutionStatus,
  FlowExecutionState,
  ExecutionLogEntry,
  FlowExecutionEvent,
  // DnD types
  DragState,
  DropIndicatorState,
  DropOrientation,
  InsertPosition,
  DragDropConfig,
  // Command types
  CommandContext,
  StepType,
  AddStepParams,
  DeleteStepParams,
  MoveStepParams,
  UpdateStepParams,
  UpdateContainerParams,
} from './types';

// Type guards and helpers
export { isFlowStep, isFlowContainer, getRefDisplayName, getRefPath } from './types';

// Theme
export type { FlowTheme } from './context';

// Constants
export {
  CONTAINER_TYPE_LABELS,
  CONTAINER_TYPE_DESCRIPTIONS,
  STEP_TEMPLATES,
  DEFAULT_DRAG_DROP_CONFIG,
  RETRY_STRATEGIES,
  RETRY_STRATEGY_DESCRIPTIONS,
  DEFAULT_AI_CONTAINER_CONFIG,
  AI_TOOL_MODE_DESCRIPTIONS,
  INITIAL_EXECUTION_STATE,
} from './types';

// Utility functions
export {
  findNodeById,
  findNodeAndParent,
  removeNodeById,
  insertNode,
  cloneFlow,
  cloneNode,
  createEmptyFlow,
  countNodes,
  getAllNodeIds,
  calculateInsertPosition,
  calculateDropIndicator,
  generateNodeId,
  generateStepId,
  generateContainerId,
} from './utils';
