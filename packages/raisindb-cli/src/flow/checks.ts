/**
 * Structural flow checks: node ids, node/container types, edge integrity,
 * OR routing, container shape, and cycle detection on the lowered graph.
 */

import { isReachable, lowerFlow } from './lower.js';
import { PackageContext } from './package-context.js';
import { checkStep, balancedCondition } from './step-checks.js';
import { checkTemplates, expressionRoots } from './template-check.js';
import {
  CONTAINER_NODE_TYPE,
  KNOWN_CONTAINER_TYPES,
  KNOWN_MERGE_STRATEGIES,
  STEP_NODE_TYPE,
  isContainer,
  isStep,
  walkNodes,
  type DesignerFlowDefinition,
  type DesignerNode,
  type Finding,
} from './types.js';

/** Run every doctor check against a designer-format flow definition. */
export function checkFlow(def: DesignerFlowDefinition, pkg: PackageContext | null): Finding[] {
  const findings: Finding[] = [];
  const nodes = def.nodes ?? [];

  if (nodes.length === 0) {
    findings.push({
      code: 'EMPTY_FLOW',
      severity: 'warning',
      nodeId: '',
      message: 'Workflow has no steps. Add at least one step to create a valid workflow.',
    });
    return findings;
  }

  checkIds(def, findings);
  checkErrorStrategy(def, findings);

  // All node ids, for checks that validate cross-references (loop.until)
  const allIds = new Set<string>();
  walkNodes(nodes, (node) => {
    if (node.id) allIds.add(node.id);
  });

  walkNodes(nodes, (node, ancestors) => {
    if (!isStep(node) && !isContainer(node)) {
      findings.push({
        code: 'UNKNOWN_NODE_TYPE',
        severity: 'error',
        nodeId: node.id ?? '<missing-id>',
        field: 'node_type',
        message: `Unknown node_type "${node.node_type ?? '(none)'}" (expected ${STEP_NODE_TYPE} or ${CONTAINER_NODE_TYPE}).`,
      });
      return;
    }
    if (isStep(node)) checkStep(node, findings, pkg);
    if (isContainer(node)) checkContainer(node, ancestors, allIds, findings);
  });

  checkEdgeIntegrity(def, findings);
  checkTemplates(def, findings);
  checkCycles(def, findings);
  return findings;
}

function checkErrorStrategy(def: DesignerFlowDefinition, findings: Finding[]): void {
  if (def.error_strategy != null && !['fail_fast', 'continue'].includes(def.error_strategy)) {
    findings.push({
      code: 'UNKNOWN_ERROR_STRATEGY',
      severity: 'warning',
      nodeId: '',
      field: 'error_strategy',
      message: `Unknown error_strategy "${def.error_strategy}" (expected fail_fast or continue) — the engine will fail to parse this definition.`,
    });
  }
}

function checkIds(def: DesignerFlowDefinition, findings: Finding[]): void {
  const seen = new Map<string, number>();
  walkNodes(def.nodes, (node) => {
    const id = node.id;
    if (id == null || String(id).trim() === '') {
      findings.push({
        code: 'EMPTY_NODE_ID',
        severity: 'error',
        nodeId: '<missing-id>',
        field: 'id',
        message: 'Node has an empty or missing id.',
      });
      return;
    }
    seen.set(id, (seen.get(id) ?? 0) + 1);
    if (id === 'start' || id === 'end' || id.startsWith('__')) {
      findings.push({
        code: 'RESERVED_NODE_ID',
        severity: 'error',
        nodeId: id,
        field: 'id',
        message: `Node id "${id}" is reserved ("start"/"end" are injected by the engine; "__" prefixes are internal).`,
      });
    }
  });
  for (const [id, count] of seen) {
    if (count > 1) {
      findings.push({
        code: 'DUPLICATE_NODE_ID',
        severity: 'error',
        nodeId: id,
        field: 'id',
        message: `Node id "${id}" is used ${count} times — ids must be unique across the whole flow.`,
      });
    }
  }
}

