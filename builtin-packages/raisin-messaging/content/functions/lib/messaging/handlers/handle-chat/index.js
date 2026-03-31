/**
 * Handle Chat Message — Core messaging delivery handler
 *
 * Delivers outbox messages to recipient inboxes with cross-workspace support.
 * Users live in raisin:access_control, agents live in ai workspace.
 *
 * Flow:
 *   1. Validate message type
 *   2. Copy to sender's sent folder
 *   3. Resolve sender and recipient (cross-workspace: users vs agents)
 *   4. Check permissions (auto-allow for agents, configurable for users)
 *   5. Upsert conversation in both sender and recipient inboxes
 *   6. Create delivered message copy in recipient's conversation
 *   7. Create notification for human recipients (skip for agents + AI intermediates)
 *   8. Mark original outbox message as delivered
 *
 * Trigger: process-chat fires on raisin:Message creation in outbox paths
 *          with status "pending" and supported message_type values.
 */

const AI_INTERMEDIATE_TYPES = new Set([
  'ai_thought', 'ai_tool_call', 'ai_tool_result', 'ai_plan', 'ai_task_update'
]);

const ALLOWED_MESSAGE_TYPES = new Set([
  'chat', 'direct_message',
  'ai_thought', 'ai_tool_call', 'ai_tool_result', 'ai_plan', 'ai_task_update'
]);

// ─── Entry Point ────────────────────────────────────────────────────────────

export async function handle_chat(input) {
  const flowInput = input.flow_input ?? input;
  const node = flowInput.node;
  const triggerWorkspace = flowInput.workspace ?? 'raisin:access_control';

  if (!node) throw new Error('No message node provided');

  const props = node.properties ?? {};
  const nodePath = node.path ?? '';
  const messageId = node.id;
  const messageType = props.message_type;

  if (!ALLOWED_MESSAGE_TYPES.has(messageType)) {
    throw new Error(`Unsupported message type: ${messageType}`);
  }

  // 1. Copy to sender's sent folder
  await copyToSentFolder(triggerWorkspace, node);

  // 2. Extract sender/recipient hints from node
  const body = props.body ?? {};
  const senderHint = {
    path: body.sender_path || props.sender_path || entityPathFromOutboxPath(nodePath),
    id: props.sender_id || body.sender_id,
    email: body.sender_email,
  };
  const recipientHint = {
    path: body.recipient_path || props.recipient_path,
    id: props.recipient_id || body.recipient_id,
    email: body.recipient_email,
  };

  // 3. Resolve sender: agent first (if ai workspace or agent: prefix), then user
  const sender = await resolveSender(triggerWorkspace, senderHint);
  if (!sender) throw new Error('Sender not found');

  // 4. Resolve recipient: user first, then agent
  const recipient = await resolveRecipient(recipientHint);
  if (!recipient) throw new Error('Recipient not found');

  // 5. Check permissions
  const perm = await checkPermission(sender.id, recipient.id, messageType);
  if (!perm.allowed) throw new Error(perm.reason ?? 'Not allowed to message this user');

  // 6. Build conversation context
  const conversationId = body.thread_id || props.conversation_id || ('conv-' + await raisin.crypto.uuid());
  const streamChannel = `chat:${conversationId}`;
  const subject = props.subject || body.subject || 'Chat';
  const isIntermediate = AI_INTERMEDIATE_TYPES.has(messageType);
  const participants = [sender.id, recipient.id].sort();
  const participantDetails = {
    [sender.id]: { display_name: sender.displayName },
    [recipient.id]: { display_name: recipient.displayName },
  };
  const agentMeta = buildAgentMetadata(sender, recipient);

  // 7. Ensure inbox structures exist
  const senderChats = await ensureChatsFolder(sender.workspace, sender.path);
  const recipientChats = await ensureChatsFolder(recipient.workspace, recipient.path);

  const senderConvPath = `${senderChats}/${conversationId}`;
  const recipientConvPath = `${recipientChats}/${conversationId}`;

  // 8. Upsert conversations
  const convBase = { conversationId, subject, participants, participantDetails, streamChannel, agentMeta };

  await upsertConversation(sender.workspace, senderConvPath, convBase, body, sender.id, recipient.id,
    /* incrementUnread */ false, /* updateLastMessage */ !isIntermediate);

  await upsertConversation(recipient.workspace, recipientConvPath, convBase, body, sender.id, recipient.id,
    /* incrementUnread */ !isIntermediate, /* updateLastMessage */ !isIntermediate);

  // 9. Create message copies in both conversations
  const messageSlug = `msg-${messageId}`;
  const createdAt = props.created_at || new Date().toISOString();
  const messageText = body.message_text || body.content || '';

  const baseMessageProps = buildMessageProps(props, body, {
    messageType, senderId: sender.id, senderPath: sender.path,
    recipientId: recipient.id, recipientPath: recipient.path,
    conversationId, createdAt, messageText,
  });

  const senderMessagePath = `${senderConvPath}/${messageSlug}`;

  await createMessageCopyIdempotent(recipient.workspace, recipientConvPath, {
    slug: messageSlug,
    name: messageSlug,
    node_type: 'raisin:Message',
    properties: {
      ...baseMessageProps,
      status: 'delivered',
      delivered_at: new Date().toISOString(),
      sender_message_path: senderMessagePath,
    },
  });

  await createMessageCopyIdempotent(sender.workspace, senderConvPath, {
    slug: messageSlug,
    name: messageSlug,
    node_type: 'raisin:Message',
    properties: { ...baseMessageProps, status: 'sent' },
  });

  // 10. Create notification for human recipients (not agents, not AI intermediate)
  if (!recipient.isAgent && !isIntermediate) {
    await createNotification(recipient, conversationId, sender.displayName, messageText, messageId);
  }

  // 11. Mark original outbox message as delivered
  await raisin.nodes.updateProperty(triggerWorkspace, nodePath, 'status', 'delivered');
  await raisin.nodes.updateProperty(triggerWorkspace, nodePath, 'delivered_at', new Date().toISOString());

  return { success: true, conversation_path: recipientConvPath };
}

