/**
 * Types for the flow doctor / explain commands.
 *
 * Mirrors the designer format schema defined in
 * crates/raisin-flow-runtime/src/types/designer_format/types.rs.
 */

export type Severity = 'error' | 'warning' | 'suggestion';

export interface Finding {
  code: string;
  severity: Severity;
  message: string;
  /** Node id the finding is attached to ('' for flow-level findings). */
  nodeId: string;
  /** Property/field the finding refers to, when applicable. */
  field?: string;
}

/** Reference: plain path string or full {"raisin:ref": ..} object. */
export type RawReference =
  | string
  | {
      'raisin:ref'?: unknown;
      'raisin:workspace'?: unknown;
      'raisin:path'?: unknown;
    };

export interface NormalizedReference {
  ref: string;
  workspace: string;
  path?: string;
}

export interface DesignerStepProperties {
  action?: string;
  function_ref?: RawReference;
  agent_ref?: RawReference;
  lua_script?: string;
  condition?: string;
  payload_key?: string;
  disabled?: boolean;
  step_type?: string;
  retry?: { max_retries?: number; base_delay_ms?: number; max_delay_ms?: number };
  retry_strategy?: string;
  timeout_ms?: number;
  error_edge?: string;
  compensation_ref?: RawReference;
  compensation_input_mapping?: unknown;
  continue_on_fail?: boolean;
  isolated_branch?: boolean;
  execution_identity?: string;
  chat_config?: { agent_ref?: RawReference | null; [key: string]: unknown };
  arguments?: unknown;
  prompt?: string;
  // Human task properties
  task_type?: string;
  assignee?: string;
  task_description?: string;
  options?: unknown;
  input_schema?: unknown;
  due_in_seconds?: number;
  priority?: number;
  min_confidence?: number;
  escalation_assignee?: string;
  timeout_edge?: string;
  [key: string]: unknown;
}

export interface DesignerContainerRule {
  condition?: string;
  next_step?: string;
}

export interface DesignerNode {
  node_type?: string; // raisin:FlowStep | raisin:FlowContainer
  id?: string;
  // Step fields
  properties?: DesignerStepProperties;
  on_error?: string;
  error_edge?: string;
  // Container fields
  container_type?: string; // and | or | parallel | ai_sequence | competition | loop
  children?: DesignerNode[];
  ai_config?: {
    agent_ref?: RawReference | null;
    tool_mode?: string;
    explicit_tools?: string[];
    max_iterations?: number;
    [key: string]: unknown;
  };
  rules?: DesignerContainerRule[];
  /** AI router for OR containers (agent decides the branch) */
  router?: {
    agent_ref?: RawReference | null;
    prompt?: string;
    min_confidence?: number;
    default_branch?: string;
    [key: string]: unknown;
  };
  /** Referee for competition containers */
  referee?: {
    agent_ref?: RawReference | null;
    prompt?: string;
    min_confidence?: number;
    max_rounds?: number;
    [key: string]: unknown;
  };
  /** Shared task prompt for competition containers */
  prompt?: string;
  /** Loop configuration for loop containers */
  loop?: {
    over?: string;
    item?: string;
    index?: string;
    max_iterations?: number;
    until?: string;
    [key: string]: unknown;
  };
  /**
   * Fan-out configuration for parallel containers: run the container's
   * child subgraph once per item of a runtime collection, then join every
   * run. Without it the children ARE the branches.
   */
  fan_out?: {
    over?: string;
    max_branches?: number;
    [key: string]: unknown;
  };
  /** Branch join strategy for parallel containers */
  merge_strategy?: string;
  timeout_ms?: number;
  [key: string]: unknown;
}

export interface DesignerFlowDefinition {
  version?: number;
  error_strategy?: string;
  timeout_ms?: number;
  nodes?: DesignerNode[];
}

export type FlowFormat = 'designer' | 'runtime';

/** One flow definition discovered on disk. */
export interface FlowSource {
  /** Absolute file path the flow was read from. */
  file: string;
  /** Flow name (properties.name / title, or filename). */
  name: string;
  format: FlowFormat;
  /** Designer definition (only set for format === 'designer'). */
  definition?: DesignerFlowDefinition;
  /** Informational note (e.g. extracted from JS, runtime format skipped). */
  note?: string;
}

export interface ParseFailure {
  file: string;
  error: string;
}

