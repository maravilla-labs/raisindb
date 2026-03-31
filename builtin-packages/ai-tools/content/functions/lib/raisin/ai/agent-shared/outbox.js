/**
 * Agent outbox messaging for cross-workspace delivery.
 *
 * When an AI agent (workspace 'ai') needs to reply to a human user
 * (workspace 'raisin:access_control'), it drops a raisin:Message node
 * into its outbox.  The process-chat trigger picks it up and mirrors
 * it into the human's inbox conversation.
 */

import { log } from './logger.js';

/**
 * Resolve outbox context for an agent conversation.
 *
 * Only applies when the handler runs in workspace 'ai' on a path matching
 * /agents/{name}/inbox/chats/{conversationId}.  Returns the routing
 * information needed by sendAgentOutboxMessage(), or null if outbox
 * delivery is not applicable.
 */
async function resolveAgentOutboxContext(workspace, chatPath, chatNode) {
  log.debug('outbox', 'Resolving outbox context', { workspace, chat: chatPath });

  if (workspace !== 'ai') return null;

  const parts = chatPath.split('/');
  // Expected: /agents/{name}/inbox/chats/{conversationId}
  if (parts.length < 6 || parts[1] !== 'agents' || parts[3] !== 'inbox' || parts[4] !== 'chats') {
    return null;
  }

  const agentName = parts[2];
  const agentHomePath = `/agents/${agentName}`;
  const conversationId = parts[parts.length - 1];

  const agentHome = await raisin.nodes.get('ai', agentHomePath);
  if (!agentHome) return null;

  const homeProps = agentHome.properties || {};
  const agentUserId = homeProps.user_id || `agent:${agentName}`;
  const agentDisplayName = homeProps.display_name || agentName;

  // Identify the human participant
  const chatProps = chatNode?.properties || {};
  const participants = chatProps.participants || [];

  let senderId = chatProps.human_sender_id || null;
  let senderPath = chatProps.human_sender_path || null;

  if (!senderId || !senderPath) {
    // Find the first non-agent participant
    for (const pid of participants) {
      if (typeof pid === 'string' && !pid.startsWith('agent:')) {
        senderId = pid;
        break;
      }
    }

    // Look up the user's home path in the access_control workspace
    if (senderId && !senderPath) {
      const users = await raisin.sql.query(`
        SELECT path FROM 'raisin:access_control'
        WHERE node_type = 'raisin:User'
          AND properties->>'user_id'::String = $1
        LIMIT 1
      `, [senderId]);
      if (users.length > 0) {
        senderPath = users[0].path;
      }
    }
  }

  if (!senderId || !senderPath) {
    log.warn('outbox', 'Could not resolve sender for outbox');
    return null;
  }

  log.debug('outbox', 'Outbox context resolved', { agent: agentUserId, sender: senderId });

  return {
    agentHomePath,
    agentUserId,
    agentDisplayName,
    senderId,
    senderPath,
    conversationId,
  };
}

/**
 * Create a message node in the agent's outbox folder.
 * The process-chat trigger will detect it and mirror it to the human's inbox.
 */
function hashDedupeKey(input) {
  const text = String(input || '');
  let hash = 2166136261;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash += (hash << 1) + (hash << 4) + (hash << 7) + (hash << 8) + (hash << 24);
  }
  return (hash >>> 0).toString(16);
}

async function sendAgentOutboxMessage(workspace, outboxCtx, content, messageType, data, options = {}) {
  log.debug('outbox', 'Sending outbox message', { type: messageType, recipient: outboxCtx.senderId });

  const outboxPath = outboxCtx.agentHomePath + '/outbox';
  const dedupeKey = typeof options?.dedupe_key === 'string' && options.dedupe_key.trim()
    ? options.dedupe_key.trim()
    : null;
  const slug = dedupeKey
    ? `msg-${messageType}-${hashDedupeKey(dedupeKey)}`
    : `msg-${Date.now()}-${Math.floor(Math.random() * 100000)}`;

  // Determine role based on message type — agent outbox messages are always from the assistant
  const role = (messageType === 'chat' || messageType === 'ai_tool_call'
    || messageType === 'ai_tool_result' || messageType === 'ai_thought'
    || messageType === 'ai_plan' || messageType === 'ai_task_update')
    ? 'assistant' : undefined;

  try {
    await raisin.nodes.create(workspace, outboxPath, {
      slug,
      name: slug,
      node_type: 'raisin:Message',
      properties: {
        ...(role ? { role } : {}),
        message_type: messageType,
        status: 'pending',
        sender_id: outboxCtx.agentUserId,
        sender_path: outboxCtx.agentHomePath,
        recipient_id: outboxCtx.senderId,
        recipient_path: outboxCtx.senderPath,
        created_at: new Date().toISOString(),
        body: {
          message_text: content,
          content,
          thread_id: outboxCtx.conversationId,
          sender_display_name: outboxCtx.agentDisplayName,
        },
        conversation_id: outboxCtx.conversationId,
        data: data || {},
      },
    });
  } catch (err) {
    if (dedupeKey && String(err?.message || '').includes('already exists')) {
      log.debug('outbox', 'Skipping duplicate outbox message', {
        type: messageType,
        dedupe_key: dedupeKey,
      });
      return;
    }
    throw err;
  }
}

/**
 * Build structured outbox data for a plan creation result.
 * Returns null if the tool call was not create_plan or if it failed.
 */
function buildPlanOutboxData(toolName, toolArgs, planningResult) {
  const normalized = (toolName || '').replace(/-/g, '_');
  if (normalized !== 'create_plan') return null;
  if (!planningResult || planningResult.error) return null;

  return {
    plan_id: planningResult.plan_id || null,
    plan_path: planningResult.plan_path || null,
    title: planningResult.title || toolArgs?.title || 'Plan',
    description: planningResult.description || toolArgs?.description || '',
    tasks: Array.isArray(planningResult.tasks)
      ? planningResult.tasks
      : (Array.isArray(toolArgs?.tasks) ? toolArgs.tasks : []),
    status: planningResult.status || (planningResult.requires_approval ? 'pending_approval' : 'in_progress'),
    requires_approval: !!planningResult.requires_approval,
  };
}

/**
 * Build structured outbox data for task update / plan status tools.
 * Returns null if the tool call is not update_task or get_plan_status.
 */
function buildTaskUpdateOutboxData(toolName, toolArgs, result) {
  const normalized = (toolName || '').replace(/-/g, '_');
  if (normalized !== 'update_task' && normalized !== 'get_plan_status') return null;
  if (!result || result.error) return null;

  return {
    task_id: result.task_id || toolArgs?.task_id || null,
    title: result.title || toolArgs?.title || null,
    status: result.new_status || result.status || toolArgs?.status || null,
    plan_path: result.plan_path || null,
    plan_id: result.plan_id || null,
    plan_status: result.plan_status || null,
    total_tasks: result.total_tasks ?? null,
    completed_tasks: result.completed_tasks ?? null,
    pending_tasks: result.pending_tasks ?? null,
  };
}

export {
  resolveAgentOutboxContext,
  sendAgentOutboxMessage,
  buildPlanOutboxData,
  buildTaskUpdateOutboxData,
};