// ─── Entity Resolution ──────────────────────────────────────────────────────

function isAgentId(id) {
  return typeof id === 'string' && id.startsWith('agent:');
}

function extractAgentName(id, path) {
  if (id && isAgentId(id)) return id.slice('agent:'.length);
  if (path) {
    const parts = path.split('/');
    if (parts.length >= 3 && parts[1] === 'agents') return parts[2];
  }
  return null;
}

function entityPathFromOutboxPath(nodePath) {
  const parts = nodePath.split('/');
  if (parts.length > 2 && (parts[1] === 'users' || parts[1] === 'agents')) {
    return `/${parts[1]}/${parts[2]}`;
  }
  return null;
}

async function resolveSender(triggerWorkspace, hint) {
  // Try agent resolution first if workspace is ai or sender_id is agent:
  if (triggerWorkspace === 'ai' || isAgentId(hint.id)) {
    const agent = await resolveAgent(hint.path, hint.id);
    if (agent) return agent;
  }
  // Fall back to user in raisin:access_control
  const user = await resolveUser('raisin:access_control', hint.path, hint.id, hint.email);
  return user;
}

async function resolveRecipient(hint) {
  // Try user first
  const user = await resolveUser('raisin:access_control', hint.path, hint.id, hint.email);
  if (user) return user;
  // Fall back to agent
  return resolveAgent(hint.path, hint.id);
}

