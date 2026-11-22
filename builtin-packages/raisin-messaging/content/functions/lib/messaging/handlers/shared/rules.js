/**
 * Who may send what: the rules of `raisin:MessagingConfig` (`raisin:access_control:/config/messaging`).
 *
 * Applied to every person-to-person message and task:
 *
 *   - `blocked_users_prevent_messaging`: a BLOCKS edge from the recipient refuses the sender.
 *   - `chat_permissions` / `task_permissions`: { mode: any_of | all_of, rules: [...] }, rules:
 *       always · never
 *       relationship        { relation: 'FRIENDS_WITH' }   an edge between the two, either direction
 *       same_circle         { circle_type?: 'course' }     both are members of one circle
 *       same_group          { group_type? }                a shared entry of the users' `groups`
 *       same_role · sender_has_role · recipient_has_role   { role }
 *   - `max_message_length`, `rate_limit_messages_per_minute`: enforced on chat.
 *
 * Group messages and tasks are governed by the circle's own policy instead (circles.js).
 *
 * The `raisin` global is injected at runtime -- do NOT import it.
 */

import { sqlRows, idCondition, isAgentId, nodeIdOf, createIdempotent } from './parties.js';
import { circlesOf } from './circles.js';

const ACL = 'raisin:access_control';
const ALWAYS = { mode: 'any_of', rules: [{ type: 'always' }] };

export const AI_INTERMEDIATE_TYPES = new Set([
  'ai_thought', 'ai_tool_call', 'ai_tool_result', 'ai_plan', 'ai_task_update',
]);

export async function getMessagingConfig() {
  const node = await raisin.nodes.get('raisin:access_control', '/config/messaging');
  if (node?.properties) return node.properties;
  return {
    enabled: true,
    chat_permissions: ALWAYS,
    task_permissions: ALWAYS,
    blocked_users_prevent_messaging: true,
  };
}

/**
 * May `senderId` send a `messageType` to `recipientId`? Ids are node ids (or home paths).
 * Returns { allowed, reason? }.
 */
export async function checkPermission(senderId, recipientId, messageType, config = null) {
  if (!senderId || !recipientId) {
    return { allowed: false, reason: 'Missing sender_id or recipient_id' };
  }
  // Auto-allow agent messaging and AI intermediate types
  if (isAgentId(senderId) || isAgentId(recipientId)) return { allowed: true };
  if (AI_INTERMEDIATE_TYPES.has(messageType)) return { allowed: true };

  config = config ?? await getMessagingConfig();
  if (!(config.enabled ?? true)) return { allowed: false, reason: 'Messaging is disabled' };

  if (config.blocked_users_prevent_messaging ?? true) {
    if (await isBlocked(recipientId, senderId)) {
      return { allowed: false, reason: 'You have been blocked by this user' };
    }
  }

  let permissions;
  if (messageType === 'task_assignment') permissions = config.task_permissions ?? ALWAYS;
  else if (messageType === 'system_notification') permissions = config.notification_permissions ?? ALWAYS;
  else permissions = config.chat_permissions ?? ALWAYS;

  return evaluatePermissions(senderId, recipientId, permissions);
}

export async function isBlocked(userId, blockedUserId) {
  const blocker = idCondition('blocker', userId);
  const blockee = idCondition('blockee', blockedUserId);
  const sql = `SELECT * FROM GRAPH_TABLE(MATCH (blocker)-[:BLOCKS]->(blockee) WHERE ${blocker} AND ${blockee} COLUMNS (blockee.id AS id)) AS g LIMIT 1`;
  const rows = await sqlRows(sql, []);
  return rows.length > 0;
}

