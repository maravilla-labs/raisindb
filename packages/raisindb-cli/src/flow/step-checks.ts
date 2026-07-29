/**
 * Per-step property checks: baseline designer-UI validations
 * (useFlowValidation.ts), human-task rules, retry config, and function
 * reference resolution against a package folder.
 */

import { PackageContext } from './package-context.js';
import {
  KNOWN_RETRY_STRATEGIES,
  KNOWN_STEP_TYPES,
  KNOWN_TASK_TYPES,
  TASK_TYPE_SLUG_PATTERN,
  determineStepKind,
  normalizeReference,
  type DesignerNode,
  type Finding,
  type RawReference,
} from './types.js';

export function checkStep(node: DesignerNode, findings: Finding[], pkg: PackageContext | null): void {
  const nodeId = node.id ?? '<missing-id>';
  const props = node.properties ?? {};

  if (props.step_type != null && !KNOWN_STEP_TYPES.includes(props.step_type)) {
    findings.push({
      code: 'UNKNOWN_STEP_TYPE',
      severity: 'error',
      nodeId,
      field: 'step_type',
      message: `Unknown step_type "${props.step_type}" (expected one of: ${KNOWN_STEP_TYPES.join(', ')}).`,
    });
  }

  if (!props.action || props.action.trim() === '') {
    findings.push({
      code: 'MISSING_ACTION',
      severity: 'error',
      nodeId,
      field: 'action',
      message: 'Step is missing an action name (used as the display label and human-task title).',
    });
  }

  const kind = determineStepKind(props);

  if (kind === 'function' && !props.function_ref && !props.lua_script) {
    findings.push({
      code: 'MISSING_FUNCTION_REF',
      severity: props.step_type === 'default' ? 'suggestion' : 'warning',
      nodeId,
      field: 'function_ref',
      message:
        props.step_type === 'default'
          ? 'Function step should have a function reference or Lua script.'
          : 'Step has no step_type, function_ref, agent_ref, or condition — it lowers to a function step with nothing to call and will fail at runtime.',
    });
  }

  if (props.step_type === 'ai_agent' && !props.agent_ref) {
    findings.push({
      code: 'MISSING_AGENT_REF',
      severity: 'error',
      nodeId,
      field: 'agent_ref',
      message: 'AI Agent step requires an agent reference.',
    });
  }

  if (props.step_type === 'chat' && !props.chat_config?.agent_ref) {
    findings.push({
      code: 'MISSING_CHAT_AGENT_REF',
      severity: 'warning',
      nodeId,
      field: 'chat_config.agent_ref',
      message:
        'Chat step has no chat_config.agent_ref — the agent must then be resolvable from the conversation context at runtime.',
    });
  }

  checkHumanTask(node, findings);
  checkConditionSyntax(node, findings);
  checkRetry(node, findings);

  // Reference shape + resolution
  checkReference(node, 'function_ref', props.function_ref, findings, pkg, props.arguments);
  checkReference(node, 'compensation_ref', props.compensation_ref, findings, pkg, undefined);
}

function checkHumanTask(node: DesignerNode, findings: Finding[]): void {
  const nodeId = node.id ?? '<missing-id>';
  const props = node.properties ?? {};
  const isHumanTask = props.step_type === 'human_task' || props.task_type != null;
  if (!isHumanTask) return;

  if (!props.task_type) {
    findings.push({
      code: 'MISSING_TASK_TYPE',
      severity: 'error',
      nodeId,
      field: 'task_type',
      message: `Human Task step requires a task type (${KNOWN_TASK_TYPES.join(', ')}, or an application-defined slug).`,
    });
  } else if (!TASK_TYPE_SLUG_PATTERN.test(props.task_type)) {
    // The SET of task types is open - only the slug SHAPE is enforced, so a
    // package can define its own task vocabulary without a CLI release.
    findings.push({
      code: 'INVALID_TASK_TYPE',
      severity: 'error',
      nodeId,
      field: 'task_type',
      message: `Invalid task_type "${props.task_type}": expected 1-64 characters matching [a-z][a-z0-9_-]* (canonical types: ${KNOWN_TASK_TYPES.join(', ')}).`,
    });
  }

  if (!props.assignee || props.assignee.trim() === '') {
    findings.push({
      code: 'MISSING_ASSIGNEE',
      severity: 'error',
      nodeId,
      field: 'assignee',
      message: 'Human Task step requires an assignee.',
    });
  }

  if (props.task_type === 'approval' && (!Array.isArray(props.options) || props.options.length === 0)) {
    findings.push({
      code: 'MISSING_TASK_OPTIONS',
      severity: 'warning',
      nodeId,
      field: 'options',
      message: 'Approval task has no options defined. Default approve/reject options will be used.',
    });
  }

  if (props.task_type === 'input' && props.input_schema == null) {
    findings.push({
      code: 'MISSING_INPUT_SCHEMA',
      severity: 'warning',
      nodeId,
      field: 'input_schema',
      message: 'Input task has no input schema. The collected data will be unstructured.',
    });
  }

  if (typeof props.assignee === 'string' && props.assignee.startsWith('/agents/')) {
    const missing: string[] = [];
    if (props.min_confidence == null) missing.push('min_confidence (default 0.7)');
    if (!props.escalation_assignee) missing.push('escalation_assignee');
    if (missing.length > 0) {
      findings.push({
        code: 'AGENT_ASSIGNEE_GUARDRAILS',
        severity: 'suggestion',
        nodeId,
        field: 'assignee',
        message: `Assignee looks like an AI agent — consider setting ${missing.join(' and ')} so low-confidence decisions escalate to a human.`,
      });
    }
  }

  if (props.min_confidence != null) {
    const v = props.min_confidence;
    if (typeof v !== 'number' || Number.isNaN(v) || v < 0 || v > 1) {
      findings.push({
        code: 'INVALID_MIN_CONFIDENCE',
        severity: 'error',
        nodeId,
        field: 'min_confidence',
        message: `min_confidence must be a number between 0 and 1 (got ${JSON.stringify(v)}).`,
      });
    }
  }
}

