async function handler(input) {
  const { task_path, chat_path, workspace = 'ai', delegation_id, result, error } = input;
  const task = await raisin.nodes.get(workspace, task_path);
  if (!task || task.node_type !== 'raisin:AITask') throw new Error(`Delegated task not found: ${task_path}`);
  const props = task.properties || {};
  if (props.delegation_id !== delegation_id) throw new Error('Delegation id does not match task');

  if (error && (error.message || error.error_type)) {
    const message = error.message || String(error);
    await raisin.nodes.update(workspace, task_path, {
      properties: {
        ...props,
        status: 'pending',
        delegation_status: 'failed',
        delegation_error: message,
        delegated_completed_at: new Date().toISOString(),
      },
    });

    await raisin.functions.call('/lib/raisin/ai/update-task', {
      task_id: task.id,
      status: 'pending',
      notes: `Delegated agent failed and can be retried: ${message}`,
      __raisin_context: { workspace, chat_path },
    });

    return { success: false, task_id: task.id, status: 'failed', error: message };
  }

  const review = result?.branch_review || {};
  const rawResult = result?.structured_output ?? result?.content ?? result;
  const delegatedResult = rawResult && typeof rawResult === 'object'
    ? rawResult
    : { content: rawResult == null ? '' : String(rawResult) };
  const taskStatus = review.branch ? 'in_progress' : 'completed';
  await raisin.nodes.update(workspace, task_path, {
    properties: {
      ...props,
      status: taskStatus,
      delegation_status: review.branch ? 'awaiting_review' : 'completed',
      delegation_result: delegatedResult,
      delegation_branch: review.branch || null,
      delegation_base_branch: review.base_branch || null,
      delegated_completed_at: new Date().toISOString(),
    },
  });

  await raisin.functions.call('/lib/raisin/ai/update-task', {
    task_id: task.id,
    status: taskStatus,
    notes: review.branch ? `Child work is ready for review on branch ${review.branch}.` : 'Child work completed.',
    __raisin_context: { workspace, chat_path },
  });

  return { success: true, task_id: task.id, status: review.branch ? 'awaiting_review' : 'completed', ...review };
}
