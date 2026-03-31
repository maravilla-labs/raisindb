/**
 * Flow Designer Type Definitions
 *
 * Data model for visual workflow definitions.
 */

/** Container types matching Svelte implementation */
export type ContainerType = 'and' | 'or' | 'parallel' | 'ai_sequence';

/** Step error behavior */
export type StepErrorBehavior = 'stop' | 'skip' | 'continue';

/**
 * RaisinDB reference type (PropertyValue::Reference)
 * Used for cross-node references with workspace context
 */
export interface RaisinReference {
  /** Node ID or path (raisin:ref) */
  'raisin:ref': string;
  /** Workspace context */
  'raisin:workspace': string;
  /** Optional resolved path */
  'raisin:path'?: string;
}

/**
 * Helper to extract display name from a reference path
 */
export function getRefDisplayName(ref: RaisinReference): string {
  const path = ref['raisin:path'] || ref['raisin:ref'];
  // Get last segment of path as display name
  const segments = path.split('/').filter(Boolean);
  return segments[segments.length - 1] || path;
}

/**
 * Helper to get the full path from a reference
 */
export function getRefPath(ref: RaisinReference): string {
  return ref['raisin:path'] || ref['raisin:ref'];
}

/** Flow-level error strategy */
export type FlowErrorStrategy = 'fail_fast' | 'continue';

/** Base flow node interface */
export interface FlowNodeBase {
  id: string;
  node_type: 'raisin:FlowStep' | 'raisin:FlowContainer';
}

/** Basic workflow step */
export interface FlowStep extends FlowNodeBase {
  node_type: 'raisin:FlowStep';
  properties: FlowStepProperties;
  on_error?: StepErrorBehavior;
  /** Target node ID for error flow (error edge at node level) */
  error_edge?: string;
}

/** Retry configuration for a step */
export interface RetryConfig {
  /** Maximum number of retry attempts (0 = no retries) */
  max_retries: number;
  /** Base delay in milliseconds for exponential backoff */
  base_delay_ms: number;
  /** Maximum delay cap in milliseconds */
  max_delay_ms: number;
}

/** Predefined retry strategies */
export const RETRY_STRATEGIES = {
  none: { max_retries: 0, base_delay_ms: 0, max_delay_ms: 0 },
  quick: { max_retries: 3, base_delay_ms: 1000, max_delay_ms: 10000 },
  standard: { max_retries: 5, base_delay_ms: 2000, max_delay_ms: 60000 },
  aggressive: { max_retries: 10, base_delay_ms: 5000, max_delay_ms: 120000 },
  llm: { max_retries: 5, base_delay_ms: 10000, max_delay_ms: 120000 },
} as const;

export type RetryStrategy = keyof typeof RETRY_STRATEGIES;

/** Retry strategy descriptions */
export const RETRY_STRATEGY_DESCRIPTIONS: Record<RetryStrategy, string> = {
  none: 'No retries - fail immediately on error',
  quick: '3 retries with 1s base delay (transient failures)',
  standard: '5 retries with 2s base delay (most operations)',
  aggressive: '10 retries with 5s base delay (critical ops)',
  llm: '5 retries with 10s base delay (optimized for LLM rate limits)',
};

/** Human task option for approval tasks */
export interface TaskOption {
  /** Option value returned when selected */
  value: string;
  /** Display label */
  label: string;
  /** Visual style */
  style?: 'default' | 'success' | 'danger' | 'warning';
}

/** Step properties */
export interface FlowStepProperties {
  /** Action name/label */
  action?: string;
  /** Function reference (raisin:ref format) */
  function_ref?: RaisinReference;
  /** Agent reference (raisin:ref format) */
  agent_ref?: RaisinReference;
  /** Lua script for evaluation */
  lua_script?: string;
  /** Condition expression (REL) */
  condition?: string;
  /** Key for payload data */
  payload_key?: string;
  /** Whether step is disabled */
  disabled?: boolean;
  /** Step type - distinguishes AI agent, chat, and human task steps from regular steps */
  step_type?: 'default' | 'ai_agent' | 'human_task' | 'chat';
  /** Retry configuration for this step */
  retry?: RetryConfig;
  /** Retry strategy preset (alternative to custom retry config) */
  retry_strategy?: RetryStrategy;
  /** Step timeout in milliseconds */
  timeout_ms?: number;

  // Error handling properties

  /** Target node ID for error flow (error edge) */
  error_edge?: string;
  /** Reference to compensation function for saga rollback */
  compensation_ref?: RaisinReference;
  /** Continue workflow on step failure */
  continue_on_fail?: boolean;
  /** Execute step in isolated git-like branch for safety */
  isolated_branch?: boolean;
  /** Execution identity mode for permission handling (FR-028) */
  execution_identity?: 'agent' | 'caller' | 'function';

  // Human task specific properties (when step_type = 'human_task')

  /** Type of human task */
  task_type?: 'approval' | 'input' | 'review' | 'action';
  /** User path to assign the task to */
  assignee?: string;
  /** Task description */
  task_description?: string;
  /** Options for approval tasks */
  options?: TaskOption[];
  /** JSON schema for input tasks */
  input_schema?: object;
  /** Task due time in seconds from creation */
  due_in_seconds?: number;
  /** Task priority (1-5, where 5 is highest) */
  priority?: number;

