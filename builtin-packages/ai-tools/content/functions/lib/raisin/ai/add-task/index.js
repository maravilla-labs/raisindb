/**
 * add-task — Adds a new task to the most recent plan in the conversation.
 *
 * Finds the latest raisin:AIPlan node, creates a new raisin:AITask child,
 * and updates the plan's estimated_steps count.
 *
 * Execution mode: inline
 * Category: planning
 */
async function handler(input) {
  const { title, description, priority, __raisin_context } = input;
  const workspace = __raisin_context?.workspace || 'ai';
  const chatPath = __raisin_context?.chat_path;

  if (!title) throw new Error('Task title is required');
  if (!chatPath) throw new Error('Missing chat_path in execution context');

  // Find the most recent plan in this conversation
  const plans = await raisin.sql.query(
    `SELECT id, path, properties FROM "${workspace}"
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AIPlan'
     ORDER BY created_at DESC
     LIMIT 1`,
    [chatPath],
  );

  if (plans.length === 0) {
    throw new Error('No plan exists. Use create_plan first.');
  }

  const plan = plans[0];
  const planProps = plan.properties || {};

  // Count existing tasks to determine the next number
  const countResult = await raisin.sql.query(
    `SELECT COUNT(*) as count FROM "${workspace}"
     WHERE CHILD_OF($1) AND node_type = 'raisin:AITask'`,
    [plan.path],
  );

  const taskNumber = parseInt(countResult[0]?.count || 0) + 1;
  const taskName = `task-${taskNumber}`;

  // Create the new task
  const taskNode = await raisin.nodes.create(workspace, plan.path, {
    name: taskName,
    node_type: 'raisin:AITask',
    properties: {
      title,
      description: description || '',
      status: 'pending',
      priority: priority || 'normal',
    },
  });

  if (!taskNode?.id) {
    throw new Error(taskNode?.error || 'Failed to create task');
  }

  // Update the plan's estimated_steps
  await raisin.nodes.update(workspace, plan.path, {
    properties: { ...planProps, estimated_steps: taskNumber },
  });

  return {
    success: true,
    task_id: taskNode.id,
    task_number: taskNumber,
    title,
    status: 'pending',
    priority: priority || 'normal',
    plan_title: planProps.title,
    message: `Added task ${taskNumber}: "${title}" to plan "${planProps.title}"`,
  };
}
