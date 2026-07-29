/**
 * Workflow definition types for RaisinDB flows.
 *
 * This is the canonical, shared data model for `raisin:Flow` definitions
 * (the `workflow_data` property of a flow node). The visual designer
 * (@raisindb/flow-designer), the admin console, and SDK consumers all
 * describe flows with these shapes.
 *
 * A flow definition is a tree of steps and containers:
 * - `FlowStep` - a single unit of work: function call, AI agent call,
 *   human task (approval/input/review/action), or chat session
 * - `FlowContainer` - groups children: and / or / parallel / ai_sequence
 *
 * Functions, AI agents, and humans are uniform step concepts: a human task
 * whose `assignee` resolves to an AI agent is decided by the agent (with
 * confidence-threshold escalation back to a human).
 */

// ============================================================================
// References
// ============================================================================

/**
 * RaisinDB reference (PropertyValue::Reference) - cross-node reference
 * with workspace context.
 */
export interface RaisinReference {
  /** Node ID or path (raisin:ref) */
  'raisin:ref': string;
  /** Workspace context */
  'raisin:workspace': string;
  /** Optional resolved path */
  'raisin:path'?: string;
}

/** Extract a display name from a reference path. */
export function getRefDisplayName(ref: RaisinReference): string {
  const path = ref['raisin:path'] || ref['raisin:ref'];
  const segments = path.split('/').filter(Boolean);
  return segments[segments.length - 1] || path;
}

/** Get the full path from a reference. */
export function getRefPath(ref: RaisinReference): string {
  return ref['raisin:path'] || ref['raisin:ref'];
}

// ============================================================================
// Core enums
// ============================================================================

/** Container types */
export type ContainerType = 'and' | 'or' | 'parallel' | 'ai_sequence';

/** Step error behavior */
export type StepErrorBehavior = 'stop' | 'skip' | 'continue';

/** Flow-level error strategy */
export type FlowErrorStrategy = 'fail_fast' | 'continue';

// ============================================================================
// Retry
// ============================================================================

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

// ============================================================================
// Human tasks
// ============================================================================

/** The human task types the flow runtime understands semantically */
export const CANONICAL_TASK_TYPES = ['approval', 'input', 'review', 'action'] as const;

/** A canonical human task type */
export type CanonicalTaskType = (typeof CANONICAL_TASK_TYPES)[number];

/**
 * Human task type.
 *
 * The canonical types are suggestions, not a closed set: an application may
 * define its own task vocabulary, and any slug matching
 * `[a-z][a-z0-9_-]{0,63}` is carried through to the task node verbatim for
 * a task UI to dispatch on. Spelled this way so the canonical values still
 * autocomplete.
 */
export type HumanTaskType = CanonicalTaskType | (string & {});

/** Human task option for approval tasks */
export interface TaskOption {
  /** Option value returned when selected */
  value: string;
  /** Display label */
  label: string;
  /** Visual style */
  style?: 'default' | 'success' | 'danger' | 'warning';
}

// ============================================================================
// Chat steps
// ============================================================================

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
export type ChatTerminationMode =
  | 'user_request'
  | 'max_turns'
  | 'inactivity'
  | 'ai_decision';

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

// ============================================================================
// AI containers
// ============================================================================

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
  /** Base delay between retries in milliseconds (default: 1000) */
  retry_delay_ms?: number;
  /** Timeout for entire container execution in milliseconds */
  timeout_ms?: number;
  /** Response format: "text", "json_object", or "json_schema" */
  response_format?: string;
  /** JSON schema for structured output (when response_format = "json_schema") */
  output_schema?: unknown;
}

// ============================================================================
// Steps and containers
// ============================================================================

/** Base flow node interface */
export interface FlowDefinitionNodeBase {
  id: string;
  node_type: 'raisin:FlowStep' | 'raisin:FlowContainer';
}