  // Chat step specific properties (when step_type = 'chat')

  /** Chat step configuration */
  chat_config?: ChatStepConfig;
}

/** Handoff target for chat steps */
export interface HandoffTarget {
  /** Target agent reference */
  agent_ref: RaisinReference;
  /** Trigger phrase patterns */
  trigger_phrases?: string[];
  /** When to trigger handoff */
  trigger_condition?: string;
}

/** Chat termination mode */
export type ChatTerminationMode = 'user_request' | 'max_turns' | 'inactivity' | 'ai_decision';

/** Chat termination configuration */
export interface ChatTerminationConfig {
  /** Termination modes that end the chat */
  modes: ChatTerminationMode[];
  /** Inactivity timeout in milliseconds */
  inactivity_timeout_ms?: number;
  /** Termination phrases to detect */
  termination_phrases?: string[];
}

/** Chat step configuration */
export interface ChatStepConfig {
  /** Reference to the agent node */
  agent_ref?: RaisinReference;
  /** System prompt override */
  system_prompt?: string;
  /** Agents to hand off to */
  handoff_targets: HandoffTarget[];
  /** Session timeout in milliseconds */
  session_timeout_ms?: number;
  /** Maximum number of conversation turns */
  max_turns: number;
  /** Termination configuration */
  termination: ChatTerminationConfig;
}

/** Default chat step configuration */
export const DEFAULT_CHAT_STEP_CONFIG: ChatStepConfig = {
  handoff_targets: [],
  max_turns: 20,
  termination: {
    modes: ['user_request', 'max_turns'],
  },
};

/** Tool execution mode for AI container */
export type AiToolMode = 'auto' | 'explicit' | 'hybrid';

/** AI error handling behavior */
export type AiErrorBehavior = 'stop' | 'continue' | 'retry';

/** AI Container configuration (for ai_sequence containers) */
export interface AiContainerConfig {
  /** Reference to the agent node */
  agent_ref?: RaisinReference;
  /** Tool execution mode */
  tool_mode: AiToolMode;
  /** Tools to expose as explicit steps (for hybrid mode) */
  explicit_tools: string[];
  /** Maximum iterations for tool call loops */
  max_iterations: number;
  /** Enable AI thinking/reasoning display */
  thinking_enabled: boolean;
  /** Reference to existing conversation to continue */
  conversation_ref?: RaisinReference;
  /** Error handling behavior */
  on_error: AiErrorBehavior;
  /** Maximum retries on transient AI failures (default: 2) */
  max_retries?: number;
  /** Base delay between retries in milliseconds (exponential backoff, default: 1000) */
  retry_delay_ms?: number;
  /** Timeout for entire container execution in milliseconds */
  timeout_ms?: number;
  /** Response format: "text", "json_object", or "json_schema" */
  response_format?: string;
  /** JSON schema for structured output (when response_format = "json_schema") */
  output_schema?: unknown;
}

/** Default AI container configuration */
export const DEFAULT_AI_CONTAINER_CONFIG: AiContainerConfig = {
  tool_mode: 'auto',
  explicit_tools: [],
  max_iterations: 10,
  thinking_enabled: false,
  on_error: 'stop',
};

/** Tool mode descriptions */
export const AI_TOOL_MODE_DESCRIPTIONS: Record<AiToolMode, string> = {
  auto: 'Agent handles tool calls internally',
  explicit: 'Tool calls appear as child steps',
  hybrid: 'Some tools internal, others explicit',
};

/** Container node with children (AND/OR/Parallel/AI) */
export interface FlowContainer extends FlowNodeBase {
  node_type: 'raisin:FlowContainer';
  container_type: ContainerType;
  rules?: ContainerRule[];
  children: FlowNode[];
  /** AI container configuration (only for ai_sequence type) */
  ai_config?: AiContainerConfig;
  /** Container timeout in milliseconds */
  timeout_ms?: number;
}

/** Union type for all flow nodes */
export type FlowNode = FlowStep | FlowContainer;

/** Container rule for conditional execution */
export interface ContainerRule {
  /** Lua condition expression */
  condition: string;
  /** ID of next step if condition matches */
  next_step: string;
}

/** Complete flow definition */
export interface FlowDefinition {
  /** Schema version */
  version: number;
  /** Error handling strategy */
  error_strategy: FlowErrorStrategy;
  /** Global timeout in milliseconds */
  timeout_ms?: number;
  /** Root workflow nodes */
  nodes: FlowNode[];
}

/** Type guard for FlowStep */
export function isFlowStep(node: FlowNode): node is FlowStep {
  return node.node_type === 'raisin:FlowStep';
}

/** Type guard for FlowContainer */
export function isFlowContainer(node: FlowNode): node is FlowContainer {
  return node.node_type === 'raisin:FlowContainer';
}

