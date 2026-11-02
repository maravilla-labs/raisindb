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
 *
 * Stable person keys: a user's home is `/users/internal/<user_key>`, where
 * user_key is `u_` + 22 base62 characters minted by the engine (homes created
 * before keys keep an email-slug path and have no key). Messages carry
 * `sender_key` / `recipient_key` and conversations `participant_keys`
 * (aligned with `participants`), always taken from the RESOLVED homes, null
 * for a party without a key (agents, visitors, legacy homes); a key the client
 * wrote into the outbox node is dropped. A recipient may be named by its key
 * (`recipient_key`, or a key in `recipient_id`). The copies a PERSON reads
 * (both conversation copies and both message copies in user homes) carry no
 * home path: ids and keys only. Agent-side copies keep the paths the agent
 * runtime works with.
 *
 * Visitors (anonymous chat sessions, /visitors/<key> in raisin:access_control)
 * are a third kind of party besides users and agents. Their outbox is written
 * only by the server (the WebSocket transport, after the agent's anonymous
 * checks), and a visitor may talk to exactly one kind of recipient: an agent
 * that allows anonymous chat. Agents answer them like anyone else.
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

  const body = props.body ?? {};

  // 1. The sender is WHERE the message lives: the home whose outbox holds it.
  // Row-level security lets a person create nodes only in their own outbox,
  // but the node's content is theirs to write, so nothing in it may choose
  // the sender. Anything that is not an exact outbox shape is refused.
  const outboxHome = entityPathFromOutboxPath(nodePath);
  if (!outboxHome) return refuse(triggerWorkspace, nodePath, 'bad_outbox_path');
  const sender = await resolveSenderHome(triggerWorkspace, outboxHome);
  if (!sender) throw new Error('Sender not found');
  // Sender fields written into the message must name that same party.
  if (!claimsMatch(sender, outboxHome, {
    ids: [props.sender_id, body.sender_id],
    paths: [props.sender_path, body.sender_path],
    emails: [props.sender_email, body.sender_email],
    keys: [props.sender_key, body.sender_key],
  })) {
    return refuse(triggerWorkspace, nodePath, 'sender_mismatch');
  }

  // 2. Copy to sender's sent folder
  await copyToSentFolder(triggerWorkspace, node);

  // 3. The recipient is the sender's choice, and must exist.
  const recipientHint = {
    path: body.recipient_path || props.recipient_path,
    id: props.recipient_id || body.recipient_id,
    email: body.recipient_email,
    key: props.recipient_key || body.recipient_key,
  };

  // 4. Resolve recipient: user first, then agent
  const recipient = await resolveRecipient(recipientHint);
  if (!recipient) throw new Error('Recipient not found');

  // 5. Check permissions
  const perm = sender.isVisitor || recipient.isVisitor
    ? checkVisitorPermission(sender, recipient)
    : await checkPermission(sender.id, recipient.id, messageType);
  if (!perm.allowed) throw new Error(perm.reason ?? 'Not allowed to message this user');

  // 6. Build conversation context
  const conversationId = body.thread_id || props.conversation_id || ('conv-' + await raisin.crypto.uuid());
  const streamChannel = `chat:${conversationId}`;
  const subject = props.subject || body.subject || 'Chat';
  const isIntermediate = AI_INTERMEDIATE_TYPES.has(messageType);
  const participants = [sender.id, recipient.id].sort();
  const participantKeys = participants.map((id) => (id === sender.id ? sender.key : recipient.key) ?? null);
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
  const convBase = {
    conversationId, subject, participants, participantDetails, participantKeys, streamChannel, agentMeta,
    keys: { [sender.id]: sender.key ?? null, [recipient.id]: recipient.key ?? null },
  };

  // A conversation belongs to the two parties that started it. Before this
  // check, a second person who reused a thread id (`body.thread_id`) with the
  // same agent was merged into the agent's copy of the conversation: its
  // human_sender_* was overwritten, and the agent's replies went to whoever
  // wrote last. That is a cross-conversation mix-up made structural, so it is
  // refused here for everyone, not only for visitors.
  await assertSameHuman(sender, recipient, `${(sender.isAgent ? senderChats : recipientChats)}/${conversationId}`);

  await upsertConversation(sender.workspace, senderConvPath, forReader(convBase, sender), body, sender.id, recipient.id,
    /* incrementUnread */ false, /* updateLastMessage */ !isIntermediate);

  await upsertConversation(recipient.workspace, recipientConvPath, forReader(convBase, recipient), body, sender.id, recipient.id,
    /* incrementUnread */ !isIntermediate, /* updateLastMessage */ !isIntermediate);

  // 9. Create message copies in both conversations
  const messageSlug = `msg-${messageId}`;
  const createdAt = props.created_at || new Date().toISOString();
  const messageText = body.message_text || body.content || '';

  const baseMessageProps = buildMessageProps(props, body, {
    messageType, senderId: sender.id, senderPath: sender.path, senderKey: sender.key ?? null,
    recipientId: recipient.id, recipientPath: recipient.path, recipientKey: recipient.key ?? null,
    conversationId, createdAt, messageText,
  });

  const senderMessagePath = `${senderConvPath}/${messageSlug}`;

  const deliveredProps = {
    ...baseMessageProps,
    status: 'delivered',
    delivered_at: new Date().toISOString(),
    sender_message_path: senderMessagePath,
  };
  await createMessageCopyIdempotent(recipient.workspace, recipientConvPath, {
    slug: messageSlug,
    name: messageSlug,
    node_type: 'raisin:Message',
    properties: readableBy(recipient, deliveredProps),
  });

  await createMessageCopyIdempotent(sender.workspace, senderConvPath, {
    slug: messageSlug,
    name: messageSlug,
    node_type: 'raisin:Message',
    properties: readableBy(sender, { ...baseMessageProps, status: 'sent' }),
  });

  // 10. Create notification for human recipients (not agents, not AI
  //     intermediates, not visitors: a visitor's page streams the reply)
  if (!recipient.isAgent && !recipient.isVisitor && !isIntermediate) {
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


/** A user key: `u_` + 22 base62 (raisin_models::auth::user_key). */
const USER_KEY = /^u_[0-9A-Za-z]{22}$/;
const USER_HOMES_ROOT = '/users/internal';

export function isUserKey(value) {
  return typeof value === 'string' && USER_KEY.test(value);
}

/** The key of a keyed home (`/users/internal/u_…`), else null. */
export function userKeyFromHome(path) {
  if (typeof path !== 'string') return null;
  const last = path.replace(/\/+$/, '').split('/').pop();
  return isUserKey(last) ? last : null;
}

/** Fields that name a home path: never in a copy a person reads. */
const PATH_FIELDS = ['sender_path', 'recipient_path', 'sender_message_path', 'participant_paths'];

/**
 * The message properties as `reader` may see them: a person's copy (a user
 * or visitor home) carries ids and keys, no home path; an agent's copy keeps
 * the paths the agent runtime works with.
 */
export function readableBy(reader, props) {
  if (reader.isAgent) return props;
  const out = { ...props };
  for (const field of PATH_FIELDS) delete out[field];
  if (out.data && typeof out.data === 'object') {
    out.data = { ...out.data };
    for (const field of PATH_FIELDS) delete out.data[field];
  }
  return out;
}

/** The conversation context as `reader`'s copy holds it (no human home path in a person's copy). */
function forReader(convBase, reader) {
  if (reader.isAgent || !convBase.agentMeta?.human_sender_path) return convBase;
  const { human_sender_path: _dropped, ...agentMeta } = convBase.agentMeta;
  return { ...convBase, agentMeta };
}

/**
 * The home whose outbox directly holds the message at `nodePath`, or null.
 * Only these exact shapes are outboxes (every segment non-empty, the message
 * a direct child of `outbox`):
 *
 *   /users/<slug>/outbox/<name>               -> /users/<slug>
 *   /users/internal/<key|slug>/outbox/<name>  -> /users/internal/<key|slug>
 *   /agents/<name>/outbox/<name>              -> /agents/<name>
 *   /visitors/<key>/outbox/<name>             -> /visitors/<key>
 *
 * The position decides, not a search for the word: a home literally named
 * `outbox` (`/users/outbox/outbox/m`) is that home, and a node nested deeper
 * under an outbox is no message.
 */
export function entityPathFromOutboxPath(nodePath) {
  if (typeof nodePath !== 'string' || !nodePath.startsWith('/')) return null;
  const parts = nodePath.split('/').slice(1);
  if (parts.some((p) => p === '' || p === '.' || p === '..')) return null;
  const [root] = parts;
  if (root === 'users' && parts.length === 5 && parts[1] === 'internal' && parts[3] === 'outbox') {
    return `/users/internal/${parts[2]}`;
  }
  if (parts.length === 4 && parts[2] === 'outbox' && ['users', 'agents', 'visitors'].includes(root)) {
    // /users/internal is the folder of homes, not a home.
    if (root === 'users' && parts[1] === 'internal') return null;
    return `/${root}/${parts[1]}`;
  }
  return null;
}

/** Resolve the party living at `home` (a path from entityPathFromOutboxPath). */
async function resolveSenderHome(triggerWorkspace, home) {
  const root = home.split('/')[1];
  if (root === 'visitors') {
    if (triggerWorkspace !== 'raisin:access_control') return null;
    return resolveVisitor(home, null);
  }
  if (root === 'agents') {
    if (triggerWorkspace !== 'ai') return null;
    return resolveAgent(home, null);
  }
  if (triggerWorkspace !== 'raisin:access_control') return null;
  const node = await raisin.nodes.get('raisin:access_control', home);
  if (!node || node.node_type !== 'raisin:User') return null;
  return userNodeToEntity(node, 'raisin:access_control');
}

/** Does every sender field the message carries name `sender` (at `home`)? */
function claimsMatch(sender, home, { ids, paths, emails, keys }) {
  const present = (list) => list.filter((v) => v !== undefined && v !== null && v !== '');
  const p = sender.properties ?? {};
  const idOk = new Set([sender.id, p.user_id, home].filter(Boolean));
  for (const v of present(ids)) if (typeof v !== 'string' || !idOk.has(v)) return false;
  for (const v of present(paths)) if (v !== home) return false;
  const email = typeof p.email === 'string' ? p.email.toLowerCase() : null;
  for (const v of present(emails)) if (typeof v !== 'string' || v.toLowerCase() !== email) return false;
  for (const v of present(keys)) if (v !== sender.key) return false;
  return true;
}

/** Mark the outbox message failed (not delivered, not retried) and stop. */
async function refuse(workspace, nodePath, reason) {
  console.warn(`[handle-chat] refused ${nodePath}: ${reason}`);
  if (nodePath) {
    await raisin.nodes.updateProperty(workspace, nodePath, 'status', 'failed');
    await raisin.nodes.updateProperty(workspace, nodePath, 'failure_reason', reason);
  }
  return { success: false, reason };
}

async function resolveRecipient(hint) {
  if (isVisitorId(hint.id) || isVisitorPath(hint.path)) {
    return resolveVisitor(hint.path, hint.id);
  }
  // Try user first
  const user = await resolveUser('raisin:access_control', hint.path, hint.id, hint.email, hint.key);
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
    definition: agentDef,
  };
}