/** Step properties */
export interface FlowStepProperties {
  /** Action name/label */
  action?: string;
  /** Function reference */
  function_ref?: RaisinReference;
  /** Agent reference */
  agent_ref?: RaisinReference;
  /** Function arguments - values support template expressions like
   * `{{ input.value }}` or `${steps.prev.id}` */
  arguments?: Record<string, unknown>;
  /** Lua script for evaluation */
  lua_script?: string;
  /** Condition expression (REL) - same namespaces as templates:
   * `input.*`, `steps.<id>.*`, `trigger.*`, flow variables */
  condition?: string;
  /** Key for payload data */
  payload_key?: string;
  /** Whether step is disabled */
  disabled?: boolean;
  /** Step type - distinguishes AI agent, chat, and human task steps */
  step_type?: 'default' | 'ai_agent' | 'human_task' | 'chat' | 'wait' | 'sub_flow' | 'decision';
  /** Prompt for ai_agent steps (template expressions supported) */
  prompt?: string;
  /** Retry configuration for this step */
  retry?: RetryConfig;
  /** Retry strategy preset (alternative to custom retry config) */
  retry_strategy?: RetryStrategy;
  /** Step timeout in milliseconds */
  timeout_ms?: number;

  // Error handling

  /** Target node ID for error flow (error edge) */
  error_edge?: string;
  /** Target node ID when a wait deadline expires */
  timeout_edge?: string;
  /** Reference to compensation function for saga rollback */
  compensation_ref?: RaisinReference;
  /** Mapping for the compensation input - values support template
   * expressions; the forward step's output is available as `output.*` */
  compensation_input_mapping?: Record<string, unknown>;
  /** Continue workflow on step failure */
  continue_on_fail?: boolean;
  /** Execute step in isolated branch for safety */
  isolated_branch?: boolean;
  /** Execution identity mode for permission handling */
  execution_identity?: 'agent' | 'caller' | 'function';

  // Human task (step_type = 'human_task')

  /** Type of human task */
  task_type?: HumanTaskType;
  /** Assignee path - a user ("/users/alice") or an AI agent
   * ("/agents/support"); agents decide tasks automatically */
  assignee?: string;
  /** Task description */
  task_description?: string;
  /** Options for approval tasks */
  options?: TaskOption[];
  /** JSON schema for input tasks */
  input_schema?: object;
  /**
   * Task due time in seconds from creation (doubles as the wait deadline).
   * A template expression (e.g. "${input.due_seconds}") is allowed, so a
   * deadline can come from data instead of being authored into the flow.
   */
  due_in_seconds?: number | string;
  /** Task priority (1-5, where 5 is highest); a template expression is allowed */
  priority?: number | string;
  /** Minimum confidence (0-1) for an agent assignee's decision to be
   * accepted; below it the task escalates (default 0.7) */
  min_confidence?: number;
  /** Human assignee to escalate to when an agent assignee fails or is
   * not confident enough */
  escalation_assignee?: string;

  // Wait step (step_type = 'wait')

  /** What to wait for (defaults to 'delay') */
  wait_type?: 'delay' | 'until' | 'event' | 'cron';
  /** Duration for a 'delay' wait, e.g. "30m" (templates allowed) */
  duration?: string;
  /** Absolute timestamp for an 'until' wait (templates allowed) */
  until?: string;
  /** Event type for an 'event' wait */
  event_type?: string;
  /** Optional deadline for an 'event' wait */
  timeout?: string;
  /** Cron expression for a 'cron' wait */
  cron?: string;

  // Sub-flow step (step_type = 'sub_flow')

  /** The flow to invoke */
  flow_ref?: RaisinReference;
  /** Input mapping for the child flow (template expressions) */
  input_mapping?: object;
  /** Run the child flow asynchronously */
  async?: boolean;

  // Decision step (step_type = 'decision', or any step with a condition)

  /** Node id to run when the condition is true (defaults to the next sibling) */
  yes_branch?: string;
  /** Node id to run when the condition is false (defaults to the next sibling) */
  no_branch?: string;

  // Chat step (step_type = 'chat')

  /** Chat step configuration */
  chat_config?: ChatStepConfig;
}

/** Basic workflow step */
export interface FlowDefinitionStep extends FlowDefinitionNodeBase {
  node_type: 'raisin:FlowStep';
  properties: FlowStepProperties;
  on_error?: StepErrorBehavior;
  /** Target node ID for error flow (error edge at node level) */
  error_edge?: string;
}

/** Container rule for conditional execution */
export interface ContainerRule {
  /** Condition expression */
  condition: string;
  /** ID of next step if condition matches */
  next_step: string;
}

