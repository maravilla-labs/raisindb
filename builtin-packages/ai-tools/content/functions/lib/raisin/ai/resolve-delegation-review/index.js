/** Record the result of an explicit, authenticated delegated-branch review. */
async function handler(input) {
  const { task_id, plan_path, workspace = 'ai', branch, action } = input;
  if (!task_id || !plan_path || !branch) throw new Error('task_id, plan_path, and branch are required');
  if (!['merged', 'discarded'].includes(action)) throw new Error('action must be merged or discarded');

  const rows = await raisin.sql.query(
    `SELECT id, path, properties FROM "${workspace}"
     WHERE CHILD_OF($1) AND node_type = 'raisin:AITask' AND id = $2
     LIMIT 1`,
    [plan_path, task_id],
  );
  if (!rows.length) throw new Error(`Task not found in conversation: ${task_id}`);

  const task = rows[0];
  const props = task.properties || {};
  if (props.delegation_status !== 'awaiting_review') {
    throw new Error(`Delegation is not awaiting review (status: ${props.delegation_status || 'unknown'})`);
  }
  if (props.delegation_branch !== branch) throw new Error('Review branch does not match the delegated task');
  const messageMarker = '/messages/';
  const markerIndex = plan_path.indexOf(messageMarker);
  if (markerIndex < 1) throw new Error('Plan path is not attached to a conversation message');
  const chatPath = plan_path.slice(0, markerIndex);

  const nextStatus = action === 'merged' ? 'completed' : 'pending';
  const delegationStatus = action === 'merged' ? 'merged' : 'discarded';
  await raisin.nodes.update(workspace, task.path, {
    properties: {
      ...props,
      status: nextStatus,
      delegation_status: delegationStatus,
      delegation_review_action: action,
      delegation_reviewed_at: new Date().toISOString(),
    },
  });

  await raisin.functions.call('/lib/raisin/ai/update-task', {
    task_id: task.id,
    status: nextStatus,
    notes: action === 'merged'
      ? `Delegated work reviewed and merged from ${branch}.`
      : `Delegated work rejected and branch ${branch} discarded.`,
    __raisin_context: { workspace, chat_path: chatPath },
  });

  return { success: true, task_id: task.id, status: nextStatus, delegation_status: delegationStatus };
}