async function resolveUser(workspace, userPath, userId, email, userKey) {
  if (isAgentId(userId)) return null;
  if (!isUserKey(userKey)) userKey = null;
  // A key given as the id names the keyed home.
  if (!userKey && isUserKey(userId)) {
    userKey = userId;
    userId = null;
  }
  if (typeof userId !== 'string' || !userId) userId = null;
  if (typeof userPath !== 'string' || !userPath) userPath = null;
  if (typeof email !== 'string' || !email) email = null;

  // Handle user_id that's actually a path (flat or nested home)
  if (userId && userId.startsWith('/users/')) {
    userPath = userId;
    userId = null;
  }
  // A path into a user's outbox names the home.
  if (userPath) userPath = entityPathFromOutboxPath(userPath) || userPath;

  // By key: the home is /users/internal/<key>.
  if (userKey) {
    const node = await raisin.nodes.get(workspace, `${USER_HOMES_ROOT}/${userKey}`);
    if (node && (!node.node_type || node.node_type === 'raisin:User')) return userNodeToEntity(node, workspace);
    return null;
  }

  // By user_id property
  if (userId) {
    const rows = await sqlRows(`
      SELECT id, path, name, node_type, properties
      FROM '${workspace}'
      WHERE node_type = 'raisin:User' AND properties->>'user_id'::String = $1
      LIMIT 2
    `, [userId]);
    // Two homes for one identity is ambiguous: no recipient rather than a guess
    // (the engine's resolver refuses it the same way).
    if (rows.length > 1) return null;
    if (rows.length === 1) return userNodeToEntity(rows[0], workspace);
  }

  // By path (only a raisin:User node is a user)
  if (userPath) {
    const node = await raisin.nodes.get(workspace, userPath);
    if (node && (!node.node_type || node.node_type === 'raisin:User')) return userNodeToEntity(node, workspace);
  }

  // By node ID (the runtime may throw on an id that is not a node id)
  if (userId && typeof raisin.nodes.getById === 'function') {
    let node = null;
    try {
      node = await raisin.nodes.getById(workspace, userId);
    } catch (_) {
      node = null;
    }
    if (node && (!node.node_type || node.node_type === 'raisin:User')) return userNodeToEntity(node, workspace);
  }

  // By email
  if (email) {
    const rows = await sqlRows(`
      SELECT id, path, name, node_type, properties
      FROM '${workspace}'
      WHERE node_type = 'raisin:User' AND properties->>'email'::String = $1
      LIMIT 2
    `, [email]);
    if (rows.length > 1) return null;
    if (rows.length === 1) return userNodeToEntity(rows[0], workspace);
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
    key: userKeyFromHome(node.path),
    isAgent: false,
    properties: p,
  };
}

