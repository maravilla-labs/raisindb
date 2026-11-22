/**
 * Handle Chat Message — Core messaging delivery handler
 *
 * Delivers outbox messages to recipient inboxes with cross-workspace support.
 * Users live in raisin:access_control, agents live in ai workspace.
 *
 * Flow (one-to-one):
 *   1. Validate message type
 *   2. Take the sender from the outbox path; refuse sender claims naming anybody else
 *   3. Copy to sender's sent folder
 *   4. Resolve the recipient (user, agent, visitor)
 *   5. Check permissions, length, rate and attachments
 *   6. Upsert conversation in both sender and recipient inboxes
 *   7. Create message copies in both conversations
 *   8. Create notification for human recipients (skip for agents + AI intermediates)
 *   9. Mark original outbox message as delivered
 *
 * GROUPS: a message addressed to a circle (`recipient_id: circle:<id>`, or a group conversation id
 * `grp-<id>`) is delivered to EVERY member and moderator of the circle, each into their own
 * `<home>/inbox/chats/grp-<id>` (deliverToCircle). The roster is the graph's (MEMBER_OF / MODERATES
 * edges only admins and the system may write), never anything the sender wrote. See
 * docs/plans/group-messaging-and-tasks.md.
 *
 * A refused message is marked `status: failed` with a `failure_reason` on the outbox node, so the
 * sender's screen can say why (bad_outbox_path, sender_mismatch, not_allowed, too_long, rate_limited,
 * bad_attachments, too_many_attachments, foreign_attachment, not_a_member, read_only, group_closed,
 * group_too_large, unknown_circle).
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

import {
  entityPathFromOutboxPath, resolveSenderHome, claimsMatch, refuse, resolveRecipient,
  readableBy, isUserKey, userKeyFromHome, extractAgentName, ensureFolderExists, ensureChatsFolder,
  createIdempotent,
} from '../shared/parties.js';
import {
  AI_INTERMEDIATE_TYPES, checkPermission, getMessagingConfig, lengthViolation, rateViolation,
  checkAttachments, conversationBetween, registerConversation,
} from '../shared/rules.js';
import { circleReference, resolveCircle, rosterFor, mayAct, conversationIdOf } from '../shared/circles.js';

export { entityPathFromOutboxPath, isUserKey, userKeyFromHome, readableBy };

const ALLOWED_MESSAGE_TYPES = new Set([
  'chat', 'direct_message',
  'ai_thought', 'ai_tool_call', 'ai_tool_result', 'ai_plan', 'ai_task_update'
]);

const TAG = 'handle-chat';

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
  if (!outboxHome) return refuse(triggerWorkspace, nodePath, 'bad_outbox_path', TAG);
  const sender = await resolveSenderHome(triggerWorkspace, outboxHome);
  if (!sender) throw new Error('Sender not found');
  // Sender fields written into the message must name that same party.
  if (!claimsMatch(sender, outboxHome, {
    ids: [props.sender_id, body.sender_id],
    paths: [props.sender_path, body.sender_path],
    emails: [props.sender_email, body.sender_email],
    keys: [props.sender_key, body.sender_key],
  })) {
    return refuse(triggerWorkspace, nodePath, 'sender_mismatch', TAG);
  }

  const isIntermediate = AI_INTERMEDIATE_TYPES.has(messageType);
  const messageText = body.message_text || body.content || '';

  // 2. Limits a person's message is held to (agents and AI steps are not).
  let config = null;
  if (!sender.isAgent && !sender.isVisitor && !isIntermediate) {
    config = await getMessagingConfig();
    const tooLong = lengthViolation(config, messageText);
    if (tooLong) return refuse(triggerWorkspace, nodePath, tooLong, TAG);
    const tooFast = await rateViolation(config, outboxHome);
    if (tooFast) return refuse(triggerWorkspace, nodePath, tooFast, TAG);
  }

  // 3. A message to a circle goes to everyone in it.
  const circleRef = circleReference(props, body);
  if (circleRef) {
    return deliverToCircle({ triggerWorkspace, node, props, body, sender, circleRef, messageText, messageType });
  }

  // 4. Copy to sender's sent folder
  await copyToSentFolder(triggerWorkspace, node);

  // 5. The recipient is the sender's choice, and must exist.
  const recipientHint = {
    path: body.recipient_path || props.recipient_path,
    id: props.recipient_id || body.recipient_id,
    email: body.recipient_email,
    key: props.recipient_key || body.recipient_key,
  };
  const recipient = await resolveRecipient(recipientHint);
  if (!recipient) throw new Error('Recipient not found');

  // 6. Check permissions
  const perm = sender.isVisitor || recipient.isVisitor
    ? checkVisitorPermission(sender, recipient)
    : await checkPermission(sender.id, recipient.id, messageType, config);
  if (!perm.allowed) throw new Error(perm.reason ?? 'Not allowed to message this user');

  // 7. Build conversation context
  // The thread is the sender's choice: only one already between these two (never a group's), and files only
  // from a conversation the sender already has.
  const thread = await conversationBetween(sender, recipient, body.thread_id || props.conversation_id);
  if (thread.error) return refuse(triggerWorkspace, node.path, thread.error, TAG);
  const conversationId = thread.id ?? ('conv-' + await raisin.crypto.uuid());
  const attachments = checkAttachments(body.attachments, thread.own ? conversationId : null);
  if (attachments.error) return refuse(triggerWorkspace, node.path, attachments.error, TAG);
  // The system's record of who the conversation is between, before either copy is written.
  if (!thread.own && !(await registerConversation(conversationId, sender.id, recipient.id))) {
    return refuse(triggerWorkspace, node.path, 'not_this_conversation', TAG);
  }

  const streamChannel = `chat:${conversationId}`;
  const subject = props.subject || body.subject || 'Chat';
  const participants = [sender.id, recipient.id].sort();
  const participantKeys = participants.map((id) => (id === sender.id ? sender.key : recipient.key) ?? null);
  const participantDetails = {
    [sender.id]: { display_name: sender.displayName },
    [recipient.id]: { display_name: recipient.displayName },
  };
  const agentMeta = buildAgentMetadata(sender, recipient);

  // 8. Ensure inbox structures exist
  const senderChats = await ensureChatsFolder(sender.workspace, sender.path);
  const recipientChats = await ensureChatsFolder(recipient.workspace, recipient.path);

  const senderConvPath = `${senderChats}/${conversationId}`;
  const recipientConvPath = `${recipientChats}/${conversationId}`;

  // 9. Upsert conversations
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

  // 10. Create message copies in both conversations
  const messageSlug = `msg-${messageId}`;
  const createdAt = props.created_at || new Date().toISOString();

  const baseMessageProps = buildMessageProps(props, body, {
    messageType, senderId: sender.id, senderPath: sender.path, senderKey: sender.key ?? null,
    recipientId: recipient.id, recipientPath: recipient.path, recipientKey: recipient.key ?? null,
    conversationId, createdAt, messageText, attachments: attachments.list,
  });

  const senderMessagePath = `${senderConvPath}/${messageSlug}`;

  const deliveredProps = {
    ...baseMessageProps,
    status: 'delivered',
    delivered_at: new Date().toISOString(),
    sender_message_path: senderMessagePath,
  };
  await createIdempotent(recipient.workspace, recipientConvPath, {
    slug: messageSlug,
    name: messageSlug,
    node_type: 'raisin:Message',
    properties: readableBy(recipient, deliveredProps),
  });

  await createIdempotent(sender.workspace, senderConvPath, {
    slug: messageSlug,
    name: messageSlug,
    node_type: 'raisin:Message',
    properties: readableBy(sender, { ...baseMessageProps, status: 'sent' }),
  });

  // 11. Create notification for human recipients (not agents, not AI
  //     intermediates, not visitors: a visitor's page streams the reply)
  if (!recipient.isAgent && !recipient.isVisitor && !isIntermediate) {
    await createNotification(recipient, conversationId, sender.displayName, messageText, messageId);
  }

  // 12. Mark original outbox message as delivered
  await raisin.nodes.updateProperty(triggerWorkspace, nodePath, 'status', 'delivered');
  await raisin.nodes.updateProperty(triggerWorkspace, nodePath, 'delivered_at', new Date().toISOString());

  return { success: true, conversation_path: recipientConvPath };
}

// ─── Group delivery ─────────────────────────────────────────────────────────

/**
 * Deliver one message to every member and moderator of a circle, each into their own home.
 *
 * Each copy of the conversation (`<home>/inbox/chats/grp-<circle id>`) is `conversation_type: group`,
 * names the circle as its one participant (`circle:<id>`, so a client answering "the other party" posts
 * back to the circle), and carries the member count, unread count and last message. Message copies are
 * `sent` for the sender and `delivered` for everyone else, with a notification for everyone else.
 */