export async function evaluatePermissions(senderId, recipientId, permissions) {
  const mode = permissions?.mode ?? 'any_of';
  const rules = Array.isArray(permissions?.rules) ? permissions.rules : [];
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

export async function evaluateRule(senderId, recipientId, rule) {
  switch (rule?.type) {
    case 'always': return true;
    case 'never': return false;
    case 'relationship': return rule.relation ? hasRelationship(senderId, recipientId, rule.relation) : false;
    case 'same_circle': return inSameCircle(senderId, recipientId, rule.circle_type);
    case 'same_group': return inSameGroup(senderId, recipientId, rule.group_type);
    case 'same_role': return hasSameRole(senderId, recipientId, rule.role);
    case 'sender_has_role': return userHasRole(senderId, rule.role);
    case 'recipient_has_role': return userHasRole(recipientId, rule.role);
    default: return false;
  }
}

const RELATION = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/;

export async function hasRelationship(userA, userB, relationType) {
  if (!RELATION.test(String(relationType))) return false;
  const label = relationType.toUpperCase().replace(/-/g, '_');
  for (const [a, b] of [[userA, userB], [userB, userA]]) {
    const rows = await sqlRows(
      `SELECT * FROM GRAPH_TABLE(MATCH (a)-[:${label}]->(b) WHERE ${idCondition('a', a)} AND ${idCondition('b', b)} COLUMNS (b.id AS id)) AS g LIMIT 1`,
      [],
    );
    if (rows.length > 0) return true;
  }
  return false;
}

/** Both users are members of one circle (optionally of a `circle_type`). */
export async function inSameCircle(userA, userB, circleType) {
  const [idA, idB] = await Promise.all([nodeIdOf(userA), nodeIdOf(userB)]);
  if (!idA || !idB) return false;
  const [circlesA, circlesB] = await Promise.all([circlesOf(idA), circlesOf(idB)]);
  const shared = circlesA.filter((c) => circlesB.some((d) => d.id === c.id));
  if (!shared.length) return false;
  if (!circleType) return true;
  for (const c of shared) {
    let node = null;
    try {
      node = await raisin.nodes.getById('raisin:access_control', c.id);
    } catch (_) {
      node = null;
    }
    if (node?.properties?.circle_type === circleType) return true;
  }
  return false;
}

async function getUserNode(userId) {
  if (!userId) return null;
  if (typeof userId === 'string' && userId.startsWith('/')) {
    return raisin.nodes.get('raisin:access_control', userId);
  }
  try {
    return await raisin.nodes.getById('raisin:access_control', userId);
  } catch (_) {
    return null;
  }
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

// ─── Limits ─────────────────────────────────────────────────────────────────

/** `max_message_length` (characters); null = no limit. Returns a failure reason or null. */
export function lengthViolation(config, text) {
  const max = Number(config?.max_message_length);
  if (!Number.isFinite(max) || max <= 0) return null;
  return [...String(text ?? '')].length > max ? 'too_long' : null;
}

/**
 * `rate_limit_messages_per_minute`: counts the sender's outbox messages of the last minute (this one
 * included). Returns a failure reason or null.
 */
export async function rateViolation(config, senderHome) {
  const max = Number(config?.rate_limit_messages_per_minute);
  if (!Number.isFinite(max) || max <= 0 || !senderHome) return null;
  const since = new Date(Date.now() - 60_000).toISOString();
  const rows = await sqlRows(
    `SELECT COUNT(*) AS count FROM 'raisin:access_control'
     WHERE CHILD_OF($1) AND node_type = 'raisin:Message' AND created_at > $2`,
    [`${senderHome}/outbox`, since],
  );
  const count = Number(rows[0]?.count ?? 0);
  return count > max ? 'rate_limited' : null;
}

// ─── Attachments ────────────────────────────────────────────────────────────

/** Where the system keeps who each one-to-one conversation is between (raisin:ConversationRecord). */
export const CONVERSATION_RECORDS = '/conversation-records';

/**
 * The one-to-one conversation a message or task may name. The sender chooses `requested`, and a person may
 * create conversation copies in their own inbox (any id, any participants), so no copy decides anything: the
 * system's record of the conversation does (`/conversation-records/<id>`, written by `registerConversation`
 * on its first delivery, readable by nobody but the engine). A requested id is taken only when its record
 * names exactly these two, or when it has none yet (a new conversation, registered on delivery). A group
 * conversation (`grp-…`) never is one: its circle is its authority.
 *
 * `{ id, own }`: `id` null when nothing was requested (the caller mints one); `own` when the record already
 * named these two, the only case in which the conversation's files may be attached (uploads are granted
 * against the same record). Or `{ error: 'not_this_conversation' }`.
 */
export async function conversationBetween(sender, recipient, requested) {
  if (requested === undefined || requested === null || requested === '') return { id: null, own: false };
  if (typeof requested !== 'string' || !CONVERSATION_ID.test(requested) || requested.startsWith('grp-')) {
    return { error: 'not_this_conversation' };
  }
  const record = await raisin.nodes.get(ACL, `${CONVERSATION_RECORDS}/${requested}`);
  if (!record) {
    // No record yet: new, or begun before records existed. A copy of the id in anyone else's home (which
    // only the system can write; the two parties' own homes are left out, since they may make copies there)
    // means it is someone else's conversation, and it is not registered for these two.
    const elsewhere = await sqlRows(
      `SELECT path FROM 'raisin:access_control'
       WHERE node_type = 'raisin:Conversation' AND properties->>'conversation_id' = $1
         AND path NOT LIKE $2 AND path NOT LIKE $3 LIMIT 1`,
      [requested, `${sender.path}/%`, `${recipient.path}/%`],
    );
    const foreign = elsewhere.some((row) => typeof row?.path === 'string'
      && !row.path.startsWith(`${sender.path}/`) && !row.path.startsWith(`${recipient.path}/`));
    return foreign ? { error: 'not_this_conversation' } : { id: requested, own: false };
  }
  if (!betweenExactly(record.properties, sender.id, recipient.id)) return { error: 'not_this_conversation' };
  return { id: requested, own: true };
}

/**
 * Record that conversation `id` is between `a` and `b`, unless it already is. False when the id is
 * already another pair's (two first messages raced for it): the caller refuses.
 */
export async function registerConversation(id, a, b) {
  const path = `${CONVERSATION_RECORDS}/${id}`;
  let record = await raisin.nodes.get(ACL, path);
  if (!record) {
    if (!(await raisin.nodes.get(ACL, CONVERSATION_RECORDS))) {
      await createIdempotent(ACL, '/', {
        slug: CONVERSATION_RECORDS.slice(1), name: CONVERSATION_RECORDS.slice(1), node_type: 'raisin:AclFolder', properties: {},
      });
    }
    await createIdempotent(ACL, CONVERSATION_RECORDS, {
      slug: id, name: id, node_type: 'raisin:ConversationRecord',
      properties: { conversation_id: id, participants: [a, b].sort(), created_at: new Date().toISOString() },
    });
    record = await raisin.nodes.get(ACL, path);
  }
  return Boolean(record) && betweenExactly(record.properties, a, b);
}

function betweenExactly(properties, a, b) {
  let p = properties;
  if (typeof p === 'string') {
    try { p = JSON.parse(p); } catch { return false; }
  }
  const parties = p?.participants;
  if (!Array.isArray(parties) || parties.length !== 2) return false;
  return parties.includes(a) && parties.includes(b);
}

const CONVERSATION_ID = /^[A-Za-z0-9_-]{1,128}$/;

export const MAX_ATTACHMENTS = 10;
export const ATTACHMENT_WORKSPACE = 'assets';
export const ATTACHMENT_ROOT = '/messages';
const SAFE_NAME = /^[^\u0000-\u001f/\\]{1,200}$/;
const MIME = /^[a-z0-9][a-z0-9.+-]*\/[a-z0-9][a-z0-9.+-]*$/i;

/**
 * The attachments of a message or task, checked: at most ten, every one an asset reference below
 * `assets:/messages/<conversation id>/`. The files get there through an upload token the application's
 * server minted for that conversation, so a reference outside it is somebody else's file.
 *
 * Shape: `{ ref: { "raisin:ref", "raisin:workspace" }, name?, mime_type?, size? }`. The reference sits in
 * its own object because the engine reduces any stored object that holds a `raisin:ref` to exactly the
 * reference keys: a name or type written beside it would be dropped. The flat envelope
 * (`{ "raisin:ref", "raisin:workspace", … }`) is accepted as input too.
 * Returns { list } or { error }.
 */
export function checkAttachments(raw, conversationId) {
  if (raw === undefined || raw === null) return { list: [] };
  if (!Array.isArray(raw)) return { error: 'bad_attachments' };
  if (raw.length > MAX_ATTACHMENTS) return { error: 'too_many_attachments' };
  if (raw.length && (typeof conversationId !== 'string' || !/^[A-Za-z0-9_-]{1,128}$/.test(conversationId))) {
    return { error: 'bad_attachments' };
  }
  const prefix = `${ATTACHMENT_ROOT}/${conversationId}/`;
  const list = [];
  for (const a of raw) {
    if (!a || typeof a !== 'object') return { error: 'bad_attachments' };
    const envelope = a.ref && typeof a.ref === 'object' ? a.ref : a;
    const ref = envelope['raisin:ref'];
    const ws = envelope['raisin:workspace'];
    if (ws !== ATTACHMENT_WORKSPACE || typeof ref !== 'string') return { error: 'bad_attachments' };
    if (!ref.startsWith(prefix) || ref.includes('/../') || ref.endsWith('/..') || ref.length > 512) {
      return { error: 'foreign_attachment' };
    }
    const item = { ref: { 'raisin:ref': ref, 'raisin:workspace': ws } };
    if (typeof a.name === 'string' && SAFE_NAME.test(a.name)) item.name = a.name;
    if (typeof a.mime_type === 'string' && MIME.test(a.mime_type)) item.mime_type = a.mime_type.toLowerCase();
    const size = Number(a.size);
    if (Number.isFinite(size) && size >= 0) item.size = Math.floor(size);
    list.push(item);
  }
  return { list };
}