// ─── Visitors (anonymous chat sessions) ─────────────────────────────────────

const VISITOR_ROOT = '/visitors';
const VISITOR_ID_PREFIX = 'visitor:';
const VISITOR_KEY = /^[0-9a-f]{16,128}$/;

function isVisitorId(id) {
  return typeof id === 'string' && id.startsWith(VISITOR_ID_PREFIX);
}

function isVisitorPath(path) {
  return typeof path === 'string' && path.startsWith(VISITOR_ROOT + '/');
}

/**
 * The visitor session named by a path and/or id. Both, when given, must name
 * the SAME session: a message that claims one visitor's id from another's
 * home is refused rather than delivered to either.
 */
async function resolveVisitor(path, id) {
  const fromId = isVisitorId(id) ? id.slice(VISITOR_ID_PREFIX.length) : null;
  // A visitor is named by its HOME, exactly `/visitors/<key>`: any longer or
  // stranger path is refused rather than trimmed to a prefix.
  const fromPath = isVisitorPath(path)
    ? (/^\/visitors\/([0-9a-f]{16,128})$/.exec(path) || [])[1] || ''
    : null;
  if (fromPath === '') return null;
  if (fromId && fromPath && fromId !== fromPath) return null;
  const key = fromId || fromPath;
  if (!key || !VISITOR_KEY.test(key)) return null;
  const homePath = `${VISITOR_ROOT}/${key}`;
  const home = await raisin.nodes.get('raisin:access_control', homePath);
  if (!home || home.node_type !== 'raisin:VisitorSession') return null;
  const props = home.properties ?? {};
  return {
    id: `${VISITOR_ID_PREFIX}${key}`,
    path: homePath,
    displayName: props.display_name || 'Visitor',
    workspace: 'raisin:access_control',
    isAgent: false,
    isVisitor: true,
    properties: props,
  };
}

