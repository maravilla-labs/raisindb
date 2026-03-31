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
  const planName = `plan-${Date.now()}`;
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
