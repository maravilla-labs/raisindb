/**
 * Shared utility functions for agent handlers.
 */

import { log } from './logger.js';

/** Fallback text when an assistant turn produces no content. */
const TERMINAL_FALLBACK_TEXT = 'I could not generate a complete response. Please try again.';
const EXECUTION_MODES = new Set(['automatic', 'approve_then_auto', 'step_by_step', 'manual']);

/**
 * Return the system prompt addition for planning capabilities.
 * Injected when at least one resolved tool has category='planning'.
 */
function getPlanningSystemPromptAddition(executionMode) {
  const mode = getEffectiveExecutionMode(executionMode);
  const automaticInstructions = `CRITICAL - Automatic Execution Mode:
- After the plan is created (and approved, if required), you MUST execute ALL tasks in sequence without stopping.
- Do NOT wait for user input between tasks. Do NOT pause after completing a task.
- For each task: call update_task to mark it in_progress -> do the actual work using your tools -> call update_task to mark it completed -> immediately move to the next pending task.
- Continue this loop until every task is completed, then provide a final summary.
- You MUST use the actual tool-calling mechanism. Never write "Calling update-task" or similar as plain text - always invoke the function directly.`;

  const approveThenAutoInstructions = `CRITICAL - Approval Then Auto Mode:
- You MUST first create a plan and wait for user approval.
- After approval arrives, you MUST execute ALL tasks in sequence without stopping.
- For each task: update_task(in_progress) -> do the work with tools -> update_task(completed) -> immediately continue with the next task.
- Never pause between tasks after approval. Finish the full plan, then provide a final summary.
- Never write "Calling ..." text. Use real function/tool calls only.`;

  const stepByStepInstructions = `IMPORTANT - Step-by-Step Mode:
- Create a plan first and wait for approval.
- After approval, execute exactly ONE full task per cycle:
  1) update_task(in_progress)
  2) do the task work
  3) update_task(completed)
- Then STOP and wait for the next user continue signal before starting another task.`;

  const modeInstructions = {
    automatic: automaticInstructions,
    approve_then_auto: approveThenAutoInstructions,
    step_by_step: stepByStepInstructions,
    manual: `IMPORTANT - Manual Mode:
- Create a plan first when no plan exists yet.
- After plan approval, do NOT auto-run tasks.
- If the user gives a generic continue message (e.g. "go ahead"), ask them to pick a specific pending task.
- Only execute tasks when the user gives a specific task instruction.`,
  };

  return `
## Task Planning

You have access to planning tools for creating and managing structured plans with tasks.

When the user asks you to:
- Create a plan, break something down, or work systematically -> use create_plan
- Before starting work on a task -> mark it in_progress with update_task
- After completing a task -> mark it completed with update_task
- Need to check progress -> use get_plan_status
- Discover additional work -> use add_task

Execution Mode: ${mode}
${modeInstructions[mode] || modeInstructions.automatic}

Guidelines:
- Work through tasks one at a time
- Always mark a task as in_progress before starting and completed when done
- Use clear, actionable task titles
- Break complex work into smaller, manageable tasks
- Update task status in real-time as you work
`;
}

function getEffectiveExecutionMode(executionMode) {
  if (typeof executionMode !== 'string') return 'automatic';
  return EXECUTION_MODES.has(executionMode) ? executionMode : 'automatic';
}

function requiresPlanApproval(executionMode) {
  const mode = getEffectiveExecutionMode(executionMode);
  return mode === 'manual' || mode === 'step_by_step' || mode === 'approve_then_auto';
}

function shouldAutoRunTasks(executionMode) {
  const mode = getEffectiveExecutionMode(executionMode);
  return mode === 'automatic' || mode === 'approve_then_auto';
}

function shouldPauseAfterTask(executionMode) {
  return getEffectiveExecutionMode(executionMode) === 'step_by_step';
}

async function updateOrchestrationState(workspace, messagePath, update) {
  if (!workspace || !messagePath || !update || typeof update !== 'object') return;
  try {
    const node = await raisin.nodes.get(workspace, messagePath);
    if (!node) return;
    await raisin.nodes.update(workspace, messagePath, {
      properties: {
        ...(node.properties || {}),
        ...update,
      },
    });
  } catch (e) {
    log.warn('utils', 'Failed to update orchestration state', { path: messagePath, error: e.message });
  }
}

/**
 * Safely stringify a value, returning '[unserializable]' on failure.
 */
function safeJson(value) {
  try {
    return JSON.stringify(value);
  } catch {
    return '[unserializable]';
  }
}

/**
 * Extract text content from an assistant message's properties.
 * Handles both string and object body shapes.
 */
function readAssistantContent(props) {
  if (!props) return '';
  if (typeof props.content === 'string') return props.content;
  if (typeof props.body === 'string') return props.body;
  if (props.body && typeof props.body === 'object') {
    return props.body.content || props.body.message_text || '';
  }
  return '';
}

/**
 * Update an assistant message node's content in both the content and body fields.
 */
async function updateAssistantContent(workspace, messageNode, content) {
  const props = messageNode?.properties || {};
  const body =
    props.body && typeof props.body === 'object'
      ? { ...props.body, content, message_text: content }
      : { content, message_text: content };

  await raisin.nodes.update(workspace, messageNode.path, {
    properties: {
      ...props,
      content,
      body,
    },
  });
}

/**
 * Count AIToolCall children of an assistant message that are still pending or running.
 */
async function countPendingToolCalls(workspace, assistantPath) {
  try {
    const children = await raisin.nodes.getChildren(workspace, assistantPath);
    const pending = (children || []).filter(child => {
      if (child.node_type !== 'raisin:AIToolCall') return false;
      const status = child.properties?.status || 'pending';
      return status === 'pending' || status === 'running';
    }).length;
    log.debug('utils', 'Counted pending tool calls', { path: assistantPath, pending });
    return pending;
  } catch (e) {
    log.warn('utils', 'Failed to inspect tool calls', { path: assistantPath, error: e.message });
    return 0;
  }
}

/**
 * Check whether an assistant message has any tool-call-related children.
 * Checks for AIToolCall nodes, the AIToolResultAggregator (multi-tool
 * coordination), and direct AIToolResult / AIToolSingleCallResult nodes.
 * Used to distinguish tool-call-only responses (empty content is normal)
 * from genuinely empty responses.
 */
const TOOL_CHILD_TYPES = new Set([
  'raisin:AIToolCall',
  'raisin:AIToolResult',
  'raisin:AIToolSingleCallResult',
  'raisin:AIToolResultAggregator',
]);

async function hasCompletedToolCalls(workspace, assistantPath) {
  try {
    const children = await raisin.nodes.getChildren(workspace, assistantPath);
    return (children || []).some(c => TOOL_CHILD_TYPES.has(c.node_type));
  } catch {
    return false;
  }
}

export {
  TERMINAL_FALLBACK_TEXT,
  getPlanningSystemPromptAddition,
  safeJson,
  readAssistantContent,
  updateAssistantContent,
  countPendingToolCalls,
  hasCompletedToolCalls,
  getEffectiveExecutionMode,
  requiresPlanApproval,
  shouldAutoRunTasks,
  shouldPauseAfterTask,
  updateOrchestrationState,
};
