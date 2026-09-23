/**
 * plan-approval-handler — the user's approve / reject of a plan card.
 *
 * A plan card is a PROJECTION of an agent run's plan; the decision goes to
 * that run (`raisin.agentRuns.control`, an `approve` command bound to the
 * digest of the plan the card showed). The runtime applies it atomically and
 * the run continues — or, on reject, the agent revises the plan. A card that
 * no run is waiting on (a historical conversation) is display-only.
 */

import { agentRunsAvailable, getRun, openApproval, controlRun, commandFor } from '../agent-shared/run-control.js';

async function handlePlanApproval(context) {
  const input = context.flow_input || context;
  const { action, plan_path, feedback } = input;

  if (!action || !plan_path) throw new Error('action and plan_path are required');
  if (action !== 'approve' && action !== 'reject') {
    throw new Error('action must be "approve" or "reject"');
  }

  const location = await findPlanNode(plan_path);
  if (!location) throw new Error(`Plan not found: ${plan_path}`);
  const planActionId = buildActionId(input.plan_action_id, action, plan_path);

  const routed = await decideRunApproval(location.planNode, action, feedback, planActionId, plan_path);
  if (routed) return routed;
  return {
    success: false,
    action,
    plan_path,
    message: 'No agent run is waiting on this plan; it is shown for the record only.',
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

export { handlePlanApproval };

/**
 * Approve or reject a RUN's plan through the runtime. The approval is bound
 * to the digest of the plan the card showed: if the plan changed since, the
 * runtime refuses it (`digest_mismatch`) instead of approving something else.
 */
async function decideRunApproval(planNode, action, feedback, controlId, planPath) {
  const runId = planNode?.properties?.run_id;
  if (!runId || !agentRunsAvailable()) return null;
  const view = await getRun(runId);
  const open = openApproval(view);
  if (!open) {
    return { success: false, action, plan_path: planPath, run_id: runId, message: 'This plan is not waiting for approval.' };
  }
  const digest = planNode.properties?.approval_digest || open.subject_digest;
  const ack = await controlRun(runId, commandFor(action, {
    requestId: open.request_id, subjectDigest: digest, reason: feedback || undefined,
  }), controlId);
  const accepted = !!ack && ack.ack !== 'rejected';
  return {
    success: accepted,
    action,
    plan_path: planPath,
    plan_action_id: controlId,
    run_id: runId,
    ack,
    already_applied: ack?.ack === 'duplicate',
    new_status: accepted ? (action === 'approve' ? 'in_progress' : 'cancelled') : planNode.properties?.status,
    message: accepted
      ? (action === 'approve' ? 'Plan approved. The run continues.' : 'Plan rejected. The agent will revise it.')
      : `The run refused the decision: ${ack?.reason || 'unknown'}`,
  };
}
