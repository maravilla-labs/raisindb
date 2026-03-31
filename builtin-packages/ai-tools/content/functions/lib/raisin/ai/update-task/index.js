/**
 * update-task — Updates a task's status and propagates progress to the parent plan.
 *
 * Status transitions: pending -> in_progress -> completed | cancelled
 * When all tasks complete, the parent plan is marked completed.
 *
 * Also updates any ai_plan message cards in the conversation so the frontend
 * plan projection stays in sync.
 *
 * Execution mode: inline
 * Category: planning
 */

const VALID_STATUSES = ['pending', 'in_progress', 'completed', 'cancelled'];

async function handler(input) {
  const { task_id, status, notes, __raisin_context } = input;
  const workspace = __raisin_context?.workspace || 'ai';
  const chatPath = __raisin_context?.chat_path;

  if (!task_id) throw new Error('task_id is required');
  if (!status) throw new Error('status is required');
  if (!VALID_STATUSES.includes(status)) {
    throw new Error(`Invalid status "${status}". Must be one of: ${VALID_STATUSES.join(', ')}`);
  }
  if (!chatPath) throw new Error('Missing chat_path in execution context');

  // Find the task by ID within this conversation
  const taskRows = await raisin.sql.query(
    `SELECT id, path, properties FROM "${workspace}"
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AITask' AND id = $2
     LIMIT 1`,
    [chatPath, task_id],
  );

  if (taskRows.length === 0) throw new Error(`Task not found: ${task_id}`);

  const taskNode = taskRows[0];
  const oldProps = taskNode.properties || {};
  const oldStatus = oldProps.status || 'pending';

  // Build updated properties
  const updatedProps = { ...oldProps, status };
  if (notes) updatedProps.completion_notes = notes;

  await raisin.nodes.update(workspace, taskNode.path, { properties: updatedProps });

  // Propagate progress to parent plan
  const planPath = taskNode.path.split('/').slice(0, -1).join('/');
  const progress = await updatePlanProgress(workspace, planPath, chatPath);

  return {
    success: true,
    task_id,
    title: oldProps.title,
    old_status: oldStatus,
    new_status: status,
    plan_path: planPath,
    plan_id: progress?.plan_id || null,
    plan_status: progress?.status || null,
    total_tasks: progress?.total_tasks ?? null,
    completed_tasks: progress?.completed_tasks ?? null,
    pending_tasks: progress?.pending_tasks ?? null,
    message: `Task "${oldProps.title}" marked as ${status}`,
  };
}

/**
 * Recalculate plan progress from task statuses and update both the plan node
 * and any ai_plan message cards in the conversation.
 */
async function updatePlanProgress(workspace, planPath, chatPath) {
  // Use two separate COUNT queries because conditional COUNT with CASE WHEN
  // returns incorrect results in RaisinDB SQL (returns total instead of filtered count).
  const totalResult = await raisin.sql.query(
    `SELECT COUNT(*) as total FROM "${workspace}"
     WHERE CHILD_OF($1) AND node_type = 'raisin:AITask'`,
    [planPath],
  );
  const completedResult = await raisin.sql.query(
    `SELECT COUNT(*) as completed FROM "${workspace}"
     WHERE CHILD_OF($1) AND node_type = 'raisin:AITask'
       AND properties->>'status'::String = 'completed'`,
    [planPath],
  );

  if (totalResult.length === 0) return null;

  const total = parseInt(totalResult[0]?.total || 0);
  const completed = parseInt(completedResult[0]?.completed || 0);
  const pending = Math.max(total - completed, 0);

  const planNode = await raisin.nodes.get(workspace, planPath);
  const planProps = planNode?.properties || {};
  let nextStatus = planProps.status || 'in_progress';

  // Auto-complete plan when all tasks are done
  if (total > 0 && completed >= total) {
    nextStatus = 'completed';
  } else if (nextStatus === 'completed') {
    // Re-open if tasks were added after completion
    nextStatus = 'in_progress';
  }

  await raisin.nodes.update(workspace, planPath, {
    properties: {
      ...planProps,
      status: nextStatus,
      completed_steps: completed,
      estimated_steps: total,
    },
  });

  // Update ai_plan message cards in the conversation
  if (chatPath) {
    await syncPlanMessageCards(workspace, chatPath, planPath, nextStatus);
  }

  return { status: nextStatus, plan_id: planNode?.id || null, total_tasks: total, completed_tasks: completed, pending_tasks: pending };
}

/**
 * Update the status in ai_plan message nodes so the frontend plan projection
 * reflects the latest state.
 */
async function syncPlanMessageCards(workspace, chatPath, planPath, nextStatus) {
  const rows = await raisin.sql.query(
    `SELECT path, properties FROM "${workspace}"
     WHERE DESCENDANT_OF($1)
       AND node_type = 'raisin:Message'
       AND properties->>'message_type'::String = 'ai_plan'
     ORDER BY created_at ASC`,
    [chatPath],
  );

  for (const row of rows || []) {
    const props = row.properties || {};
    const data = props.data || {};
    if (props.message_type !== 'ai_plan') continue;
    if (data.plan_path !== planPath) continue;

    const updatedData = { ...data, status: nextStatus };
    if (nextStatus === 'in_progress' && updatedData.requires_approval === true) {
      updatedData.requires_approval = false;
    }

    await raisin.nodes.update(workspace, row.path, {
      properties: { ...props, data: updatedData },
    });
  }

  // Also update mirrored conversation in the user's workspace
  if (workspace === 'ai') {
    await syncMirroredPlanCards(chatPath, planPath, nextStatus);
  }
}

/**
 * If the primary conversation is in the AI workspace, find the mirrored copy
 * in raisin:access_control and update its plan cards too.
 */
async function syncMirroredPlanCards(chatPath, planPath, nextStatus) {
  try {
    const agentChat = await raisin.nodes.get('ai', chatPath);
    const humanSenderPath = agentChat?.properties?.human_sender_path;
    if (!humanSenderPath) return;

    const conversationId = chatPath.split('/').pop();
    if (!conversationId) return;

    const mirrorChatPath = `${humanSenderPath}/inbox/chats/${conversationId}`;
    const mirrorChat = await raisin.nodes.get('raisin:access_control', mirrorChatPath);
    if (!mirrorChat) return;

    await syncPlanMessageCards('raisin:access_control', mirrorChatPath, planPath, nextStatus);
  } catch (err) {
    console.log('[update-task] Mirror sync failed:', err.message);
  }
}
