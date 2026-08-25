/**
 * useFlowValidation Hook
 *
 * Provides flow validation with debouncing for real-time feedback.
 * Validates flow structure, step configurations, and error handling paths.
 */

import { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import type { FlowDefinition, FlowNode, FlowStep, FlowContainer } from '../types';
import type { ValidationResult, ValidationIssue } from '../context/FlowDesignerContext';
import { isFlowStep, isFlowContainer, getErrorEdge } from '../utils';
import { isValidTaskTypeSlug } from '../types';

export interface UseFlowValidationOptions {
  /** Debounce delay in milliseconds (default: 300ms) */
  debounceMs?: number;
  /** Custom validators to run in addition to built-in ones */
  customValidators?: FlowValidator[];
  /** Whether validation is enabled (default: true) */
  enabled?: boolean;
}

export interface UseFlowValidationReturn {
  /** Current validation result */
  validation: ValidationResult;
  /** Whether validation is currently running */
  isValidating: boolean;
  /** Manually trigger validation */
  validate: () => ValidationResult;
  /** Get issues for a specific node */
  getNodeIssues: (nodeId: string) => ValidationIssue[];
  /** Check if a specific node has errors */
  hasNodeErrors: (nodeId: string) => boolean;
}

/** Custom validator function type */
export type FlowValidator = (flow: FlowDefinition) => ValidationIssue[];

/**
 * Built-in validation rules
 */
function validateEmptyFlow(flow: FlowDefinition): ValidationIssue[] {
  if (flow.nodes.length === 0) {
    return [{
      nodeId: '',
      code: 'EMPTY_FLOW',
      message: 'Workflow has no steps. Add at least one step to create a valid workflow.',
      severity: 'warning',
    }];
  }
  return [];
}

function validateStepProperties(nodes: FlowNode[], _parentId?: string): ValidationIssue[] {
  const issues: ValidationIssue[] = [];

  for (const node of nodes) {
    if (isFlowStep(node)) {
      const step = node as FlowStep;

      // Check for missing action name
      if (!step.properties.action || step.properties.action.trim() === '') {
        issues.push({
          nodeId: step.id,
          field: 'action',
          code: 'MISSING_ACTION',
          message: 'Step is missing an action name.',
          severity: 'error',
        });
      }

      // Check for function step without function reference
      if (step.properties.step_type === 'default' && !step.properties.function_ref && !step.properties.lua_script) {
        issues.push({
          nodeId: step.id,
          field: 'function_ref',
          code: 'MISSING_FUNCTION_REF',
          message: 'Function step should have a function reference or Lua script.',
          severity: 'suggestion',
        });
      }

      // Check for AI agent step without agent reference
      if (step.properties.step_type === 'ai_agent' && !step.properties.agent_ref) {
        issues.push({
          nodeId: step.id,
          field: 'agent_ref',
          code: 'MISSING_AGENT_REF',
          message: 'AI Agent step requires an agent reference.',
          severity: 'error',
        });
      }

      // Check for human task step without task type. The SET of types is
      // open (applications may define their own), so only presence and the
      // slug shape are checked - not membership in the canonical four.
      if (step.properties.step_type === 'human_task' && !step.properties.task_type) {
        issues.push({
          nodeId: step.id,
          field: 'task_type',
          code: 'MISSING_TASK_TYPE',
          message:
            'Human Task step requires a task type (approval, input, review, action, or an application-defined slug).',
          severity: 'error',
        });
      }

      if (
        step.properties.step_type === 'human_task' &&
        step.properties.task_type &&
        !isValidTaskTypeSlug(step.properties.task_type)
      ) {
        issues.push({
          nodeId: step.id,
          field: 'task_type',
          code: 'INVALID_TASK_TYPE',
          message: `task_type "${step.properties.task_type}" must be 1-64 characters matching [a-z][a-z0-9_-]*.`,
          severity: 'error',
        });
      }

      if (step.properties.step_type === 'human_task') {
        // Human task must have an assignee
        if (!step.properties.assignee || step.properties.assignee.trim() === '') {
          issues.push({
            nodeId: step.id,
            field: 'assignee',
            code: 'MISSING_ASSIGNEE',
            message: 'Human Task step requires an assignee.',
            severity: 'error',
          });
        }

        // Approval tasks should define options
        if (
          step.properties.task_type === 'approval' &&
          (!step.properties.options || step.properties.options.length === 0)
        ) {
          issues.push({
            nodeId: step.id,
            field: 'options',
            code: 'MISSING_TASK_OPTIONS',
            message: 'Approval task has no options defined. Default approve/reject options will be used.',
            severity: 'warning',
          });
        }

        // Input tasks should define an input schema
        if (step.properties.task_type === 'input' && !step.properties.input_schema) {
          issues.push({
            nodeId: step.id,
            field: 'input_schema',
            code: 'MISSING_INPUT_SCHEMA',
            message: 'Input task has no input schema. The collected data will be unstructured.',
            severity: 'warning',
          });
        }
      }

      // Check for chat step without agent reference
      if (step.properties.step_type === 'chat' && !step.properties.chat_config?.agent_ref) {
        issues.push({
          nodeId: step.id,
          field: 'chat_config.agent_ref',
          code: 'MISSING_CHAT_AGENT_REF',
          message: 'Chat step requires an agent reference in chat configuration.',
          severity: 'error',
        });
      }

      // Note: error edges pointing to non-existent nodes are validated
      // against the full flow in validateErrorEdges
    }

    if (isFlowContainer(node)) {
      const container = node as FlowContainer;

      // Check for empty containers
      if (container.children.length === 0) {
        issues.push({
          nodeId: container.id,
          code: 'EMPTY_CONTAINER',
          message: `${container.container_type} container is empty. Add steps or remove the container.`,
          severity: 'warning',
        });
      }

      // Check for AI sequence without config
      if (container.container_type === 'ai_sequence' && !container.ai_config) {
        issues.push({
          nodeId: container.id,
          field: 'ai_config',
          code: 'MISSING_AI_CONFIG',
          message: 'AI Sequence container requires agent configuration.',
          severity: 'error',
        });
      }

      // Check for AI sequence config without agent reference
      if (
        container.container_type === 'ai_sequence' &&
        container.ai_config &&
        !container.ai_config.agent_ref
      ) {
        issues.push({
          nodeId: container.id,
          field: 'ai_config.agent_ref',
          code: 'MISSING_AI_AGENT_REF',
          message: 'AI Sequence container requires an agent reference (ai_config.agent_ref).',
          severity: 'error',
        });
      }

      // AI router is only supported on OR containers
      if (container.router && container.container_type !== 'or') {
        issues.push({
          nodeId: container.id,
          field: 'router',
          code: 'ROUTER_ON_NON_OR',
          message: `AI router is only supported on OR containers (this is "${container.container_type}").`,
          severity: 'error',
        });
      }

      // Router configuration checks (the agent decides when no REL rule matched)
      if (container.router) {
        const router = container.router;
        const childIds = new Set(container.children.map((child) => child.id));

        if (!router.agent_ref) {
          issues.push({
            nodeId: container.id,
            field: 'router.agent_ref',
            code: 'MISSING_ROUTER_AGENT',
            message: 'AI router requires an agent_ref (the agent that decides the branch).',
            severity: 'error',
          });
        }
        if (router.default_branch != null && !childIds.has(router.default_branch)) {
          issues.push({
            nodeId: container.id,
            field: 'router.default_branch',
            code: 'INVALID_DEFAULT_BRANCH',
            message: `router.default_branch "${router.default_branch}" is not a child of this container.`,
            severity: 'error',
          });
        }
        if (
          router.min_confidence != null &&
          (router.min_confidence < 0 || router.min_confidence > 1)
        ) {
          issues.push({
            nodeId: container.id,
            field: 'router.min_confidence',
            code: 'INVALID_MIN_CONFIDENCE',
            message: `router.min_confidence must be between 0 and 1 (got ${router.min_confidence}).`,
            severity: 'error',
          });
        }
      }

      // Check OR container children are reachable via a rule or condition.
      // With an AI router every child is reachable (the agent can pick any).
      if (container.container_type === 'or' && container.children.length > 0 && !container.router) {
        const routedIds = new Set(
          (container.rules ?? []).map((rule) => rule.next_step)
        );
        for (const child of container.children) {
          const hasOwnCondition =
            isFlowStep(child) && !!child.properties.condition?.trim();
          if (!routedIds.has(child.id) && !hasOwnCondition) {
            issues.push({
              nodeId: child.id,
              code: 'UNROUTED_OR_CHILD',
              message: 'Step is inside an OR container but no rule or condition routes to it.',
              severity: 'warning',
            });
          }
        }
      }

      // Competition container checks (mirrors the CLI doctor)
      if (container.container_type === 'competition') {
        if (!container.referee || !container.referee.agent_ref) {
          issues.push({
            nodeId: container.id,
            field: 'referee',
            code: 'COMPETITION_NEEDS_REFEREE',
            message: 'Competition container requires a referee with an agent_ref to judge the answers.',
            severity: 'error',
          });
        }

        const agentChildren = container.children.filter(
          (child) => isFlowStep(child) && child.properties.agent_ref != null
        );
        if (agentChildren.length < 2) {
          issues.push({
            nodeId: container.id,
            code: 'COMPETITION_TOO_FEW_AGENTS',
            message: `Competition container needs at least 2 children with agent_ref (found ${agentChildren.length}) — competitors are agents, each may use a different LLM.`,
            severity: 'error',
          });
        }
        for (const child of container.children) {
          if (isFlowStep(child) && child.properties.agent_ref == null) {
            issues.push({
              nodeId: child.id,
              code: 'COMPETITION_NON_AGENT_CHILD',
              message: 'Competition children must be agent steps (agent_ref) — this child is ignored by the competition.',
              severity: 'warning',
            });
          }
        }

        const refMinConfidence = container.referee?.min_confidence;
        if (refMinConfidence != null && (refMinConfidence < 0 || refMinConfidence > 1)) {
          issues.push({
            nodeId: container.id,
            field: 'referee.min_confidence',
            code: 'INVALID_MIN_CONFIDENCE',
            message: `referee.min_confidence must be between 0 and 1 (got ${refMinConfidence}).`,
            severity: 'error',
          });
        }
      }

      // Loop container checks (mirrors the CLI doctor)
      if (container.container_type === 'loop') {
        // EXACTLY ONE shape. Demanding `over` rejected while/times loops the
        // engine runs happily, once its designer format gained them.
        const loop = container.loop;
        const named = [
          loop?.over?.trim() ? 'over' : null,
          loop?.while?.trim() ? 'while' : null,
          typeof loop?.times === 'number' ? 'times' : null,
        ].filter((x): x is string => x != null);

        if (named.length === 0) {
          issues.push({
            nodeId: container.id,
            field: 'loop',
            code: 'LOOP_MISSING_SHAPE',
            message:
              'Loop container needs exactly one of loop.over (a collection to iterate), loop.while (a condition re-tested each iteration), or loop.times (a fixed count).',
            severity: 'error',
          });
        } else if (named.length > 1) {
          issues.push({
            nodeId: container.id,
            field: 'loop',
            code: 'LOOP_AMBIGUOUS_SHAPE',
            message: `Loop container names ${named.length} shapes at once (${named.join(', ')}); exactly one decides how it iterates.`,
            severity: 'error',
          });
        }

        if (loop?.unbounded === true) {
          if (!loop.while?.trim()) {
            issues.push({
              nodeId: container.id,
              field: 'loop.unbounded',
              code: 'LOOP_UNBOUNDED_WITHOUT_WHILE',
              message:
                'loop.unbounded applies only to a while loop - a for_each is bounded by its collection and a times by its count.',
              severity: 'error',
            });
          }
          if (loop.max_iterations != null) {
            issues.push({
              nodeId: container.id,
              field: 'loop.unbounded',
              code: 'LOOP_UNBOUNDED_WITH_MAX',
              message: 'Loop is both unbounded and capped by max_iterations; pick one.',
              severity: 'error',
            });
          }
        }
        if (container.loop?.item != null && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(container.loop.item)) {
          issues.push({
            nodeId: container.id,
            field: 'loop.item',
            code: 'LOOP_INVALID_ITEM',
            message: `loop.item "${container.loop.item}" must be a snake_case identifier ([A-Za-z0-9_]) so it can be referenced in templates and REL conditions.`,
            severity: 'error',
          });
        }
        if (container.loop?.max_iterations != null && container.loop.max_iterations < 1) {
          issues.push({
            nodeId: container.id,
            field: 'loop.max_iterations',
            code: 'LOOP_INVALID_MAX_ITERATIONS',
            message: `loop.max_iterations must be at least 1 (got ${container.loop.max_iterations}).`,
            severity: 'error',
          });
        }
      }

      // Parallel fan-out: one branch per collection item
      if (container.fan_out) {
        if (!container.fan_out.over?.trim()) {
          issues.push({
            nodeId: container.id,
            field: 'fan_out.over',
            code: 'FAN_OUT_MISSING_OVER',
            message:
              'Fan-out requires fan_out.over - the collection expression to fan out over (e.g. ${steps.plan.items}).',
            severity: 'error',
          });
        }
        if (container.fan_out.max_branches != null && container.fan_out.max_branches < 1) {
          issues.push({
            nodeId: container.id,
            field: 'fan_out.max_branches',
            code: 'FAN_OUT_INVALID_MAX_BRANCHES',
            message: `fan_out.max_branches must be at least 1 (got ${container.fan_out.max_branches}).`,
            severity: 'error',
          });
        }
        // A fan-out instantiates the children as ONE branch subgraph per
        // item, so an empty container fans out over nothing to run.
        if (container.children.length === 0) {
          issues.push({
            nodeId: container.id,
            code: 'FAN_OUT_EMPTY_BRANCH',
            message: 'Fan-out container has no children - there is no branch to run per item.',
            severity: 'error',
          });
        }
      }

      // Fan-out / merge strategy are only supported on parallel containers
      if (container.fan_out && container.container_type !== 'parallel') {
        issues.push({
          nodeId: container.id,
          field: 'fan_out',
          code: 'FAN_OUT_ON_NON_PARALLEL',
          message: `Fan-out config is only supported on parallel containers (this is "${container.container_type}").`,
          severity: 'error',
        });
      }
      if (container.merge_strategy && container.container_type !== 'parallel') {
        issues.push({
          nodeId: container.id,
          field: 'merge_strategy',
          code: 'MERGE_STRATEGY_ON_NON_PARALLEL',
          message: `Merge strategy is only supported on parallel containers (this is "${container.container_type}").`,
          severity: 'error',
        });
      }

      // Loop config is only supported on loop containers
      if (container.loop && container.container_type !== 'loop') {
        issues.push({
          nodeId: container.id,
          field: 'loop',
          code: 'LOOP_ON_NON_LOOP',
          message: `Loop config is only supported on loop containers (this is "${container.container_type}").`,
          severity: 'error',
        });
      }

      // Recursively validate children
      issues.push(...validateStepProperties(container.children, container.id));
    }
  }

  return issues;
}

function validateErrorEdges(flow: FlowDefinition): ValidationIssue[] {
  const issues: ValidationIssue[] = [];
  const allNodeIds = new Set<string>();

  // Collect all node IDs
  function collectIds(nodes: FlowNode[]) {
    for (const node of nodes) {
      allNodeIds.add(node.id);
      if (isFlowContainer(node)) {
        collectIds((node as FlowContainer).children);
      }
    }
  }
  collectIds(flow.nodes);

  // Check error edges point to valid nodes
  // (error edges may live at step level or in step.properties)
  function checkErrorEdges(nodes: FlowNode[]) {
    for (const node of nodes) {
      if (isFlowStep(node)) {
        const step = node as FlowStep;
        const errorEdge = getErrorEdge(step);
        if (errorEdge && !allNodeIds.has(errorEdge)) {
          issues.push({
            nodeId: step.id,
            field: 'error_edge',
            code: 'INVALID_ERROR_EDGE',
            message: `Error edge points to non-existent node: ${errorEdge}`,
            severity: 'error',
          });
        }
      }
      if (isFlowContainer(node)) {
        checkErrorEdges((node as FlowContainer).children);
      }
    }
  }
  checkErrorEdges(flow.nodes);

  return issues;
}

function validateConditions(nodes: FlowNode[], _parentId?: string): ValidationIssue[] {
  const issues: ValidationIssue[] = [];

  for (const node of nodes) {
    if (isFlowStep(node)) {
      const step = node as FlowStep;

      // Check for malformed conditions
      if (step.properties.condition) {
        try {
          // Basic syntax check - ensure it's not empty or just whitespace
          const condition = step.properties.condition.trim();
          if (condition === '') {
            issues.push({
              nodeId: step.id,
              field: 'condition',
              code: 'EMPTY_CONDITION',
              message: 'Condition expression is empty.',
              severity: 'warning',
            });
          }
        } catch {
          issues.push({
            nodeId: step.id,
            field: 'condition',
            code: 'INVALID_CONDITION',
            message: 'Condition expression appears to be invalid.',
            severity: 'error',
          });
        }
      }
    }

    if (isFlowContainer(node)) {
      issues.push(...validateConditions((node as FlowContainer).children, node.id));
    }
  }

  return issues;
}

/**
 * Run all validators and collect issues
 */
function runValidation(
  flow: FlowDefinition,
  customValidators: FlowValidator[] = []
): ValidationResult {
  const allIssues: ValidationIssue[] = [];

  // Run built-in validators
  allIssues.push(...validateEmptyFlow(flow));
  allIssues.push(...validateStepProperties(flow.nodes));
  allIssues.push(...validateErrorEdges(flow));
  allIssues.push(...validateConditions(flow.nodes));

  // Run custom validators
  for (const validator of customValidators) {
    try {
      allIssues.push(...validator(flow));
    } catch (error) {
      console.error('Custom validator failed:', error);
    }
  }

  // Separate by severity
  const errors = allIssues.filter(i => i.severity === 'error');
  const warnings = allIssues.filter(i => i.severity === 'warning');
  const suggestions = allIssues.filter(i => i.severity === 'suggestion');

  return {
    valid: errors.length === 0,
    errors,
    warnings,
    suggestions,
  };
}

/**
 * useFlowValidation - Validates flow definitions with debouncing
 */
export function useFlowValidation(
  flow: FlowDefinition,
  options: UseFlowValidationOptions = {}
): UseFlowValidationReturn {
  const {
    debounceMs = 300,
    customValidators = [],
    enabled = true,
  } = options;

  const [validation, setValidation] = useState<ValidationResult>(() => ({
    valid: true,
    errors: [],
    warnings: [],
    suggestions: [],
  }));
  const [isValidating, setIsValidating] = useState(false);
  const timeoutRef = useRef<ReturnType<typeof setTimeout>>();

  // Immediate validation function
  const validate = useCallback((): ValidationResult => {
    if (!enabled) {
      return { valid: true, errors: [], warnings: [], suggestions: [] };
    }
    const result = runValidation(flow, customValidators);
    setValidation(result);
    return result;
  }, [flow, customValidators, enabled]);

  // Debounced validation on flow changes
  useEffect(() => {
    if (!enabled) return;

    setIsValidating(true);

    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current);
    }

    timeoutRef.current = setTimeout(() => {
      validate();
      setIsValidating(false);
    }, debounceMs);

    return () => {
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }
    };
  }, [flow, debounceMs, validate, enabled]);

  // Get issues for a specific node
  const getNodeIssues = useCallback(
    (nodeId: string): ValidationIssue[] => {
      return [
        ...validation.errors,
        ...validation.warnings,
        ...validation.suggestions,
      ].filter(issue => issue.nodeId === nodeId);
    },
    [validation]
  );

  // Check if node has errors
  const hasNodeErrors = useCallback(
    (nodeId: string): boolean => {
      return validation.errors.some(error => error.nodeId === nodeId);
    },
    [validation]
  );

  return useMemo(
    () => ({
      validation,
      isValidating,
      validate,
      getNodeIssues,
      hasNodeErrors,
    }),
    [validation, isValidating, validate, getNodeIssues, hasNodeErrors]
  );
}

export default useFlowValidation;
