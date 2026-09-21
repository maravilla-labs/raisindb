/**
 * PLAN PROGRESS — recount a plan from its tasks and mirror the result onto the
 * ai_plan message cards. Split out of `index.js` (which is the gate) so neither
 * file carries the other; the gate's rule for AUTO-COMPLETION lives here,
 * because counting ticked boxes is itself a completion claim.
 */

import { FINALIZE_POLICY_VERIFIED, taskBuildTarget, verificationGaps } from '../agent-shared/finalize.js';
import { collectRunWrites, staleCompletedTasks } from '../agent-shared/run-evidence.js';

/**
 * Recalculate plan progress from task statuses and update both the plan node
 * and any ai_plan message cards in the conversation.
 */
export async function updatePlanProgress(workspace, planPath, chatPath, finalizePolicy, run = null) {
  const gateOn = finalizePolicy === FINALIZE_POLICY_VERIFIED;

  let total = 0;
  let completed = 0;
  let unverified = [];
  const verifiedRows = [];

  if (gateOn) {
    /* One row-returning query instead of the two counts, because the gate needs
     * each task's build target and verification record — a COUNT cannot answer
     * "completed AND verified". */
    const rows = await raisin.sql.query(
      `SELECT path, properties FROM "${workspace}"
       WHERE CHILD_OF($1) AND node_type = 'raisin:AITask'`,
      [planPath],
    );
    if (!rows || rows.length === 0) return null;
    for (const row of rows) {
      const props = row.properties || {};
      total++;
      if ((props.status || 'pending') !== 'completed') continue;
      completed++;
      const target = taskBuildTarget(props);
      if (!target) continue;
      const missing = verificationGaps(props, target);
      if (missing.length > 0) {
        unverified.push({ path: row.path, title: props.title || row.path, target, missing });
        continue;
      }
      verifiedRows.push({ path: row.path, props });
    }
    /* A record the run has overwritten since is not proof of what is there
     * now: a completed task whose artifact was re-written after its
     * verification keeps the plan open, exactly like a missing record. */
    if (verifiedRows.length > 0) {
      const writes = run || (chatPath ? await collectRunWrites(workspace, chatPath) : null);
      if (writes && writes.readable !== false) {
        for (const s of staleCompletedTasks({ tasks: verifiedRows }, writes)) {
          unverified.push({ path: s.path, title: s.title, target: s.target, missing: [s.stale] });
        }
      } else if (writes) {
        for (const r of verifiedRows) {
          unverified.push({ path: r.path, title: r.props.title || r.path, target: taskBuildTarget(r.props), missing: ['a readable record of this run\'s tool calls'] });
        }
      }
    }
  } else {
    // Use two separate COUNT queries because conditional COUNT with CASE WHEN
    // returns incorrect results in RaisinDB SQL (returns total instead of filtered count).
    const totalResult = await raisin.sql.query(
      `SELECT COUNT(*) as total FROM "${workspace}"
       WHERE CHILD_OF($1) AND node_type = 'raisin:AITask'`,
      [planPath],
    );
    /* NO CAST ON THE PROPERTY. A String cast here is not decoration — it changes
     * the expression the planner matches, defeats the node_type predicate, and
     * sends this hot `ai`-workspace query to a full type scan or to a foreign
     * type's index. Leave the predicate bare. */
    const completedResult = await raisin.sql.query(
      `SELECT COUNT(*) as completed FROM "${workspace}"
       WHERE CHILD_OF($1) AND node_type = 'raisin:AITask'
         AND properties->>'status' = 'completed'`,
      [planPath],
    );

    if (totalResult.length === 0) return null;

    total = parseInt(totalResult[0]?.total || 0);
    completed = parseInt(completedResult[0]?.completed || 0);
  }

  const pending = Math.max(total - completed, 0);

  const planNode = await raisin.nodes.get(workspace, planPath);
  const planProps = planNode?.properties || {};
  let nextStatus = planProps.status || 'in_progress';

  /* AUTO-COMPLETION IS A CLAIM TOO.
   *
   * Counting ticked boxes is exactly the evidence the gate refuses on a single
   * task, so it cannot be allowed to complete the whole plan. When the agent
   * declares the finalize policy, a plan holding a completed task whose build
   * target has no verification record stays in_progress. */
  if (total > 0 && completed >= total && unverified.length === 0) {
    nextStatus = 'completed';
  } else if (nextStatus === 'completed') {
    // Re-open if tasks were added after completion, or evidence went missing
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

  return {
    status: nextStatus,
    plan_id: planNode?.id || null,
    total_tasks: total,
    completed_tasks: completed,
    pending_tasks: pending,
    unverified_tasks: unverified,
  };
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
       AND properties->>'message_type' = 'ai_plan'
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