async function resolveAgent(agentPath, agentId) {
  const name = extractAgentName(agentId, agentPath);
  if (!name) return null;

  // Verify agent definition exists in functions workspace
  const defPath = `/agents/${name}`;
  const agentDef = await raisin.nodes.get('functions', defPath);
  if (!agentDef) return null;

  const defProps = agentDef.properties ?? {};

  // Auto-provision agent home in ai workspace
  const homePath = `/agents/${name}`;
  let home = await raisin.nodes.get('ai', homePath);

  if (!home) {
    await ensureFolderExists('ai', '/', 'agents', 'Agents');
    const prettyName = defProps.title || name.replace(/-/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
    try {
      await raisin.nodes.create('ai', '/agents', {
        slug: name, name, node_type: 'raisin:Folder',
        properties: {
          title: prettyName,
          user_id: `agent:${name}`,
          display_name: prettyName,
          agent_ref: { 'raisin:ref': '', 'raisin:workspace': 'functions', 'raisin:path': defPath },
          max_turns: 10,
        },
      });
    } catch (e) {
      // Race condition: another worker created it
      if (!String(e?.message || '').includes('already exists')) throw e;
    }
    home = await raisin.nodes.get('ai', homePath);
  }

  if (!home) return null;
  const homeProps = home.properties ?? {};
  return {
    id: homeProps.user_id || `agent:${name}`,
    path: homePath,
    displayName: homeProps.display_name || name,
    workspace: 'ai',
    isAgent: true,
    properties: homeProps,
  };
}

async function resolveUser(workspace, userPath, userId, email) {
  if (isAgentId(userId)) return null;

  // Handle user_id that's actually a path
  if (userId && typeof userId === 'string' && userId.startsWith('/users/')) {
    userPath = userId;
    userId = null;
  }

  // By user_id property
  if (userId) {
    const rows = await sqlRows(`
      SELECT id, path, name, node_type, properties
      FROM '${workspace}'
      WHERE node_type = 'raisin:User' AND properties->>'user_id'::String = $1
      LIMIT 1
    `, [userId]);
    if (rows.length > 0) return userNodeToEntity(rows[0], workspace);
  }

  // By path
  if (userPath) {
    const node = await raisin.nodes.get(workspace, userPath);
    if (node) return userNodeToEntity(node, workspace);
  }

  // By node ID
  if (userId) {
    const node = await raisin.nodes.getById(workspace, userId);
    if (node) return userNodeToEntity(node, workspace);
  }

  // By email
  if (email) {
    const rows = await sqlRows(`
      SELECT id, path, name, node_type, properties
      FROM '${workspace}'
      WHERE node_type = 'raisin:User' AND properties->>'email'::String = $1
      LIMIT 1
    `, [email]);
    if (rows.length > 0) return userNodeToEntity(rows[0], workspace);
  }

  return null;
}

function userNodeToEntity(node, workspace) {
  const p = node.properties ?? {};
  return {
    id: node.id,
    path: node.path,
    displayName: p.display_name || node.name || node.id,
    workspace,
    isAgent: false,
    properties: p,
  };
}

// ─── Folder / Inbox Provisioning ────────────────────────────────────────────

async function ensureFolderExists(workspace, parentPath, slug, title) {
  const fullPath = `${parentPath === '/' ? '' : parentPath}/${slug}`;
  const existing = await raisin.nodes.get(workspace, fullPath);
  if (existing) return fullPath;
  try {
    await raisin.nodes.create(workspace, parentPath, {
      slug, name: slug, node_type: 'raisin:Folder', properties: { title },
    });
  } catch (e) {
    if (!String(e?.message || '').includes('already exists')) throw e;
  }
  return fullPath;
}

async function ensureChatsFolder(workspace, entityPath) {
  const inboxPath = await ensureFolderExists(workspace, entityPath, 'inbox', 'Inbox');
  return ensureFolderExists(workspace, inboxPath, 'chats', 'Chats');
}

async function createMessageCopyIdempotent(workspace, parentPath, nodeData) {
  try {
    await raisin.nodes.create(workspace, parentPath, nodeData);
  } catch (e) {
    if (!String(e?.message || '').includes('already exists')) {
      throw e;
    }
  }
}

// ─── Conversation Upsert ────────────────────────────────────────────────────

async function upsertConversation(workspace, convPath, ctx, body, senderId, recipientId, incrementUnread, updateLastMessage) {
  const now = new Date().toISOString();
  const messageText = body.message_text || body.content || '';
  const senderDetail = ctx.participantDetails[senderId] ?? {};
  const recipientDetail = ctx.participantDetails[recipientId] ?? {};

  const lastMessage = {
    content: messageText,
    sender_id: senderId,
    sender_display_name: senderDetail.display_name,
    recipient_id: recipientId,
    recipient_display_name: recipientDetail.display_name,
    created_at: now,
  };

  const existing = await raisin.nodes.get(workspace, convPath);

  if (!existing) {
    const props = {
      subject: ctx.subject,
      conversation_id: ctx.conversationId,
      participants: ctx.participants,
      participant_details: ctx.participantDetails,
      stream_channel: ctx.streamChannel,
      unread_count: incrementUnread ? 1 : 0,
      updated_at: now,
      ...ctx.agentMeta,
    };
    if (updateLastMessage) props.last_message = lastMessage;

    const parentPath = convPath.split('/').slice(0, -1).join('/');
    try {
      await raisin.nodes.create(workspace, parentPath, {
        slug: ctx.conversationId, name: ctx.conversationId,
        node_type: 'raisin:Conversation', properties: props,
      });
    } catch (e) {
      if (!String(e?.message || '').includes('already exists')) throw e;
      // Created by another worker — fall through to update path
      return upsertConversation(workspace, convPath, ctx, body, senderId, recipientId, incrementUnread, updateLastMessage);
    }
    return;
  }

  // Update existing conversation
  const existingProps = existing.properties ?? {};
  const unreadCount = incrementUnread ? (existingProps.unread_count ?? 0) + 1 : (existingProps.unread_count ?? 0);

  const updates = {
    conversation_id: ctx.conversationId,
    participants: ctx.participants,
    participant_details: ctx.participantDetails,
    stream_channel: ctx.streamChannel,
    unread_count: unreadCount,
    updated_at: now,
  };
  if (updateLastMessage) {
    updates.subject = ctx.subject;
    updates.last_message = lastMessage;
  }
  if (ctx.agentMeta) Object.assign(updates, ctx.agentMeta);

  for (const [key, value] of Object.entries(updates)) {
    await raisin.nodes.updateProperty(workspace, convPath, key, value);
  }
}

// ─── Message Building ───────────────────────────────────────────────────────

function buildMessageProps(originalProps, body, ctx) {
  const base = { ...originalProps };

  // Strip fields that shouldn't propagate to inbox copies
  for (const key of ['sender_email', 'recipient_email', 'sender_display_name', 'recipient_display_name', 'participant_paths']) {
    delete base[key];
  }

  base.title = 'Chat message';
  base.body = { content: ctx.messageText, message_text: ctx.messageText };
  base.message_type = ctx.messageType;
  base.sender_id = ctx.senderId;
  base.sender_path = ctx.senderPath;
  base.recipient_id = ctx.recipientId;
  base.recipient_path = ctx.recipientPath;
  base.conversation_id = ctx.conversationId;
  base.created_at = ctx.createdAt;

  // Merge body + original data for the data property
  const originalData = originalProps.data ?? {};
  const merged = { ...body };
  delete merged.sender_email;
  delete merged.recipient_email;
  if (typeof originalData === 'object' && originalData !== null) {
    Object.assign(merged, originalData);
  }
  base.data = merged;

  return base;
}

// ─── Agent Metadata ─────────────────────────────────────────────────────────

function buildAgentMetadata(sender, recipient) {
  let agentName = null;
  let humanSenderId = null;
  let humanSenderPath = null;

  if (sender.isAgent) {
    agentName = extractAgentName(sender.id, sender.path);
    humanSenderId = recipient.id;
    humanSenderPath = recipient.path;
  } else if (recipient.isAgent) {
    agentName = extractAgentName(recipient.id, recipient.path);
    humanSenderId = sender.id;
    humanSenderPath = sender.path;
  }

  if (!agentName) return {};

  const meta = {
    agent_ref: { 'raisin:ref': '', 'raisin:workspace': 'functions', 'raisin:path': `/agents/${agentName}` },
  };
  if (humanSenderId) meta.human_sender_id = humanSenderId;
  if (humanSenderPath) meta.human_sender_path = humanSenderPath;
  return meta;
}

// ─── Sent Folder ────────────────────────────────────────────────────────────

async function copyToSentFolder(workspace, node) {
  const nodePath = node?.path ?? '';
  const parts = nodePath.split('/');

  // Path: /{users|agents}/{name}/outbox/{slug}
  if (parts.length < 5 || parts[3] !== 'outbox') return;
  const entityType = parts[1];
  if (entityType !== 'users' && entityType !== 'agents') return;

  const entityName = parts[2];
  const messageSlug = parts[4];
  const sentPath = `/${entityType}/${entityName}/sent`;

  const sentProps = { ...(node.properties ?? {}) };
  sentProps.status = 'sent';
  sentProps.sent_at = new Date().toISOString();

  try {
    await raisin.nodes.create(workspace, sentPath, {
      slug: messageSlug, name: messageSlug,
      node_type: node?.node_type ?? 'raisin:Message',
      properties: sentProps,
    });
  } catch (e) {
    if (!String(e?.message || '').includes('already exists')) throw e;
  }

  await raisin.nodes.updateProperty(workspace, nodePath, 'status', 'sent');
  await raisin.nodes.updateProperty(workspace, nodePath, 'sent_at', new Date().toISOString());
}

// ─── Notifications ──────────────────────────────────────────────────────────

async function createNotification(recipient, conversationId, senderName, messageText, messageId) {
  const notificationsPath = await ensureFolderExists(
    recipient.workspace, `${recipient.path}/inbox`, 'notifications', 'Notifications'
  );
  const slug = `notif-chat-${String(messageId).slice(-8)}`;
  const now = new Date().toISOString();

  try {
    await raisin.nodes.create(recipient.workspace, notificationsPath, {
      slug, name: slug, node_type: 'raisin:Notification',
      properties: {
        type: 'message',
        title: `New message from ${senderName}`,
        body: messageText.substring(0, 80),
        link: `/inbox/chats/${conversationId}`,
        read: false,
        data: { conversation_id: conversationId, sender_id: senderName, created_at: now },
      },
    });
  } catch (e) {
    // Notification creation is best-effort
    console.warn('[handle-chat] Notification creation failed:', e.message);
  }
}

// ─── Permissions ────────────────────────────────────────────────────────────

async function checkPermission(senderId, recipientId, messageType) {
  if (!senderId || !recipientId) {
    return { allowed: false, reason: 'Missing sender_id or recipient_id' };
  }

  // Auto-allow agent messaging
  if (isAgentId(senderId) || isAgentId(recipientId)) {
    return { allowed: true };
  }

  // Auto-allow AI intermediate types
  if (AI_INTERMEDIATE_TYPES.has(messageType)) {
    return { allowed: true };
  }

  const config = await getMessagingConfig();

  if (!(config.enabled ?? true)) {
    return { allowed: false, reason: 'Messaging is disabled' };
  }

  if (config.blocked_users_prevent_messaging ?? true) {
    if (await isBlocked(recipientId, senderId)) {
      return { allowed: false, reason: 'You have been blocked by this user' };
    }
  }

  let permissions;
  if (messageType === 'chat' || messageType === 'direct_message') {
    permissions = config.chat_permissions ?? { mode: 'any_of', rules: [{ type: 'always' }] };
  } else if (messageType === 'task_assignment') {
    permissions = config.task_permissions ?? { mode: 'any_of', rules: [{ type: 'always' }] };
  } else {
    permissions = config.chat_permissions ?? { mode: 'any_of', rules: [{ type: 'always' }] };
  }

  return evaluatePermissions(senderId, recipientId, permissions);
}

async function getMessagingConfig() {
  const node = await raisin.nodes.get('raisin:access_control', '/config/messaging');
  if (node?.properties) return node.properties;
  return {
    enabled: true,
    chat_permissions: { mode: 'any_of', rules: [{ type: 'always' }] },
    task_permissions: { mode: 'any_of', rules: [{ type: 'always' }] },
    blocked_users_prevent_messaging: true,
  };
}

async function isBlocked(userId, blockedUserId) {
  const blocker = idCondition('blocker', userId);
  const blockee = idCondition('blockee', blockedUserId);
  const sql = `SELECT * FROM GRAPH_TABLE(MATCH (blocker)-[:BLOCKS]->(blockee) WHERE ${blocker} AND ${blockee} COLUMNS (blockee.id AS id)) AS g LIMIT 1`;
  const rows = await sqlRows(sql, []);
  return rows.length > 0;
}

async function evaluatePermissions(senderId, recipientId, permissions) {
  const mode = permissions.mode ?? 'any_of';
  const rules = permissions.rules ?? [];
  if (!rules.length) return { allowed: true };

  for (const rule of rules) {
    const matched = await evaluateRule(senderId, recipientId, rule);
    if (mode === 'any_of' && matched) return { allowed: true };
    if (mode === 'all_of' && !matched) return { allowed: false, reason: `Permission rule not satisfied: ${rule.type}` };
  }

  return mode === 'any_of'
    ? { allowed: false, reason: 'No permission rule satisfied' }
    : { allowed: true };
}

async function evaluateRule(senderId, recipientId, rule) {
  switch (rule.type) {
    case 'always': return true;
    case 'never': return false;
    case 'relationship': return rule.relation ? hasRelationship(senderId, recipientId, rule.relation) : false;
    case 'same_group': return inSameGroup(senderId, recipientId, rule.group_type);
    case 'same_role': return hasSameRole(senderId, recipientId, rule.role);
    case 'sender_has_role': return userHasRole(senderId, rule.role);
    case 'recipient_has_role': return userHasRole(recipientId, rule.role);
    default: return false;
  }
}

async function hasRelationship(userA, userB, relationType) {
  const label = relationType.toUpperCase().replace(/-/g, '_');
  const aCond = idCondition('a', userA);
  const bCond = idCondition('b', userB);

  // Check A→B
  let rows = await sqlRows(`SELECT * FROM GRAPH_TABLE(MATCH (a)-[:${label}]->(b) WHERE ${aCond} AND ${bCond} COLUMNS (b.id AS id)) AS g LIMIT 1`, []);
  if (rows.length > 0) return true;

  // Check B→A (bidirectional)
  const aCondRev = idCondition('a', userB);
  const bCondRev = idCondition('b', userA);
  rows = await sqlRows(`SELECT * FROM GRAPH_TABLE(MATCH (a)-[:${label}]->(b) WHERE ${aCondRev} AND ${bCondRev} COLUMNS (b.id AS id)) AS g LIMIT 1`, []);
  return rows.length > 0;
}

async function inSameGroup(userAId, userBId, groupType) {
  const userA = await getUserNode(userAId);
  const userB = await getUserNode(userBId);
  if (!userA || !userB) return false;

  let groupsA = Array.isArray(userA.properties?.groups) ? userA.properties.groups : [];
  let groupsB = Array.isArray(userB.properties?.groups) ? userB.properties.groups : [];
  if (!groupsA.length || !groupsB.length) return false;

  if (groupType) {
    const filter = groupType.toLowerCase();
    groupsA = groupsA.filter(g => g.toLowerCase().includes(filter));
    groupsB = groupsB.filter(g => g.toLowerCase().includes(filter));
  }

  return groupsA.some(g => groupsB.includes(g));
}

async function hasSameRole(userAId, userBId, role) {
  if (role) return (await userHasRole(userAId, role)) && (await userHasRole(userBId, role));
  const a = await getUserNode(userAId);
  const b = await getUserNode(userBId);
  if (!a || !b) return false;
  const rolesA = Array.isArray(a.properties?.roles) ? a.properties.roles : [];
  const rolesB = Array.isArray(b.properties?.roles) ? b.properties.roles : [];
  return rolesA.some(r => rolesB.includes(r));
}

async function userHasRole(userId, role) {
  if (!role) return false;
  const user = await getUserNode(userId);
  if (!user) return false;
  const roles = Array.isArray(user.properties?.roles) ? user.properties.roles : [];
  return roles.includes(role);
}

async function getUserNode(userId) {
  if (!userId) return null;
  if (typeof userId === 'string' && userId.startsWith('/')) {
    return raisin.nodes.get('raisin:access_control', userId);
  }
  return raisin.nodes.getById('raisin:access_control', userId);
}

// ─── Helpers ────────────────────────────────────────────────────────────────

function idCondition(alias, entityId) {
  const isPath = typeof entityId === 'string' && entityId.startsWith('/');
  return isPath ? `${alias}.path = '${entityId}'` : `${alias}.id = '${entityId}'`;
}

async function sqlRows(sql, params) {
  const result = await raisin.sql.query(sql, params);
  return Array.isArray(result) ? result : (result?.rows ?? []);
}
