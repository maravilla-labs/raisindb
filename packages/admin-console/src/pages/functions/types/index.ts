/**
 * Types for the Functions IDE
 */

import type { Node as NodeType } from '../../../api/nodes'

// Function language types
export type FunctionLanguage = 'javascript' | 'starlark' | 'sql'

// Execution mode types
export type ExecutionMode = 'async' | 'sync' | 'both'

// Trigger types
export type TriggerType = 'node_event' | 'schedule' | 'http'

// Event kinds for node triggers
export type EventKind =
  | 'Created'
  | 'Updated'
  | 'Deleted'
  | 'Published'
  | 'Unpublished'
  | 'Moved'
  | 'Renamed'

// Flow error strategies
export type FlowErrorStrategy = 'fail_fast' | 'continue'

// Step error behavior
export type StepErrorBehavior = 'stop' | 'skip' | 'continue'

// Flow step status
export type FlowStepStatus = 'pending' | 'running' | 'completed' | 'failed' | 'skipped'

// Flow status
export type FlowStatus = 'pending' | 'running' | 'completed' | 'partial' | 'failed'

/**
 * Reference to a function in a flow
 */
export interface FunctionRef {
  /** Path to the function node (e.g., /lib/raisin/function/my_func) */
  path: string
  /** Optional timeout for this specific function */
  timeout_ms?: number
}

/**
 * Step in a flow execution
 */
export interface FlowStep {
  /** Unique ID for the step */
  id: string
  /** Display name for the step */
  name: string
  /** Functions to execute in this step */
  functions: FunctionRef[]
  /** If true, execute all functions in parallel; if false, execute sequentially */
  parallel: boolean
  /** IDs of steps that must complete before this step can run */
  depends_on: string[]
  /** Optional timeout for the entire step */
  timeout_ms?: number
  /** Behavior when an error occurs in this step */
  on_error: StepErrorBehavior
}

/**
 * Function flow definition for multi-function execution
 */
export interface FunctionFlow {
  /** Schema version */
  version: number
  /** How to handle errors during flow execution */
  error_strategy: FlowErrorStrategy
  /** Optional global timeout for entire flow */
  timeout_ms?: number
  /** Steps to execute in order (respecting depends_on) */
  steps: FlowStep[]
}

/**
 * Result of a single function execution within a flow
 */
export interface FunctionResult {
  function_path: string
  success: boolean
  result?: unknown
  error?: string
  duration_ms: number
  logs: LogEntry[]
}

/**
 * Result of a step execution within a flow
 */
export interface StepResult {
  step_id: string
  status: FlowStepStatus
  function_results: FunctionResult[]
  started_at: string
  completed_at?: string
  duration_ms?: number
}

/**
 * Result of a complete flow execution
 */
export interface FlowExecutionResult {
  flow_execution_id: string
  trigger_path: string
  status: FlowStatus
  step_results: Record<string, StepResult>
  started_at: string
  completed_at?: string
  duration_ms?: number
  final_result?: unknown
}

/**
 * Trigger condition configuration
 */
export interface TriggerCondition {
  name: string
  trigger_type: TriggerType
  event_kinds?: EventKind[]
  cron_expression?: string
  filters?: TriggerFilters
  enabled: boolean
  priority: number
}

/**
 * Resource limits for function execution
 */
export interface ResourceLimits {
  timeout_ms: number
  max_memory_bytes: number
  max_stack_bytes: number
}

/**
 * Network policy for function HTTP access
 */
export interface NetworkPolicy {
  http_enabled: boolean
  allowed_urls: string[]
  max_concurrent_requests: number
  request_timeout_ms: number
}

/**
 * Function node properties (from raisin:Function)
 */
export interface FunctionProperties {
  name: string
  title?: string
  description?: string
  language: FunctionLanguage
  execution_mode: ExecutionMode
  /** Entry file in format 'filename:function' (e.g., 'index.js:handler') */
  entry_file: string
  /** @deprecated Use entry_file instead */
  entrypoint?: string
  enabled: boolean
  version: number
  resource_limits?: ResourceLimits
  network_policy?: NetworkPolicy
  triggers?: TriggerCondition[]
  input_schema?: Record<string, unknown>
  output_schema?: Record<string, unknown>
}

/**
 * Function node (raisin:Function)
 * Using intersection type to allow Node properties plus function-specific properties
 */
export type FunctionNode = Omit<NodeType, 'properties'> & {
  node_type: 'raisin:Function'
  properties: FunctionProperties
}

/**
 * Trigger filters for matching events
 */
export interface TriggerFilters {
  /** Workspaces to match (or '*' for all) */
  workspaces?: string[]
  /** Path patterns to match (glob syntax) */
  paths?: string[]
  /** Node types to match */
  node_types?: string[]
  /** Property value filters (key-value pairs) */
  property_filters?: Record<string, any>
}

/**
 * Trigger configuration (varies by trigger_type)
 */
export interface TriggerConfig {
  /** Event kinds for node_event triggers */
  event_kinds?: EventKind[]
  /** Cron expression for schedule triggers */
  cron_expression?: string
  /** HTTP methods allowed for http triggers */
  methods?: string[]
  /** Path suffix for http triggers */
  path_suffix?: string
}

/**
 * Raisin reference format for linking to other nodes
 */
export interface RaisinReference {
  'raisin:ref': string
  'raisin:workspace': string
  'raisin:path': string
}