export interface DiscoveryResult {
  sources: FlowSource[];
  failures: ParseFailure[];
  /** Set when the target was a directory (enables function resolution). */
  packageDir?: string;
}

export interface FlowReport {
  source: FlowSource;
  findings: Finding[];
}

export const STEP_NODE_TYPE = 'raisin:FlowStep';
export const CONTAINER_NODE_TYPE = 'raisin:FlowContainer';

export const KNOWN_STEP_TYPES = [
  'default',
  'ai_agent',
  'chat',
  'human_task',
  'wait',
  'sub_flow',
  'decision',
];
export const KNOWN_CONTAINER_TYPES = ['and', 'or', 'parallel', 'ai_sequence', 'competition', 'loop'];
/**
 * The human task types the flow runtime understands semantically.
 *
 * These are CANONICAL, not exhaustive: an application may define its own
 * task vocabulary, and any slug matching {@link TASK_TYPE_SLUG_PATTERN} is
 * carried through to the task node verbatim. Validation therefore checks
 * the slug's shape, not its membership here.
 */
export const KNOWN_TASK_TYPES = ['approval', 'input', 'review', 'action'];

/** Shape a task_type slug must have (mirrors the engine's own rule). */
export const TASK_TYPE_SLUG_PATTERN = /^[a-z][a-z0-9_-]{0,63}$/;
export const KNOWN_RETRY_STRATEGIES = ['none', 'quick', 'standard', 'aggressive', 'llm'];
export const KNOWN_MERGE_STRATEGIES = ['merge_all', 'first_success', 'all_success'];

/** Template/condition root namespaces resolvable by the flow runtime. */
export const KNOWN_TEMPLATE_ROOTS = [
  'input',
  'steps',
  'trigger',
  'output',
  'flow',
  'item',
  'index',
  'branch',
  // Populated on error paths (docs/workflows.md section 5.2)
  'error',
  // Well-known flow variable set by human task completion (section 6.2)
  '__human_response',
];

export function isStep(node: DesignerNode): boolean {
  return node.node_type === STEP_NODE_TYPE;
}

export function isContainer(node: DesignerNode): boolean {
  return node.node_type === CONTAINER_NODE_TYPE;
}

/**
 * Normalize a reference (plain string or full object).
 * Returns null when the shape is invalid.
 */
export function normalizeReference(raw: RawReference | null | undefined): NormalizedReference | null {
  if (raw == null) return null;
  if (typeof raw === 'string') {
    if (raw.trim() === '') return null;
    return { ref: raw, workspace: 'functions' };
  }
  if (typeof raw === 'object' && !Array.isArray(raw)) {
    const ref = (raw as Record<string, unknown>)['raisin:ref'];
    if (typeof ref !== 'string' || ref.trim() === '') return null;
    const workspace = (raw as Record<string, unknown>)['raisin:workspace'];
    const path = (raw as Record<string, unknown>)['raisin:path'];
    return {
      ref,
      workspace: typeof workspace === 'string' && workspace !== '' ? workspace : 'functions',
      path: typeof path === 'string' ? path : undefined,
    };
  }
  return null;
}

/**
 * Determine the effective (runtime) step kind, mirroring
 * conversion.rs::determine_step_type priority order.
 */
export function determineStepKind(props: DesignerStepProperties): string {
  if (props.step_type === 'human_task' || props.task_type != null) return 'human_task';
  if (props.step_type === 'wait') return 'wait';
  if (props.step_type === 'sub_flow') return 'sub_flow';
  if (props.step_type === 'decision') return 'decision';
  if (props.function_ref != null) return 'function';
  if (props.step_type === 'ai_agent') return 'ai_agent';
  if (props.agent_ref != null) return 'ai_container';
  if (props.step_type === 'chat') return 'chat';
  if (props.condition != null) return 'decision';
  return 'function';
}

/** Walk all nodes depth-first (pre-order), with container ancestry. */
export function walkNodes(
  nodes: DesignerNode[] | undefined,
  visit: (node: DesignerNode, ancestors: DesignerNode[]) => void,
  ancestors: DesignerNode[] = []
): void {
  for (const node of nodes ?? []) {
    visit(node, ancestors);
    if (Array.isArray(node.children)) {
      walkNodes(node.children, visit, [...ancestors, node]);
    }
  }
}