/** Container type display names */
export const CONTAINER_TYPE_LABELS: Record<ContainerType, string> = {
  and: 'AND',
  or: 'OR',
  parallel: 'Parallel',
  ai_sequence: 'AI Sequence',
};

/** Container type descriptions */
export const CONTAINER_TYPE_DESCRIPTIONS: Record<ContainerType, string> = {
  and: 'All children must pass',
  or: 'Any child can pass',
  parallel: 'Execute children concurrently',
  ai_sequence: 'AI-orchestrated execution',
};

// ============================================================================
// Execution State Types (for real-time flow visualization)
// ============================================================================

/** Execution state of a step */
export type StepExecutionState = 'idle' | 'running' | 'completed' | 'failed' | 'waiting' | 'skipped';

/** Step execution info for real-time tracking */
export interface StepExecutionInfo {
  /** Current execution state */
  state: StepExecutionState;
  /** When execution started */
  startedAt?: Date;
  /** When execution completed/failed */
  endedAt?: Date;
  /** Duration in milliseconds */
  durationMs?: number;
  /** Step output (if completed) */
  output?: unknown;
  /** Error message (if failed) */
  error?: string;
  /** Wait reason (if waiting) */
  waitReason?: string;
}

/** Flow execution status */
export type FlowExecutionStatus = 'idle' | 'running' | 'completed' | 'failed' | 'waiting' | 'cancelled';

/** Flow execution state */
export interface FlowExecutionState {
  /** Flow instance ID */
  instanceId?: string;
  /** Overall execution status */
  status: FlowExecutionStatus;
  /** Execution state per step (keyed by node ID) */
  steps: Record<string, StepExecutionInfo>;
  /** Currently executing node ID */
  currentNodeId?: string;
  /** When execution started */
  startedAt?: Date;
  /** When execution ended */
  endedAt?: Date;
  /** Total duration in milliseconds */
  totalDurationMs?: number;
  /** Final output (if completed) */
  output?: unknown;
  /** Error message (if failed) */
  error?: string;
  /** Execution logs */
  logs: ExecutionLogEntry[];
}

/** Execution log entry */
export interface ExecutionLogEntry {
  /** Timestamp */
  timestamp: Date;
  /** Log level */
  level: 'debug' | 'info' | 'warn' | 'error';
  /** Log message */
  message: string;
  /** Related node ID */
  nodeId?: string;
}

/** Initial execution state */
export const INITIAL_EXECUTION_STATE: FlowExecutionState = {
  status: 'idle',
  steps: {},
  logs: [],
};

// ============================================================================
// Flow Execution Events (matching backend FlowExecutionEvent)
// ============================================================================

/** Base event properties */
interface BaseEvent {
  timestamp: string;
}

/** Step started event */
export interface StepStartedEvent extends BaseEvent {
  type: 'step_started';
  node_id: string;
  step_name?: string;
  step_type: string;
}

/** Step completed event */
export interface StepCompletedEvent extends BaseEvent {
  type: 'step_completed';
  node_id: string;
  output: unknown;
  duration_ms: number;
}

/** Step failed event */
export interface StepFailedEvent extends BaseEvent {
  type: 'step_failed';
  node_id: string;
  error: string;
  duration_ms: number;
}

/** Flow waiting event */
export interface FlowWaitingEvent extends BaseEvent {
  type: 'flow_waiting';
  node_id: string;
  wait_type: string;
  reason: string;
}

/** Flow resumed event */
export interface FlowResumedEvent extends BaseEvent {
  type: 'flow_resumed';
  node_id: string;
  wait_duration_ms: number;
}

/** Flow completed event */
export interface FlowCompletedEvent extends BaseEvent {
  type: 'flow_completed';
  output: unknown;
  total_duration_ms: number;
}

/** Flow failed event */
export interface FlowFailedEvent extends BaseEvent {
  type: 'flow_failed';
  error: string;
  failed_at_node?: string;
  total_duration_ms: number;
}

/** Partial text content from AI streaming */
export interface TextChunkEvent extends BaseEvent {
  type: 'text_chunk';
  text: string;
}

/** AI tool call started */
export interface ToolCallStartedEvent extends BaseEvent {
  type: 'tool_call_started';
  tool_call_id: string;
  function_name: string;
  arguments: unknown;
}

/** AI tool call completed */
export interface ToolCallCompletedEvent extends BaseEvent {
  type: 'tool_call_completed';
  tool_call_id: string;
  result: unknown;
}

/** Partial thinking/reasoning content from AI streaming */
export interface ThoughtChunkEvent extends BaseEvent {
  type: 'thought_chunk';
  text: string;
}

/** Log event */
export interface LogEvent extends BaseEvent {
  type: 'log';
  level: string;
  message: string;
  node_id?: string;
}

/** Union of all execution events */
export type FlowExecutionEvent =
  | StepStartedEvent
  | StepCompletedEvent
  | StepFailedEvent
  | FlowWaitingEvent
  | FlowResumedEvent
  | FlowCompletedEvent
  | FlowFailedEvent
  | TextChunkEvent
  | ToolCallStartedEvent
  | ToolCallCompletedEvent
  | ThoughtChunkEvent
  | LogEvent;