/**
 * A visitor talks to agents that allow it, and agents answer visitors. Nothing
 * else: no visitor-to-user and no visitor-to-visitor messages.
 */
function checkVisitorPermission(sender, recipient) {
  if (sender.isVisitor && recipient.isVisitor) {
    return { allowed: false, reason: 'Visitors cannot message each other' };
  }
  const agent = sender.isVisitor ? recipient : sender;
  const visitor = sender.isVisitor ? sender : recipient;
  if (!agent.isAgent) {
    return { allowed: false, reason: 'Visitors can only chat with agents' };
  }
  if (!agentAllowsAnonymous(agent.definition)) {
    return { allowed: false, reason: 'This agent does not accept anonymous visitors' };
  }
  const bound = visitor.properties?.agent_path;
  if (bound && bound !== `/agents/${extractAgentName(agent.id, agent.path)}`) {
    return { allowed: false, reason: 'This visitor session belongs to another agent' };
  }
  return { allowed: true };
}

/** `allow_anonymous: true`, or `anonymous: { enabled: true }`, on the agent. */
function agentAllowsAnonymous(def) {
  const p = def?.properties ?? {};
  if (p.allow_anonymous === true) return true;
  return !!(p.anonymous && typeof p.anonymous === 'object' && p.anonymous.enabled === true);
}

