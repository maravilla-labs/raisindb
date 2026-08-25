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
  ContainerRouterConfig,
  ContainerRefereeConfig,
  LoopConfig,
  FanOutConfig,
  MergeStrategy,
  TaskOption,
  TaskTypeSlug,
  RetryConfig,
  RetryStrategy,
  ChatStepConfig,
} from './flow';
import { DEFAULT_LOOP_CONFIG } from './flow';
import type { InsertPosition } from './dnd';

/** Context provided to commands for state access */
export interface CommandContext {
  /** Get current flow state */
  getState: () => FlowDefinition;
  /** Update flow state with a reducer function */
  setState: (updater: (prev: FlowDefinition) => FlowDefinition) => void;
}

/** Step types that can be added */
export type StepType = 'step' | 'ai_agent' | 'human_task' | 'chat' | 'and' | 'or' | 'parallel' | 'ai_sequence' | 'competition' | 'loop';

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
    task_type?: TaskTypeSlug;
    assignee?: string;
    task_description?: string;
    options?: TaskOption[];
    /** Number, or a template expression resolved at run time */
    priority?: number | string;
    /** Number, or a template expression resolved at run time */
    due_in_seconds?: number | string;
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
  /** AI router for OR containers (null removes the router) */
  router?: ContainerRouterConfig | null;
  /** Referee for competition containers (null removes the referee) */
  referee?: ContainerRefereeConfig | null;
  /** Loop configuration for loop containers (null removes the config) */
  loop?: LoopConfig | null;
  /** Fan-out configuration for parallel containers (null removes the config) */
  fan_out?: FanOutConfig | null;
  /** Branch join strategy for parallel containers */
  merge_strategy?: MergeStrategy;
  /** Shared task prompt for competition containers (null removes the prompt) */
  prompt?: string | null;
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
  /** Seed config for a loop container, so a dropped loop is editable at once. */
  loop?: LoopConfig;
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
  competition: {
    node_type: 'raisin:FlowContainer',
    container_type: 'competition',
    children: [],
  },
  loop: {
    node_type: 'raisin:FlowContainer',
    container_type: 'loop',
    children: [],
    // Seed the config so a newly dropped loop lands in a shape the editor can
    // show and the validator can talk about. Without it the container arrived
    // with no `loop` at all, which reads as "not configured" everywhere — and
    // `DEFAULT_LOOP_CONFIG` deliberately does NOT name a shape, so the author
    // still chooses between for-each / while / times.
    loop: { ...DEFAULT_LOOP_CONFIG },
  },
};