function checkContainer(
  node: DesignerNode,
  ancestors: DesignerNode[],
  allIds: Set<string>,
  findings: Finding[]
): void {
  const nodeId = node.id ?? '<missing-id>';
  const containerType = node.container_type;
  const children = node.children ?? [];

  if (containerType == null || !KNOWN_CONTAINER_TYPES.includes(containerType)) {
    findings.push({
      code: 'UNKNOWN_CONTAINER_TYPE',
      severity: 'error',
      nodeId,
      field: 'container_type',
      message: `Unknown container_type "${containerType ?? '(none)'}" (expected one of: ${KNOWN_CONTAINER_TYPES.join(', ')}).`,
    });
    return;
  }

  // ai_sequence with auto tool mode legitimately has no children
  // (see builtin-packages/ai-tools .../flows/ai-agent-handler)
  if (children.length === 0 && containerType !== 'ai_sequence') {
    findings.push({
      code: 'EMPTY_CONTAINER',
      severity: 'warning',
      nodeId,
      message: `${containerType} container is empty. Add steps or remove the container.`,
    });
  }

  if (containerType === 'ai_sequence') {
    if (!node.ai_config) {
      findings.push({
        code: 'MISSING_AI_CONFIG',
        severity: 'error',
        nodeId,
        field: 'ai_config',
        message: 'AI Sequence container requires agent configuration (ai_config).',
      });
    } else if (!node.ai_config.agent_ref) {
      findings.push({
        code: 'MISSING_AI_AGENT_REF',
        severity: 'error',
        nodeId,
        field: 'ai_config.agent_ref',
        message: 'AI Sequence container requires an agent reference (ai_config.agent_ref; "$auto" derives it from the conversation).',
      });
    }
  }

  if (containerType === 'parallel' && ancestors.some((a) => a.container_type === 'parallel')) {
    findings.push({
      code: 'NESTED_PARALLEL',
      severity: 'warning',
      nodeId,
      message: 'Parallel container nested inside another parallel container — supported, but each branch spawns a child flow instance (expensive).',
    });
  }

  if (node.router != null && containerType !== 'or') {
    findings.push({
      code: 'ROUTER_ON_NON_OR',
      severity: 'error',
      nodeId,
      field: 'router',
      message: `AI router is only supported on OR containers (this is "${containerType}").`,
    });
  }

  if (node.loop != null && containerType !== 'loop') {
    findings.push({
      code: 'LOOP_ON_NON_LOOP',
      severity: 'error',
      nodeId,
      field: 'loop',
      message: `Loop config is only supported on loop containers (this is "${containerType}").`,
    });
  }

  if (node.fan_out != null && containerType !== 'parallel') {
    findings.push({
      code: 'FAN_OUT_ON_NON_PARALLEL',
      severity: 'error',
      nodeId,
      field: 'fan_out',
      message: `Fan-out config is only supported on parallel containers (this is "${containerType}").`,
    });
  }

  if (node.merge_strategy != null) {
    if (containerType !== 'parallel') {
      findings.push({
        code: 'MERGE_STRATEGY_ON_NON_PARALLEL',
        severity: 'error',
        nodeId,
        field: 'merge_strategy',
        message: `Merge strategy is only supported on parallel containers (this is "${containerType}").`,
      });
    } else if (!KNOWN_MERGE_STRATEGIES.includes(node.merge_strategy)) {
      findings.push({
        code: 'UNKNOWN_MERGE_STRATEGY',
        severity: 'error',
        nodeId,
        field: 'merge_strategy',
        message: `Unknown merge_strategy "${node.merge_strategy}" (expected one of: ${KNOWN_MERGE_STRATEGIES.join(', ')}).`,
      });
    }
  }

  if (containerType === 'or') checkOrRouting(node, findings);
  if (containerType === 'competition') checkCompetition(node, findings);
  if (containerType === 'loop') checkLoop(node, allIds, findings);
  if (node.fan_out != null) checkFanOut(node, findings);
}