async function deliverToCircle({ triggerWorkspace, node, props, body, sender, circleRef, messageText, messageType }) {
  const nodePath = node.path;
  if (sender.isAgent || sender.isVisitor) return refuse(triggerWorkspace, nodePath, 'not_a_member', TAG);

  const circle = await resolveCircle(circleRef);
  if (!circle) return refuse(triggerWorkspace, nodePath, 'unknown_circle', TAG);
  if (circle.policy.chat === 'off') return refuse(triggerWorkspace, nodePath, 'group_closed', TAG);

  const roster = await rosterFor(circle, sender.id);
  if (!roster.role) return refuse(triggerWorkspace, nodePath, 'not_a_member', TAG);
  if (!mayAct(circle.policy, roster.role, 'chat')) return refuse(triggerWorkspace, nodePath, 'read_only', TAG);
  if (roster.everyone.length > circle.policy.max_members) {
    return refuse(triggerWorkspace, nodePath, 'group_too_large', TAG);
  }

  const conversationId = conversationIdOf(circle);
  const attachments = checkAttachments(body.attachments, conversationId);
  if (attachments.error) return refuse(triggerWorkspace, nodePath, attachments.error, TAG);

  await copyToSentFolder(triggerWorkspace, node);

  const now = new Date().toISOString();
  const createdAt = props.created_at || now;
  const messageSlug = `msg-${node.id}`;
  const circleParty = `circle:${circle.id}`;
  const lastMessage = {
    content: messageText,
    sender_id: sender.id,
    sender_key: sender.key ?? null,
    sender_display_name: sender.displayName,
    created_at: now,
  };
  const messageProps = {
    title: 'Chat message',
    message_type: messageType,
    role: 'user',
    body: { content: messageText, message_text: messageText },
    sender_id: sender.id,
    sender_key: sender.key ?? null,
    sender_display_name: sender.displayName,
    recipient_id: circleParty,
    conversation_id: conversationId,
    client_id: typeof props.client_id === 'string' ? props.client_id : undefined,
    created_at: createdAt,
    data: attachments.list.length ? { attachments: attachments.list } : {},
  };
  if (attachments.list.length) messageProps.attachments = attachments.list;
  if (messageProps.client_id === undefined) delete messageProps.client_id;

  let delivered = 0;
  for (const person of roster.everyone) {
    const own = person.id === sender.id;
    const chats = await ensureChatsFolder(person.workspace, person.path);
    const convPath = `${chats}/${conversationId}`;
    await upsertGroupConversation(person.workspace, convPath, {
      conversationId, circle, circleParty, memberCount: roster.memberCount, lastMessage, now, incrementUnread: !own,
    });
    await createIdempotent(person.workspace, convPath, {
      slug: messageSlug,
      name: messageSlug,
      node_type: 'raisin:Message',
      properties: own
        ? { ...messageProps, status: 'sent' }
        : { ...messageProps, status: 'delivered', delivered_at: now },
    });
    if (!own) {
      await createNotification(person, conversationId, sender.displayName, messageText, node.id, circle.name);
      delivered += 1;
    }
  }

  await raisin.nodes.updateProperty(triggerWorkspace, nodePath, 'status', 'delivered');
  await raisin.nodes.updateProperty(triggerWorkspace, nodePath, 'delivered_at', now);
  return { success: true, conversation_id: conversationId, delivered };
}

