/**
 * get-plan-status — Returns the current plan and all task statuses.
 *
 * Finds the most recent raisin:AIPlan in the conversation, queries its
 * raisin:AITask children, and returns a structured summary with counts.
 * Used by the continue-handler to detect plan completion.
 *
 * Execution mode: inline
 * Category: planning
 */
async function handler(input) {
  const { __raisin_context } = input;
  const workspace = __raisin_context?.workspace || 'ai';
  const chatPath = __raisin_context?.chat_path;

  if (!chatPath) throw new Error('Missing chat_path in execution context');

  // Find the most recent plan
  const plans = await raisin.sql.query(
    `SELECT id, path, properties FROM "${workspace}"
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AIPlan'
     ORDER BY created_at DESC
     LIMIT 1`,
    [chatPath],
  );

  if (plans.length === 0) {
    return {
      has_plan: false,
      message: 'No plan exists in this conversation. Use create_plan to create one.',
    };
  }

  const plan = plans[0];
  const planProps = plan.properties || {};

  // Get all tasks ordered by creation time
  const tasks = await raisin.sql.query(
    `SELECT id, properties FROM "${workspace}"
     WHERE CHILD_OF($1) AND node_type = 'raisin:AITask'
     ORDER BY created_at ASC`,
    [plan.path],
  );

  const taskList = tasks.map((task, i) => ({
    task_id: task.id,
    task_number: i + 1,
    title: task.properties?.title || 'Untitled',
    description: task.properties?.description || '',
    status: task.properties?.status || 'pending',
    priority: task.properties?.priority || 'normal',
    delegation: task.properties?.delegation_id ? {
      id: task.properties.delegation_id,
      status: task.properties.delegation_status || 'unknown',
      agent_ref: task.properties.delegated_agent_ref || null,
      flow_instance_id: task.properties.delegation_flow_instance_id || null,
      branch: task.properties.delegation_branch || null,
      base_branch: task.properties.delegation_base_branch || null,
      result: task.properties.delegation_result || null,
      error: task.properties.delegation_error || null,
    } : null,
  }));

  const pending = taskList.filter(t => t.status === 'pending');
  const inProgress = taskList.filter(t => t.status === 'in_progress');
  const completed = taskList.filter(t => t.status === 'completed');

  const summary = `Plan "${planProps.title}": ${completed.length}/${taskList.length} tasks completed` +
    (pending.length > 0 ? `. Next: "${pending[0].title}"` : '. All tasks done!');

  return {
    has_plan: true,
    plan_id: plan.id,
    plan_path: plan.path,
    title: planProps.title || 'Untitled Plan',
    description: planProps.description || '',
    status: planProps.status || 'in_progress',
    total_tasks: taskList.length,
    completed_tasks: completed.length,
    in_progress_tasks: inProgress.length,
    pending_tasks: pending.length,
    tasks: taskList,
    next_task: pending[0] || null,
    message: summary,
  };
}