/**
 * A fan-out turns the container's children into ONE branch subgraph run per
 * collection item, so it needs both a resolvable collection and a body.
 */
function checkFanOut(node: DesignerNode, findings: Finding[]): void {
  const nodeId = node.id ?? '<missing-id>';
  const fanOut = node.fan_out;
  if (fanOut == null) return;

  if (typeof fanOut.over !== 'string' || fanOut.over.trim() === '') {
    findings.push({
      code: 'FAN_OUT_MISSING_OVER',
      severity: 'error',
      nodeId,
      field: 'fan_out.over',
      message:
        'Fan-out requires fan_out.over — the collection expression to fan out over (e.g. ${steps.plan.items}).',
    });
  }

  if (
    fanOut.max_branches != null &&
    (typeof fanOut.max_branches !== 'number' || fanOut.max_branches < 1)
  ) {
    findings.push({
      code: 'FAN_OUT_INVALID_MAX_BRANCHES',
      severity: 'error',
      nodeId,
      field: 'fan_out.max_branches',
      message: `fan_out.max_branches must be a number >= 1 (got ${fanOut.max_branches}).`,
    });
  }

  if (!Array.isArray(node.children) || node.children.length === 0) {
    findings.push({
      code: 'FAN_OUT_EMPTY_BRANCH',
      severity: 'error',
      nodeId,
      message: 'Fan-out container has no children — there is no branch to run per item.',
    });
  }
}

function checkLoop(node: DesignerNode, allIds: Set<string>, findings: Finding[]): void {
  const nodeId = node.id ?? '<missing-id>';
  const loop = node.loop;

  if (loop == null || typeof loop.over !== 'string' || loop.over.trim() === '') {
    findings.push({
      code: 'LOOP_MISSING_OVER',
      severity: 'error',
      nodeId,
      field: 'loop.over',
      message:
        'Loop container requires loop.over — the collection expression to iterate (e.g. ${steps.pick.items}). The engine refuses a loop block without it.',
    });
  }
  if (loop == null) return;

  // item/index become flow variables referenced as REL identifiers —
  // anything outside [A-Za-z0-9_] silently breaks template resolution.
  for (const [field, value] of [
    ['loop.item', loop.item],
    ['loop.index', loop.index],
  ] as const) {
    if (value != null && (typeof value !== 'string' || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(value))) {
      findings.push({
        code: 'LOOP_INVALID_VARIABLE',
        severity: 'error',
        nodeId,
        field,
        message: `${field} "${value}" must be a snake_case identifier ([A-Za-z0-9_]) — it is referenced as a REL identifier in templates and conditions.`,
      });
    }
  }

  if (loop.max_iterations != null && (typeof loop.max_iterations !== 'number' || loop.max_iterations < 1)) {
    findings.push({
      code: 'LOOP_INVALID_MAX_ITERATIONS',
      severity: 'error',
      nodeId,
      field: 'loop.max_iterations',
      message: `loop.max_iterations must be a number >= 1 (got ${loop.max_iterations}).`,
    });
  }

  if (typeof loop.until === 'string' && loop.until.trim() !== '') {
    if (!balancedCondition(loop.until)) {
      findings.push({
        code: 'INVALID_CONDITION',
        severity: 'error',
        nodeId,
        field: 'loop.until',
        message: `loop.until condition has unbalanced parentheses or quotes: ${loop.until}`,
      });
    }
    const { stepRefs } = expressionRoots(loop.until);
    for (const stepId of stepRefs) {
      if (!allIds.has(stepId)) {
        findings.push({
          code: 'LOOP_UNTIL_UNKNOWN_STEP',
          severity: 'error',
          nodeId,
          field: 'loop.until',
          message: `loop.until references steps.${stepId} but no step with id "${stepId}" exists.`,
        });
      }
    }
  }
}