/**
 * Fan-out configuration for parallel containers.
 *
 * Without it, a parallel container's children ARE its branches - a fixed
 * set, all running at once. With it, the children form ONE branch subgraph
 * that runs once per item of a runtime collection, and the container joins
 * every one of those runs. That is what a per-item human task needs: one
 * task per row, then a real join.
 */
export interface FanOutConfig {
  /** Collection expression to fan out over, e.g. "${steps.plan.items}" */
  over: string;
  /** Safety cap on branches created (fan-out width comes from runtime data) */
  max_branches?: number;
}

/** How a parallel container joins its branches */
export type MergeStrategy = 'merge_all' | 'first_success' | 'all_success';

/** Container node with children (AND/OR/Parallel/AI) */
export interface FlowDefinitionContainer extends FlowDefinitionNodeBase {
  node_type: 'raisin:FlowContainer';
  container_type: ContainerType;
  rules?: ContainerRule[];
  children: FlowDefinitionNode[];
  /** AI container configuration (only for ai_sequence type) */
  ai_config?: AiContainerConfig;
  /** Fan-out configuration (only for parallel type): one branch per collection item */
  fan_out?: FanOutConfig;
  /** How branches are joined (only for parallel type; default "merge_all") */
  merge_strategy?: MergeStrategy;
  /** Container timeout in milliseconds */
  timeout_ms?: number;
}

/** Union type for all flow definition nodes */
export type FlowDefinitionNode = FlowDefinitionStep | FlowDefinitionContainer;

/** Complete flow definition (the `workflow_data` of a raisin:Flow node) */
export interface FlowDefinition {
  /** Schema version */
  version: number;
  /** Error handling strategy */
  error_strategy: FlowErrorStrategy;
  /** Global timeout in milliseconds */
  timeout_ms?: number;
  /** Root workflow nodes */
  nodes: FlowDefinitionNode[];
}

/** Type guard for FlowDefinitionStep */
export function isFlowDefinitionStep(
  node: FlowDefinitionNode,
): node is FlowDefinitionStep {
  return node.node_type === 'raisin:FlowStep';
}

/** Type guard for FlowDefinitionContainer */
export function isFlowDefinitionContainer(
  node: FlowDefinitionNode,
): node is FlowDefinitionContainer {
  return node.node_type === 'raisin:FlowContainer';
}

// ============================================================================
// Inbox tasks (human-in-the-loop primitive)
// ============================================================================

/** Status of an inbox task */
export type InboxTaskStatus = 'pending' | 'completed' | 'expired' | 'cancelled';

/**
 * An inbox task (`raisin:InboxTask` node) - created by human_task steps
 * (or `raisin.tasks.create` in functions) and completed by users or agents.
 */
export interface InboxTask {
  /** Node ID */
  id: string;
  /** Node path (under the assignee's inbox) */
  path: string;
  /** Task type */
  task_type: HumanTaskType;
  /** Task title */
  title: string;
  /** Task description */
  description?: string;
  /** Assignee path (user or agent) */
  assignee: string;
  /** Current status */
  status: InboxTaskStatus;
  /** Priority (1-5, where 5 is highest) */
  priority?: number;
  /** Options for approval tasks */
  options?: TaskOption[];
  /** JSON schema for input tasks */
  input_schema?: object;
  /** Owning flow instance (absent for standalone tasks) */
  flow_instance_id?: string;
  /** Step that created the task */
  step_id?: string;
  /** Creation timestamp (ISO 8601) */
  created_at?: string;
  /** Due timestamp (ISO 8601) */
  due_at?: string;
  /** The submitted response (when completed) */
  response?: Record<string, unknown>;
  /** Who completed the task (user id or agent path) */
  completed_by?: string;
  /** Completion timestamp (ISO 8601) */
  responded_at?: string;
  /** Set when an agent assignee escalated the task */
  escalated_from?: string;
  /** Why the task was escalated */
  escalation_reason?: string;
}

/** Result of completing an inbox task */
export interface TaskCompletionResult {
  task_id: string;
  task_path: string;
  status: string;
  /** Present when the completion resumed a waiting flow */
  flow?: {
    instance_id: string;
    job_id: string;
  };
}