/**
 * Execution target mode for triggers
 */
export type TriggerExecutionMode = 'functions' | 'flow'

/**
 * Standalone trigger node properties (from raisin:Trigger)
 */
export interface TriggerProperties {
  name: string
  title?: string
  description?: string
  trigger_type: TriggerType
  config: TriggerConfig
  filters?: TriggerFilters
  enabled: boolean
  priority?: number
  /** Maximum retry attempts on failure (0 = no retries, default = 3) */
  max_retries?: number
  /** Auto-generated nanoid for HTTP trigger webhook URL */
  webhook_id?: string
  /** Execution target mode: 'functions' for inline functions, 'flow' for external flow reference */
  execution_mode?: TriggerExecutionMode
  /** Reference to an external raisin:Flow node */
  flow_ref?: RaisinReference
  /** @deprecated Use function_flow for new triggers */
  function_path?: string
  /** Multi-function flow definition */
  function_flow?: FunctionFlow
}

/**
 * Standalone trigger node (raisin:Trigger)
 */
export type TriggerNode = Omit<NodeType, 'properties'> & {
  node_type: 'raisin:Trigger'
  properties: TriggerProperties
}

/**
 * Flow workflow data definition (visual workflow nodes)
 */
export interface FlowWorkflowData {
  version: number
  error_strategy: FlowErrorStrategy
  timeout_ms?: number
  nodes: FlowWorkflowNode[]
}

/**
 * Base workflow node in flow
 */
export interface FlowWorkflowNodeBase {
  id: string
  node_type: 'raisin:FlowStep' | 'raisin:FlowContainer'
}

/**
 * Workflow step node
 */
export interface FlowWorkflowStep extends FlowWorkflowNodeBase {
  node_type: 'raisin:FlowStep'
  properties: {
    action?: string
    function_ref?: string
    lua_script?: string
    payload_key?: string
    disabled?: boolean
  }
  on_error?: StepErrorBehavior
}

/**
 * Container types for workflow containers
 */
export type FlowContainerType = 'and' | 'or' | 'parallel' | 'ai_sequence'

/**
 * Workflow container node
 */
export interface FlowWorkflowContainer extends FlowWorkflowNodeBase {
  node_type: 'raisin:FlowContainer'
  container_type: FlowContainerType
  rules?: { condition: string; next_step: string }[]
  children: FlowWorkflowNode[]
}

/**
 * Union type for workflow nodes
 */
export type FlowWorkflowNode = FlowWorkflowStep | FlowWorkflowContainer

/**
 * Flow node properties (from raisin:Flow)
 */
export interface FlowProperties {
  name: string
  title?: string
  description?: string
  enabled: boolean
  workflow_data?: FlowWorkflowData
  timeout_ms?: number
}

/**
 * Flow node (raisin:Flow)
 */
export type FlowNode = Omit<NodeType, 'properties'> & {
  node_type: 'raisin:Flow'
  properties: FlowProperties
}

/**
 * Code asset node (raisin:Asset) that contains function source code
 */
export interface CodeAsset extends NodeType {
  node_type: 'raisin:Asset'
  properties: {
    mime_type: string
    size?: number
  }
}

/**
 * Open editor tab
 */
export interface EditorTab {
  id: string
  path: string
  name: string
  node_type: string
  language: FunctionLanguage
  isDirty: boolean
}

/**
 * Log entry from function execution
 */
export interface LogEntry {
  level: 'debug' | 'info' | 'warn' | 'error'
  message: string
  timestamp: string
}

/**
 * Function execution record
 */
export interface ExecutionRecord {
  id: string
  execution_id: string
  function_path: string
  trigger_name?: string
  status: 'pending' | 'running' | 'completed' | 'failed'
  started_at: string
  completed_at?: string
  duration_ms?: number
  result?: unknown
  error?: string
  logs: LogEntry[]
}

/**
 * Functions IDE preferences (stored in localStorage)
 */
export interface FunctionsPreferences {
  sidebarWidth: number
  propertiesWidth: number
  outputHeight: number
  sidebarVisible: boolean
  propertiesVisible: boolean
  outputVisible: boolean
  fontSize: number
  lastOpenTabs: string[]
  expandedFolders: string[]
}

/**
 * Validation problem severity
 */
export type ProblemSeverity = 'error' | 'warning' | 'suggestion'

/**
 * Validation problem from editor (code or flow)
 */
export interface ValidationProblem {
  /** Unique ID for the problem */
  id: string
  /** Source file or node path */
  source: string
  /** Node ID for flow problems */
  nodeId?: string
  /** Property field for specific issues */
  field?: string
  /** Problem code for categorization */
  code: string
  /** Human-readable message */
  message: string
  /** Severity level */
  severity: ProblemSeverity
  /** Line number (for code problems) */
  line?: number
  /** Column number (for code problems) */
  column?: number
}

/**
 * Functions IDE context state
 */
export interface FunctionsContextState {
  // Tree state
  nodes: NodeType[]
  expandedNodes: Set<string>
  selectedNode: FunctionNode | null
  loading: boolean

  // Editor state
  openTabs: EditorTab[]
  activeTabId: string | null
  codeCache: Map<string, string> // path -> code

  // Output state
  logs: LogEntry[]
  executions: ExecutionRecord[]
  problems: ValidationProblem[]

  // Preferences
  preferences: FunctionsPreferences
}