function checkCompetition(node: DesignerNode, findings: Finding[]): void {
  const nodeId = node.id ?? '<missing-id>';
  const children = node.children ?? [];

  if (!node.referee || !node.referee.agent_ref) {
    findings.push({
      code: 'COMPETITION_NEEDS_REFEREE',
      severity: 'error',
      nodeId,
      field: 'referee',
      message: 'Competition container requires a referee with an agent_ref to judge the answers.',
    });
  }

  const agentChildren = children.filter(
    (c) => isStep(c) && c.properties?.agent_ref != null
  );
  if (agentChildren.length < 2) {
    findings.push({
      code: 'COMPETITION_TOO_FEW_AGENTS',
      severity: 'error',
      nodeId,
      message: `Competition container needs at least 2 children with agent_ref (found ${agentChildren.length}) — competitors are agents, each may use a different LLM.`,
    });
  }
  for (const child of children) {
    if (isStep(child) && child.properties?.agent_ref == null) {
      findings.push({
        code: 'COMPETITION_NON_AGENT_CHILD',
        severity: 'warning',
        nodeId: child.id ?? '<missing-id>',
        message: 'Competition children must be agent steps (agent_ref) — this child is ignored by the competition.',
      });
    }
  }

  const minConfidence = node.referee?.min_confidence;
  if (minConfidence != null && (minConfidence < 0 || minConfidence > 1)) {
    findings.push({
      code: 'INVALID_MIN_CONFIDENCE',
      severity: 'error',
      nodeId,
      field: 'referee.min_confidence',
      message: `referee.min_confidence must be between 0 and 1 (got ${minConfidence}).`,
    });
  }
}

function checkOrRouting(node: DesignerNode, findings: Finding[]): void {
  const nodeId = node.id ?? '<missing-id>';
  const children = node.children ?? [];
  const rules = (node.rules ?? []).filter((r) => r != null);
  const childIds = new Set(children.map((c) => c.id).filter((id): id is string => id != null));

  for (const [idx, rule] of rules.entries()) {
    if (rule.next_step == null || !childIds.has(rule.next_step)) {
      findings.push({
        code: 'INVALID_RULE_TARGET',
        severity: 'error',
        nodeId,
        field: `rules[${idx}].next_step`,
        message: `Rule ${idx + 1} routes to "${rule.next_step ?? '(none)'}" which is not a child of this container.`,
      });
    }
    if (rule.condition == null || rule.condition.trim() === '') {
      findings.push({
        code: 'EMPTY_CONDITION',
        severity: 'warning',
        nodeId,
        field: `rules[${idx}].condition`,
        message: `Rule ${idx + 1} has an empty condition.`,
      });
    } else if (!balancedCondition(rule.condition)) {
      findings.push({
        code: 'INVALID_CONDITION',
        severity: 'error',
        nodeId,
        field: `rules[${idx}].condition`,
        message: `Rule ${idx + 1} condition has unbalanced parentheses or quotes: ${rule.condition}`,
      });
    }
  }

  if (children.length === 0) return;

  const childrenWithCondition = children.filter(
    (c) => isStep(c) && typeof c.properties?.condition === 'string' && c.properties.condition.trim() !== ''
  );

  // Router validation (the agent decides when no REL rule matched)
  const router = node.router;
  if (router != null) {
    if (!router.agent_ref) {
      findings.push({
        code: 'MISSING_ROUTER_AGENT',
        severity: 'error',
        nodeId,
        field: 'router.agent_ref',
        message: 'AI router requires an agent_ref (the agent that decides the branch).',
      });
    }
    if (router.default_branch != null && !childIds.has(router.default_branch)) {
      findings.push({
        code: 'INVALID_DEFAULT_BRANCH',
        severity: 'error',
        nodeId,
        field: 'router.default_branch',
        message: `router.default_branch "${router.default_branch}" is not a child of this container.`,
      });
    }
    if (
      router.min_confidence != null &&
      (router.min_confidence < 0 || router.min_confidence > 1)
    ) {
      findings.push({
        code: 'INVALID_MIN_CONFIDENCE',
        severity: 'error',
        nodeId,
        field: 'router.min_confidence',
        message: `router.min_confidence must be between 0 and 1 (got ${router.min_confidence}).`,
      });
    }
  }

  if (rules.length === 0 && childrenWithCondition.length === 0 && router == null) {
    findings.push({
      code: 'OR_UNROUTABLE',
      severity: 'error',
      nodeId,
      message:
        'OR container has no rules, no child conditions, and no AI router — there is no routing information (the engine falls through to the first child only).',
    });
    return;
  }

  // With an AI router every child is reachable (the agent can pick any),
  // so unrouted-child analysis only applies without one.
  if (router != null) return;

  // Children not reachable by any routing information.
  // Note: when explicit rules exist the engine ignores child conditions
  // entirely (conversion.rs), so only rules count in that case.
  const routed = new Set(rules.map((r) => r.next_step));
  for (const child of children) {
    const childId = child.id ?? '<missing-id>';
    if (routed.has(childId)) continue;
    const hasOwnCondition =
      rules.length === 0 && isStep(child) && !!child.properties?.condition?.trim();
    if (!hasOwnCondition) {
      findings.push({
        code: 'UNROUTED_OR_CHILD',
        severity: 'warning',
        nodeId: childId,
        message:
          rules.length > 0
            ? 'Step is inside an OR container but no rule routes to it (child conditions are ignored when explicit rules exist) — it will never run.'
            : 'Step is inside an OR container but has no condition while siblings do — it will never run.',
      });
    }
  }
}