function checkConditionSyntax(node: DesignerNode, findings: Finding[]): void {
  const nodeId = node.id ?? '<missing-id>';
  const condition = node.properties?.condition;
  if (condition == null) return;
  if (condition.trim() === '') {
    findings.push({
      code: 'EMPTY_CONDITION',
      severity: 'warning',
      nodeId,
      field: 'condition',
      message: 'Condition expression is empty.',
    });
    return;
  }
  if (!balancedCondition(condition)) {
    findings.push({
      code: 'INVALID_CONDITION',
      severity: 'error',
      nodeId,
      field: 'condition',
      message: `Condition has unbalanced parentheses or quotes: ${condition}`,
    });
  }
}

/** Cheap REL sanity check: balanced parens and closed string literals. */
export function balancedCondition(expr: string): boolean {
  let depth = 0;
  let quote: string | null = null;
  for (let i = 0; i < expr.length; i++) {
    const ch = expr[i];
    if (quote) {
      if (ch === '\\') i += 1;
      else if (ch === quote) quote = null;
      continue;
    }
    if (ch === '"' || ch === "'") quote = ch;
    else if (ch === '(') depth += 1;
    else if (ch === ')') {
      depth -= 1;
      if (depth < 0) return false;
    }
  }
  return depth === 0 && quote === null;
}

function checkRetry(node: DesignerNode, findings: Finding[]): void {
  const nodeId = node.id ?? '<missing-id>';
  const props = node.properties ?? {};

  if (props.retry_strategy != null && !KNOWN_RETRY_STRATEGIES.includes(props.retry_strategy)) {
    findings.push({
      code: 'UNKNOWN_RETRY_STRATEGY',
      severity: 'error',
      nodeId,
      field: 'retry_strategy',
      message: `Unknown retry_strategy "${props.retry_strategy}" (expected one of: ${KNOWN_RETRY_STRATEGIES.join(', ')}).`,
    });
  }

  const retry = props.retry;
  if (retry == null || typeof retry !== 'object') return;
  const problems: string[] = [];
  const { max_retries, base_delay_ms, max_delay_ms } = retry;
  if (typeof max_retries === 'number' && max_retries < 0) problems.push('max_retries is negative');
  if (typeof base_delay_ms === 'number' && base_delay_ms <= 0) problems.push('base_delay_ms must be positive');
  if (typeof max_delay_ms === 'number' && max_delay_ms <= 0) problems.push('max_delay_ms must be positive');
  if (
    typeof base_delay_ms === 'number' &&
    typeof max_delay_ms === 'number' &&
    max_delay_ms > 0 &&
    max_delay_ms < base_delay_ms
  ) {
    problems.push(`max_delay_ms (${max_delay_ms}) is smaller than base_delay_ms (${base_delay_ms})`);
  }
  if (problems.length > 0) {
    findings.push({
      code: 'INVALID_RETRY_CONFIG',
      severity: 'warning',
      nodeId,
      field: 'retry',
      message: `Suspicious retry config: ${problems.join('; ')}.`,
    });
  }
}

function checkReference(
  node: DesignerNode,
  field: string,
  raw: RawReference | undefined,
  findings: Finding[],
  pkg: PackageContext | null,
  args: unknown
): void {
  if (raw == null) return;
  const nodeId = node.id ?? '<missing-id>';
  const ref = normalizeReference(raw);
  if (!ref) {
    findings.push({
      code: 'INVALID_REFERENCE',
      severity: 'error',
      nodeId,
      field,
      message: `${field} has an invalid shape — expected a path string or {"raisin:ref": "...", "raisin:workspace": "..."}.`,
    });
    return;
  }
  if (!pkg) return; // single-file mode: cannot resolve

  const refPath = ref.path ?? ref.ref;
  // Templated refs (e.g. "$auto" or "{{ ... }}") cannot be resolved statically
  if (refPath.includes('{{') || refPath.includes('${') || refPath.startsWith('$')) return;

  const resolved = pkg.resolve(refPath, ref.workspace);
  if (!resolved) {
    findings.push({
      code: 'FUNCTION_NOT_FOUND',
      severity: 'warning',
      nodeId,
      field,
      message: `${field} "${refPath}" does not resolve to a function in this package (it may exist on the server).`,
    });
    return;
  }
  if (resolved.nodeType && resolved.nodeType !== 'raisin:Function') {
    findings.push({
      code: 'FUNCTION_NOT_A_FUNCTION',
      severity: 'error',
      nodeId,
      field,
      message: `${field} "${refPath}" resolves to a ${resolved.nodeType} node, not a raisin:Function (${resolved.file}).`,
    });
    return;
  }

  // Required arguments check (only for function_ref; arguments may be a
  // whole-string template, in which case we cannot check statically)
  if (field !== 'function_ref') return;
  const required = PackageContext.requiredArguments(resolved);
  if (required.length === 0) return;
  if (typeof args === 'string') return;
  const argKeys = args != null && typeof args === 'object' && !Array.isArray(args) ? Object.keys(args) : [];
  for (const name of required) {
    if (!argKeys.includes(name)) {
      findings.push({
        code: 'MISSING_REQUIRED_ARGUMENT',
        severity: 'error',
        nodeId,
        field: 'arguments',
        message: `Function "${refPath}" declares required input "${name}" but the step arguments do not provide it.`,
      });
    }
  }
}
