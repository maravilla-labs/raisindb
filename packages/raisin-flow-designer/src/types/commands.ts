/**
 * Command Pattern Type Definitions
 *
 * Types for the undo/redo command system.
 */

import type {
  FlowDefinition,
  FlowNode,
  ContainerRule,
  ContainerType,
  StepErrorBehavior,
  RaisinReference,
  AiContainerConfig,
  TaskOption,
  RetryConfig,
  RetryStrategy,
  ChatStepConfig,
} from './flow';
import type { InsertPosition } from './dnd';

/** Context provided to commands for state access */
export interface CommandContext {
  /** Get current flow state */
  getState: () => FlowDefinition;
  /** Update flow state with a reducer function */
  setState: (updater: (prev: FlowDefinition) => FlowDefinition) => void;
}

/** Step types that can be added */
export type StepType = 'step' | 'ai_agent' | 'human_task' | 'chat' | 'and' | 'or' | 'parallel' | 'ai_sequence';

/** Parameters for AddStepCommand */
export interface AddStepParams {
  /** Type of step to add */
  type: StepType;
  /** ID of target node (null = append to root) */
  targetId: string | null;
  /** Where to insert relative to target */
  insertPosition: InsertPosition;
}

/** Parameters for DeleteStepCommand */
export interface DeleteStepParams {
  /** ID of node to delete */
  nodeId: string;
  /** Whether to preserve children (move to parent) */
  preserveChildren?: boolean;
}

/** Parameters for MoveStepCommand */
export interface MoveStepParams {
  /** ID of node to move */
  sourceId: string;
  /** ID of target node */
  targetId: string;
  /** Where to insert relative to target */
  insertPosition: InsertPosition;
}

/** Parameters for UpdateStepCommand */
export interface UpdateStepParams {
  /** ID of node to update */
  nodeId: string;
  /** Properties to update */
  updates: {
    action?: string;
    /** Function reference in raisin:ref format */
    function_ref?: RaisinReference;
    /** Agent reference in raisin:ref format */
    agent_ref?: RaisinReference;
    lua_script?: string;
    condition?: string;
    payload_key?: string;
    disabled?: boolean;
    on_error?: StepErrorBehavior;
    // Human task properties
    task_type?: 'approval' | 'input' | 'review' | 'action';
    assignee?: string;
    task_description?: string;
    options?: TaskOption[];
    priority?: number;
    due_in_seconds?: number;
    // Retry configuration
    retry_strategy?: RetryStrategy;
    retry?: RetryConfig;
    timeout_ms?: number;
    // Error handling and execution identity
    error_edge?: string;
    compensation_ref?: RaisinReference;
    continue_on_fail?: boolean;
    isolated_branch?: boolean;
    execution_identity?: 'agent' | 'caller' | 'function';
    // Chat step configuration
    chat_config?: ChatStepConfig;
  };
}

/** Parameters for UpdateContainerCommand */
export interface UpdateContainerParams {
  /** ID of container to update */
  containerId: string;
  /** Container type */
  container_type?: ContainerType;
  /** Container rules */
  rules?: ContainerRule[];
  /** AI container configuration (for ai_sequence containers) */
  ai_config?: AiContainerConfig;
  /** Container timeout in milliseconds */
  timeout_ms?: number;
}

/** Command type identifiers */
export type CommandType =
  | 'ADD_STEP'
  | 'DELETE_STEP'
  | 'MOVE_STEP'
  | 'UPDATE_STEP'
  | 'UPDATE_CONTAINER';

/** Command metadata for history display */
export interface CommandMetadata {
  /** Command type */
  type: CommandType;
  /** Human-readable description */
  description: string;
  /** Timestamp when command was executed */
  timestamp: number;
}

/** Template shapes for new steps */
export interface StepTemplate {
  node_type: 'raisin:FlowStep' | 'raisin:FlowContainer';
  container_type?: ContainerType;
  properties?: {
    action?: string;
    disabled?: boolean;
    step_type?: 'default' | 'ai_agent' | 'human_task' | 'chat';
    retry_strategy?: RetryStrategy;
  };
  children?: FlowNode[];
}

/** Map of step types to their templates */
export const STEP_TEMPLATES: Record<StepType, StepTemplate> = {
  step: {
    node_type: 'raisin:FlowStep',
    properties: {
      action: 'New Step',
      disabled: false,
      retry_strategy: 'none',
    },
  },
  ai_agent: {
    node_type: 'raisin:FlowStep',
    properties: {
      action: 'AI Agent',
      step_type: 'ai_agent',
      disabled: false,
      retry_strategy: 'none',
    },
  },
  human_task: {
    node_type: 'raisin:FlowStep',
    properties: {
      action: 'Human Task',
      step_type: 'human_task',
      disabled: false,
      retry_strategy: 'none',
    },
  },
  chat: {
    node_type: 'raisin:FlowStep',
    properties: {
      action: 'Chat Session',
      step_type: 'chat',
      disabled: false,
      retry_strategy: 'none',
    },
  },
  and: {
    node_type: 'raisin:FlowContainer',
    container_type: 'and',
    children: [],
  },
  or: {
    node_type: 'raisin:FlowContainer',
    container_type: 'or',
    children: [],
  },
  parallel: {
    node_type: 'raisin:FlowContainer',
    container_type: 'parallel',
    children: [],
  },
  ai_sequence: {
    node_type: 'raisin:FlowContainer',
    container_type: 'ai_sequence',
    children: [],
  },
};
