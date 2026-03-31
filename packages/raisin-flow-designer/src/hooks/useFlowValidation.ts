/**
 * useFlowValidation Hook
 *
 * Provides flow validation with debouncing for real-time feedback.
 * Validates flow structure, step configurations, and error handling paths.
 */

import { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import type { FlowDefinition, FlowNode, FlowStep, FlowContainer } from '../types';
import type { ValidationResult, ValidationIssue } from '../context/FlowDesignerContext';
import { isFlowStep, isFlowContainer } from '../utils';

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

      // Check for human task step without task type
      if (step.properties.step_type === 'human_task' && !step.properties.task_type) {
        issues.push({
          nodeId: step.id,
          field: 'task_type',
          code: 'MISSING_TASK_TYPE',
          message: 'Human Task step requires a task type (approval, input, review, or action).',
          severity: 'error',
        });
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

      // Check for error edge pointing to non-existent node
      if (step.error_edge) {
        // Note: We'd need to validate this against all nodes, done in validateErrorEdges
      }
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
  function checkErrorEdges(nodes: FlowNode[]) {
    for (const node of nodes) {
      if (isFlowStep(node)) {
        const step = node as FlowStep;
        if (step.error_edge && !allNodeIds.has(step.error_edge)) {
          issues.push({
            nodeId: step.id,
            field: 'error_edge',
            code: 'INVALID_ERROR_EDGE',
            message: `Error edge points to non-existent node: ${step.error_edge}`,
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