function checkEdgeIntegrity(def: DesignerFlowDefinition, findings: Finding[]): void {
  const allIds = new Set<string>();
  walkNodes(def.nodes, (node) => {
    if (node.id) allIds.add(node.id);
  });

  walkNodes(def.nodes, (node) => {
    if (!isStep(node)) return;
    const nodeId = node.id ?? '<missing-id>';
    const errorEdge = node.error_edge ?? node.properties?.error_edge;
    if (errorEdge != null && !allIds.has(errorEdge)) {
      findings.push({
        code: 'INVALID_ERROR_EDGE',
        severity: 'error',
        nodeId,
        field: 'error_edge',
        message: `Error edge points to non-existent node: ${errorEdge}`,
      });
    }
    const timeoutEdge = node.properties?.timeout_edge;
    if (timeoutEdge != null && !allIds.has(timeoutEdge)) {
      findings.push({
        code: 'INVALID_TIMEOUT_EDGE',
        severity: 'error',
        nodeId,
        field: 'timeout_edge',
        message: `Timeout edge points to non-existent node: ${timeoutEdge}`,
      });
    }
  });
}

/**
 * Detect error_edge / timeout_edge targets that can flow back to the step
 * that declared the edge in the lowered graph (possible infinite loop when
 * combined with retries).
 */
function checkCycles(def: DesignerFlowDefinition, findings: Finding[]): void {
  let plan;
  try {
    plan = lowerFlow(def);
  } catch {
    return; // structural errors elsewhere; skip cycle analysis
  }
  for (const node of plan.nodes.values()) {
    for (const [field, target] of [
      ['error_edge', node.errorEdge],
      ['timeout_edge', node.timeoutEdge],
    ] as const) {
      if (!target || !plan.nodes.has(target)) continue;
      if (isReachable(plan, target, node.id)) {
        findings.push({
          code: 'POSSIBLE_CYCLE',
          severity: 'warning',
          nodeId: node.id,
          field,
          message: `${field} "${target}" can flow back to "${node.id}" — combined with retries this can loop indefinitely.`,
        });
      }
    }
  }
}