/** Create or refresh one member's copy of a group conversation (one read and one write). */
async function upsertGroupConversation(workspace, convPath, { conversationId, circle, circleParty, memberCount, lastMessage, now, incrementUnread }) {
  const shape = {
    conversation_type: 'group',
    conversation_id: conversationId,
    subject: circle.name,
    circle_id: circle.id,
    circle_type: circle.type,
    participants: [circleParty],
    member_count: memberCount,
    stream_channel: `chat:${conversationId}`,
    last_message: lastMessage,
    updated_at: now,
    membership: 'member',
  };
  const existing = await raisin.nodes.get(workspace, convPath);
  if (!existing) {
    const parentPath = convPath.split('/').slice(0, -1).join('/');
    const created = await createIdempotent(workspace, parentPath, {
      slug: conversationId, name: conversationId, node_type: 'raisin:Conversation',
      properties: { ...shape, unread_count: incrementUnread ? 1 : 0 },
    });
    if (created) return;
  }
  const current = (existing ?? await raisin.nodes.get(workspace, convPath))?.properties ?? {};
  const unread = (Number(current.unread_count) || 0) + (incrementUnread ? 1 : 0);
  await raisin.nodes.update(workspace, convPath, { properties: { ...current, ...shape, unread_count: unread } });
}

// ─── Visitors and agents ────────────────────────────────────────────────────

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

/** The conversation context as `reader`'s copy holds it (no human home path in a person's copy). */
function forReader(convBase, reader) {
  if (reader.isAgent || !convBase.agentMeta?.human_sender_path) return convBase;
  const { human_sender_path: _dropped, ...agentMeta } = convBase.agentMeta;
  return { ...convBase, agentMeta };
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
    'sender_key', 'recipient_key', 'participant_keys', 'attachments']) {
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

  // Merge body + original data for the data property. Attachments are only the
  // checked list, never what the sender wrote.
  const originalData = originalProps.data ?? {};
  const merged = { ...body };
  delete merged.sender_email;
  delete merged.recipient_email;
  delete merged.sender_key;
  delete merged.recipient_key;
  delete merged.participant_keys;
  delete merged.attachments;
  if (typeof originalData === 'object' && originalData !== null) {
    Object.assign(merged, originalData);
    delete merged.attachments;
  }
  if (ctx.attachments?.length) {
    merged.attachments = ctx.attachments;
    base.attachments = ctx.attachments;
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

  await createIdempotent(workspace, sentPath, {
    slug: messageSlug, name: messageSlug,
    node_type: node?.node_type ?? 'raisin:Message',
    properties: sentProps,
  });

  await raisin.nodes.updateProperty(workspace, nodePath, 'status', 'sent');
  await raisin.nodes.updateProperty(workspace, nodePath, 'sent_at', new Date().toISOString());
}

// ─── Notifications ──────────────────────────────────────────────────────────

async function createNotification(recipient, conversationId, senderName, messageText, messageId, groupName = null) {
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
        title: groupName ? `${senderName} in ${groupName}` : `New message from ${senderName}`,
        body: String(messageText).substring(0, 80),
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