/**
 * Refuse a message that would merge a second person into an existing
 * agent-side conversation (see the call site).
 */
async function assertSameHuman(sender, recipient, agentConvPath) {
  if (sender.isAgent === recipient.isAgent) return; // not an agent conversation
  const agent = sender.isAgent ? sender : recipient;
  const human = sender.isAgent ? recipient : sender;
  const existing = await raisin.nodes.get(agent.workspace, agentConvPath);
  const owner = existing?.properties?.human_sender_id;
  if (owner && owner !== human.id) {
    throw new Error('Conversation belongs to another participant');
  }
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
  if (ctx.keys) {
    lastMessage.sender_key = ctx.keys[senderId] ?? null;
    lastMessage.recipient_key = ctx.keys[recipientId] ?? null;
  }

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
    if (ctx.participantKeys) props.participant_keys = ctx.participantKeys;
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
  if (ctx.participantKeys) updates.participant_keys = ctx.participantKeys;
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
  // Keys are never taken from the outbox node: only from the resolved homes.
  for (const key of ['sender_email', 'recipient_email', 'sender_display_name', 'recipient_display_name', 'participant_paths',
    'sender_key', 'recipient_key', 'participant_keys']) {
    delete base[key];
  }

  base.title = 'Chat message';
  base.body = { content: ctx.messageText, message_text: ctx.messageText };
  base.message_type = ctx.messageType;
  base.sender_id = ctx.senderId;
  base.sender_path = ctx.senderPath;
  base.recipient_id = ctx.recipientId;
  base.recipient_path = ctx.recipientPath;
  base.sender_key = ctx.senderKey;
  base.recipient_key = ctx.recipientKey;
  base.conversation_id = ctx.conversationId;
  base.created_at = ctx.createdAt;

  // Merge body + original data for the data property
  const originalData = originalProps.data ?? {};
  const merged = { ...body };
  delete merged.sender_email;
  delete merged.recipient_email;
  delete merged.sender_key;
  delete merged.recipient_key;
  delete merged.participant_keys;
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

  // conversation_type lets clients filter (the SDK lists with
  // conversation_type = 'ai_chat'); without it, agent-initiated
  // conversations are invisible to typed list queries.
  if (!agentName) return { conversation_type: 'direct_message' };

  const meta = {
    conversation_type: 'ai_chat',
    agent_ref: { 'raisin:ref': '', 'raisin:workspace': 'functions', 'raisin:path': `/agents/${agentName}` },
  };
  if (humanSenderId) meta.human_sender_id = humanSenderId;
  if (humanSenderPath) meta.human_sender_path = humanSenderPath;
  const human = sender.isAgent ? recipient : sender;
  meta.human_sender_key = human.key ?? null;
  return meta;
}

// ─── Sent Folder ────────────────────────────────────────────────────────────

async function copyToSentFolder(workspace, node) {
  const nodePath = node?.path ?? '';

  // Path: <home>/outbox/{slug}, the home being /{users|agents}/{name} or a
  // nested one such as /users/internal/{slug}. Only direct outbox children.
  const home = entityPathFromOutboxPath(nodePath);
  if (!home) return;
  const entityType = home.split('/')[1];
  // Visitors keep no sent folder: their home is ephemeral, and the delivered
  // copy in the conversation is the record.
  if (entityType !== 'users' && entityType !== 'agents') return;

  const messageSlug = nodePath.split('/').pop();
  // The sent folder is not provisioned with the entity's home, so ensure it
  // before writing: a node created under a missing parent is stored but never
  // listed as a child of anything.
  const sentPath = await ensureFolderExists(workspace, home, 'sent', 'Sent');

  const sentProps = { ...(node.properties ?? {}) };
  // Keys come from resolved homes only (see buildMessageProps).
  delete sentProps.sender_key;
  delete sentProps.recipient_key;
  delete sentProps.participant_keys;
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
