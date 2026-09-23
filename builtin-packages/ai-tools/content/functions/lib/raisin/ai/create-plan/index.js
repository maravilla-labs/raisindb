import { idempotencyKey, keyToken } from '../agent-shared/tool-envelope.js';
import { runEnvelope } from '../agent-shared/tool-envelope.js';
import { TOOL_META } from '../agent-shared/tool-meta.js';
/**
 * create-plan — Creates a structured plan with tasks as child nodes.
 *
 * Node tree:
 *   {msg_path}/
 *     plan-{timestamp}   (raisin:AIPlan)
 *       task-1            (raisin:AITask)
 *       task-2            (raisin:AITask)
 *
 * Execution mode: inline (runs synchronously during the AI turn)
 * Category: planning
 */
async function handler(input) {
  const enveloped = await runEnvelope(input, TOOL_META.createPlan, handler); if (enveloped) return enveloped;
  const { title, description, tasks, __raisin_context } = input;
  const workspace = __raisin_context?.workspace || 'ai';
  const msgPath = __raisin_context?.msg_path;
  const executionMode = __raisin_context?.execution_mode || 'automatic';

  if (!title) throw new Error('Plan title is required');
  if (!Array.isArray(tasks) || tasks.length === 0) {
    throw new Error('Plan must have at least one task');
  }
  if (!msgPath) throw new Error('Missing msg_path in execution context');

  // Determine initial plan status based on execution mode
  const requiresApproval =
    executionMode === 'step_by_step' ||
    executionMode === 'manual' ||
    executionMode === 'approve_then_auto';
  const planStatus = requiresApproval ? 'pending_approval' : 'in_progress';

  // Create plan node under the assistant message
  /* Keyed by the operation id under a run (or an explicit idempotency_key):
   * a replayed call finds the plan it already created. */
  const idemKey = idempotencyKey(input);
  const planName = idemKey ? `plan-${keyToken(idemKey)}` : `plan-${Date.now()}`;
  if (idemKey) {
    const prior = await raisin.nodes.get(workspace, `${msgPath}/${planName}`);
    if (prior) {
      return { success: true, replayed: true, plan_id: prior.id || null, plan_path: prior.path, title: prior.properties?.title || title, status: prior.properties?.status || planStatus, requires_approval: requiresApproval, total_tasks: tasks.length, tasks: [], message: 'This plan was already created by the same operation.' };
    }
  }
  const planNode = await raisin.nodes.create(workspace, msgPath, {
    name: planName,
    node_type: 'raisin:AIPlan',
    properties: {
      title,
      description: description || '',
      status: planStatus,
      estimated_steps: tasks.length,
      completed_steps: 0,
    },
  });

  if (!planNode?.path) {
    throw new Error(planNode?.error || 'Failed to create plan node');
  }

  const planPath = planNode.path;

  // Create task nodes as children of the plan
  const createdTasks = [];
  for (let i = 0; i < tasks.length; i++) {
    const task = tasks[i];
    const taskName = `task-${i + 1}`;

    const taskNode = await raisin.nodes.create(workspace, planPath, {
      name: taskName,
      node_type: 'raisin:AITask',
      properties: {
        title: task.title,
        description: task.description || '',
        status: 'pending',
        priority: task.priority || 'normal',
        /* A plan describes intended outcomes, not proven writes. Persisting a
         * guessed build target here makes discovery/reuse tasks impossible to
         * close: the completion gate correctly asks for verification of an
         * artifact the task never created. `update-task` binds the concrete
         * path after an authoring tool has actually returned it. */
      },
    });

    if (!taskNode?.id) {
      throw new Error(taskNode?.error || `Failed to create task ${i + 1}`);
    }

    createdTasks.push({
      task_id: taskNode.id,
      task_number: i + 1,
      title: task.title,
      status: 'pending',
      priority: task.priority || 'normal',
    });
  }

  const message = requiresApproval
    ? `Created plan "${title}" with ${tasks.length} task(s). Waiting for user approval.`
    : `Created plan "${title}" with ${tasks.length} task(s). Start with task 1: "${tasks[0].title}"`;

  return {
    success: true,
    plan_id: planNode.id || null,
    plan_path: planPath,
    title,
    status: planStatus,
    requires_approval: requiresApproval,
    total_tasks: tasks.length,
    tasks: createdTasks,
    message,
  };
}

export { handler };
