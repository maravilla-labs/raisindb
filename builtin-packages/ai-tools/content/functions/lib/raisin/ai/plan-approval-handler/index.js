/**
 * plan-approval-handler — Handles user approval or rejection of AI-generated plans.
 *
 * Called when a user clicks approve/reject in the UI for approval-gated plans.
 *
 * Actions:
 * - approve: Sets plan status to 'in_progress'. Continuation creation depends on mode:
 *   approve_then_auto (run all), step_by_step (run one), manual (wait for explicit task instruction).
 * - reject: Sets plan status to 'cancelled', creates a continuation message
 *   asking the agent to revise.
 *
 * Execution mode: async
 */

import { getEffectiveExecutionMode } from '../agent-shared/utils.js';

async function handlePlanApproval(context) {
  const input = context.flow_input || context;
  const { action, plan_path, feedback } = input;

  if (!action || !plan_path) throw new Error('action and plan_path are required');
  if (action !== 'approve' && action !== 'reject') {
    throw new Error('action must be "approve" or "reject"');
  }

  // Derive conversation paths from the plan path
  // Plan path: {chatPath}/{msgName}/{planName}
  const pathParts = plan_path.split('/');
  const planName = pathParts.pop();
  const msgName = pathParts.pop();
  const chatPath = pathParts.join('/');
  const msgPath = `${chatPath}/${msgName}`;

  // Find the plan node (it may be in ai or raisin:access_control workspace)
  const location = await findPlanNode(plan_path);
  if (!location) throw new Error(`Plan not found: ${plan_path}`);

  const { workspace, planNode } = location;
  const currentStatus = planNode.properties?.status;
  const planActionId = buildActionId(input.plan_action_id, action, plan_path);

  // Check for idempotent re-application
  if (action === 'approve' && (currentStatus === 'in_progress' || currentStatus === 'completed')) {
    await syncAllPlanCards(workspace, chatPath, plan_path, planNode.id, currentStatus, planActionId);
    return { success: true, action, plan_path, plan_action_id: planActionId, new_status: currentStatus, already_applied: true };
  }
  if (action === 'reject' && currentStatus === 'cancelled') {
    await syncAllPlanCards(workspace, chatPath, plan_path, planNode.id, 'cancelled', planActionId);
    return { success: true, action, plan_path, plan_action_id: planActionId, new_status: 'cancelled', already_applied: true };
  }

  if (currentStatus !== 'pending_approval') {
    throw new Error(`Cannot ${action} plan in status '${currentStatus || 'unknown'}'`);
  }

  const newStatus = action === 'approve' ? 'in_progress' : 'cancelled';

  // Update the plan node
  const updatedProps = { ...planNode.properties, status: newStatus, last_plan_action_id: planActionId };
  if (action === 'reject' && feedback) updatedProps.rejection_feedback = feedback;
  await raisin.nodes.update(workspace, plan_path, { properties: updatedProps });

  // Sync ai_plan message cards in both workspaces
  await syncAllPlanCards(workspace, chatPath, plan_path, planNode.id, newStatus, planActionId);

  const executionMode = await resolveAgentExecutionMode(workspace, chatPath);
  const mode = getEffectiveExecutionMode(executionMode);
  await updatePlanApprovalMetadata(workspace, msgPath, action, planActionId);

  const shouldCreateContinuation =
    action === 'reject' ||
    (action === 'approve' && (mode === 'approve_then_auto' || mode === 'step_by_step'));

  // Create a continuation message to trigger the agent when policy allows it
  const continuation = shouldCreateContinuation
    ? await createContinuationMessage(workspace, chatPath, action, plan_path, planActionId, feedback, mode)
    : null;

  console.log(`[plan-approval] Plan ${action}d: ${plan_path}`);

  return {
    success: true,
    action,
    plan_path,
    plan_action_id: planActionId,
    new_status: newStatus,
    already_applied: false,
    continuation_message_path: continuation?.path || null,
    feedback: feedback || null,
    message: action === 'approve'
      ? (shouldCreateContinuation
        ? 'Plan approved. Agent will continue according to execution mode.'
        : 'Plan approved. Waiting for explicit task instruction.')
      : 'Plan rejected. Agent will propose a new plan.',
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function buildActionId(provided, action, planPath) {
  if (typeof provided === 'string' && provided.trim()) return provided.trim();
  // FNV-1a hash for a stable, deterministic ID
  const text = `${action}:${planPath}`;
  let hash = 2166136261;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash += (hash << 1) + (hash << 4) + (hash << 7) + (hash << 8) + (hash << 24);
  }
  return `plan-action-${action}-${(hash >>> 0).toString(16)}`;
}

async function findPlanNode(planPath) {
  const preferred = planPath.startsWith('/agents/')
    ? ['ai', 'raisin:access_control']
    : ['raisin:access_control', 'ai'];

  for (const ws of preferred) {
    try {
      const node = await raisin.nodes.get(ws, planPath);
      if (node) return { workspace: ws, planNode: node };
    } catch (_) {
      // Not found in this workspace
    }
  }
  return null;
}

/**
 * Update ai_plan message cards in a workspace to reflect the new status.
 */
async function updatePlanCards(workspace, chatPath, planPath, planId, newStatus, planActionId) {
  const rows = await raisin.sql.query(
    `SELECT path, properties FROM "${workspace}"
     WHERE DESCENDANT_OF($1)
       AND node_type = 'raisin:Message'
       AND properties->>'message_type'::STRING = 'ai_plan'
     ORDER BY created_at ASC`,
    [chatPath],
  );

  let updated = 0;
  for (const row of rows || []) {
    const props = row.properties || {};
    const data = typeof props.data === 'object' ? (props.data || {}) : {};
    if (props.message_type !== 'ai_plan') continue;

    // Match by plan_path or plan_id
    const match = (data.plan_path && data.plan_path === planPath) ||
                  (planId && data.plan_id === planId) ||
                  samePlanLeaf(data.plan_path, planPath);
    if (!match) continue;

    const updatedData = { ...data, status: newStatus };
    if (newStatus === 'in_progress' && updatedData.requires_approval === true) {
      updatedData.requires_approval = false;
    }
    if (planActionId) updatedData.last_plan_action_id = planActionId;

    // Also update tool_result data if this is a create_plan result
    if (props.message_type === 'ai_tool_result' && data.tool_name === 'create_plan' && data.result) {
      const result = typeof data.result === 'object' ? { ...data.result } : {};
      result.status = newStatus;
      if (newStatus === 'in_progress') result.requires_approval = false;
      updatedData.result = result;
    }

    await raisin.nodes.update(workspace, row.path, {
      properties: { ...props, data: updatedData },
    });
    updated++;
  }

  // Fallback: if no cards matched by path, update the latest pending one
  if (updated === 0 && rows && rows.length > 0) {
    const pending = rows
      .filter(r => r.properties?.message_type === 'ai_plan' && (r.properties?.data?.status || '') === 'pending_approval')
      .sort((a, b) => (Date.parse(b.properties?.created_at || '') || 0) - (Date.parse(a.properties?.created_at || '') || 0));

    if (pending.length > 0) {
      const target = pending[0];
      const props = target.properties || {};
      const data = typeof props.data === 'object' ? { ...props.data } : {};
      data.status = newStatus;
      if (newStatus === 'in_progress') data.requires_approval = false;
      if (planActionId) data.last_plan_action_id = planActionId;
      await raisin.nodes.update(workspace, target.path, { properties: { ...props, data } });
      console.log('[plan-approval] Fallback-updated latest pending card:', target.path);
    }
  }

  return updated;
}

function samePlanLeaf(pathA, pathB) {
  if (!pathA || !pathB) return false;
  if (pathA === pathB) return true;
  return pathA.split('/').pop() === pathB.split('/').pop();
}

/**
 * Update plan cards in the primary workspace and any mirrored conversations.
 */
async function syncAllPlanCards(primaryWorkspace, chatPath, planPath, planId, newStatus, planActionId) {
  await updatePlanCards(primaryWorkspace, chatPath, planPath, planId, newStatus, planActionId);

  if (primaryWorkspace !== 'ai') return;

  // Find mirrored conversation in raisin:access_control
  try {
    const agentChat = await raisin.nodes.get('ai', chatPath);
    const humanSenderPath = agentChat?.properties?.human_sender_path;
    const conversationId = chatPath.split('/').pop();
    if (!conversationId) return;

    const mirrorPaths = new Set();
    if (humanSenderPath) mirrorPaths.add(`${humanSenderPath}/inbox/chats/${conversationId}`);

    // Also search by conversation ID in case the path doesn't match
    const found = await raisin.sql.query(
      `SELECT path FROM "raisin:access_control"
       WHERE node_type = 'raisin:Conversation' AND path LIKE $1
       ORDER BY created_at DESC`,
      [`%/inbox/chats/${conversationId}`],
    );
    for (const row of found || []) if (row.path) mirrorPaths.add(row.path);

    for (const mirrorPath of mirrorPaths) {
      try {
        const mirrorNode = await raisin.nodes.get('raisin:access_control', mirrorPath);
        if (mirrorNode) {
          await updatePlanCards('raisin:access_control', mirrorPath, planPath, planId, newStatus, planActionId);
        }
      } catch (_) {
        // Mirror not found, skip
      }
    }
  } catch (err) {
    console.log('[plan-approval] Mirror sync failed:', err.message);
  }
}

async function updatePlanApprovalMetadata(workspace, msgPath, action, planActionId) {
  try {
    const msgNode = await raisin.nodes.get(workspace, msgPath);
    if (!msgNode) return;
    const props = msgNode.properties || {};
    await raisin.nodes.update(workspace, msgPath, {
      properties: {
        ...props,
        approval_status: action === 'approve' ? 'approved' : 'rejected',
        approved_plan_action_id: planActionId,
        approved_at: new Date().toISOString(),
      },
    });
  } catch (err) {
    console.log('[plan-approval] Failed to update assistant message:', err.message);
  }
}

async function resolveAgentExecutionMode(workspace, chatPath) {
  try {
    const chatNode = await raisin.nodes.get(workspace, chatPath);
    const agentRef = chatNode?.properties?.agent_ref;
    const agentPath = typeof agentRef === 'string' ? agentRef : agentRef?.['raisin:path'];
    if (!agentPath) return 'automatic';

    const agentWorkspace = typeof agentRef === 'object'
      ? (agentRef['raisin:workspace'] || 'functions')
      : 'functions';
    const agentNode = await raisin.nodes.get(agentWorkspace, agentPath);
    return getEffectiveExecutionMode(agentNode?.properties?.execution_mode);
  } catch {
    return 'automatic';
  }
}

/**
 * Create a user-role continuation message that triggers the agent to resume.
 */
async function createContinuationMessage(workspace, chatPath, action, planPath, planActionId, feedback, mode) {
  const continuationName = `msg-plan-action-${planActionId}`;
  const continuationPath = `${chatPath}/${continuationName}`;

  // Idempotency: don't create if it already exists
  try {
    const existing = await raisin.nodes.get(workspace, continuationPath);
    if (existing) return existing;
  } catch (_) {
    // Doesn't exist, proceed to create
  }

  let content;
  if (action === 'approve') {
    // Load plan node to get title + first task for a more explicit prompt
    let planTitle = 'the plan';
    let firstTask = null;
    try {
      const planNode = await raisin.nodes.get(workspace, planPath);
      if (planNode?.properties?.title) planTitle = planNode.properties.title;
      const children = await raisin.nodes.getChildren(workspace, planPath);
      const tasks = (children || []).filter(c => c.node_type === 'raisin:AITask');
      if (tasks.length > 0) firstTask = tasks[0].properties?.title;
    } catch (_) { /* best-effort */ }

    if (mode === 'step_by_step') {
      content = firstTask
        ? `The plan "${planTitle}" has been approved. Execute exactly one task now: mark task 1 ("${firstTask}") as in_progress, complete it, mark it completed, then stop and wait.`
        : `The plan "${planTitle}" has been approved. Execute exactly one task now, mark it completed, then stop and wait for the next continue signal.`;
    } else if (mode === 'approve_then_auto') {
      content = firstTask
        ? `The plan "${planTitle}" has been approved. Start executing now. First, call update_task to mark task 1 ("${firstTask}") as in_progress, then execute it and continue through all remaining tasks.`
        : `The plan "${planTitle}" has been approved. Start executing all tasks now. Use update_task to mark each task as in_progress before executing it and completed when done.`;
    } else {
      content = `The plan "${planTitle}" has been approved.`;
    }
  } else {
    content = feedback?.trim()
      ? `I'd like to reject this plan. ${feedback.trim()}\n\nPlease revise the plan based on this feedback.`
      : "I'd like to reject this plan. Please create a different plan.";
  }

  const node = await raisin.nodes.create(workspace, chatPath, {
    name: continuationName,
    node_type: 'raisin:Message',
    properties: {
      role: 'user',
      body: { content, message_text: content },
      content,
      sender_id: 'system',
      sender_display_name: 'System',
      message_type: 'chat',
      status: 'delivered',
      created_at: new Date().toISOString(),
      is_system_generated: true,
      plan_action_id: planActionId,
      plan_path: planPath,
      plan_action: action,
    },
  });

  return node;
}

export { handlePlanApproval };
