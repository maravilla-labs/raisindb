/**
 * Circles: the groups people talk and work in.
 *
 * A group is a `raisin:EntityCircle` (usually under `raisin:access_control:/circles`). Its members are the
 * users with a `MEMBER_OF` edge to it, its moderators the users with a `MODERATES` edge. Ordinary users
 * may not relate, so the roster is decided by an admin, the system or an application function, and is
 * read from the graph, never from anything a member wrote.
 *
 * The circle's `messaging` property (absent = the defaults below) says what the group may do:
 *
 *   { chat: 'members' | 'moderators' | 'off', tasks: 'moderators' | 'members' | 'off', max_members: 500 }
 *
 * The `raisin` global is injected at runtime -- do NOT import it.
 */

import { sqlRows, userNodeToEntity } from './parties.js';

export const CIRCLE_PREFIX = 'circle:';
/** Every member's copy of a circle's conversation has this id: one thread, one attachment folder. */
export const GROUP_CONVERSATION_PREFIX = 'grp-';
export const MEMBER_OF = 'MEMBER_OF';
export const MODERATES = 'MODERATES';
const ACL = 'raisin:access_control';
const DEFAULT_POLICY = { chat: 'members', tasks: 'moderators', max_members: 500 };
const SAFE_ID = /^[A-Za-z0-9_-]{1,96}$/;

/** The conversation id of a circle's group chat. */
export function conversationIdOf(circle) {
  return `${GROUP_CONVERSATION_PREFIX}${circle.id}`;
}

/**
 * The circle a message or task addresses, as a reference `resolveCircle` takes, or null. Any of:
 * `recipient_id` / `assignee` = `circle:<id>`, `body.circle` = `circle:<id>` or a `/circles/…` path,
 * or a group conversation id (`grp-<id>`) as `conversation_id` / `thread_id`.
 */
export function circleReference(props = {}, body = {}) {
  const candidates = [
    props.recipient_id, body.recipient_id, body.assignee, props.assignee,
    body.circle, props.circle,
  ];
  for (const c of candidates) {
    if (typeof c !== 'string') continue;
    if (c.startsWith(CIRCLE_PREFIX)) return c;
    if (c.startsWith('/') && (c === '/circles' || c.startsWith('/circles/'))) return c;
  }
  for (const c of [body.thread_id, props.conversation_id, body.conversation_id]) {
    if (typeof c === 'string' && c.startsWith(GROUP_CONVERSATION_PREFIX)) {
      return CIRCLE_PREFIX + c.slice(GROUP_CONVERSATION_PREFIX.length);
    }
  }
  return null;
}

export function policyOf(node) {
  const m = node?.properties?.messaging;
  const p = m && typeof m === 'object' ? m : {};
  const pick = (v, allowed, d) => (allowed.includes(v) ? v : d);
  const max = Number(p.max_members);
  return {
    chat: pick(p.chat, ['members', 'moderators', 'off'], DEFAULT_POLICY.chat),
    tasks: pick(p.tasks, ['members', 'moderators', 'off'], DEFAULT_POLICY.tasks),
    max_members: Number.isFinite(max) && max > 0 ? Math.min(max, 1000) : DEFAULT_POLICY.max_members,
  };
}

/** Resolve a circle reference (`circle:<node id>`, a `/circles/…` path). */
export async function resolveCircle(reference) {
  if (typeof reference !== 'string' || !reference) return null;
  let node = null;
  if (reference.startsWith(CIRCLE_PREFIX)) {
    const id = reference.slice(CIRCLE_PREFIX.length);
    if (!SAFE_ID.test(id)) return null;
    try {
      node = await raisin.nodes.getById(ACL, id);
    } catch (_) {
      node = null;
    }
  } else if (reference.startsWith('/')) {
    node = await raisin.nodes.get(ACL, reference);
  }
  if (!node || node.node_type !== 'raisin:EntityCircle') return null;
  const p = node.properties ?? {};
  return {
    id: node.id,
    path: node.path,
    name: p.name || node.name,
    type: p.circle_type || null,
    policy: policyOf(node),
    node,
  };
}

/** The users related to a circle by `relation` (incoming edges), as message parties. */
async function relatedUsers(circleId, relation) {
  const rows = await sqlRows(
    `SELECT id, path, node_type FROM NEIGHBORS($1, 'IN', '${relation}')`,
    [circleId],
  );
  const out = [];
  const seen = new Set();
  for (const row of rows) {
    if (!row || row.node_type !== 'raisin:User' || seen.has(row.id)) continue;
    seen.add(row.id);
    let node = null;
    try {
      node = await raisin.nodes.getById(ACL, row.id);
    } catch (_) {
      node = null;
    }
    if (node && node.node_type === 'raisin:User') out.push(userNodeToEntity(node, ACL));
  }
  return out;
}

export function membersOf(circle) {
  return relatedUsers(circle.id, MEMBER_OF);
}

export function moderatorsOf(circle) {
  return relatedUsers(circle.id, MODERATES);
}

/**
 * Everyone a circle's conversation reaches (members and moderators, once each) and what the sender may
 * do in it. `role` is `moderator`, `member` or null (not in the circle).
 */
export async function rosterFor(circle, senderId) {
  const [members, moderators] = await Promise.all([membersOf(circle), moderatorsOf(circle)]);
  const byId = new Map();
  for (const m of members) byId.set(m.id, m);
  for (const m of moderators) byId.set(m.id, m);
  const isModerator = moderators.some((m) => m.id === senderId);
  const isMember = members.some((m) => m.id === senderId);
  return {
    everyone: [...byId.values()],
    memberCount: members.length,
    role: isModerator ? 'moderator' : isMember ? 'member' : null,
  };
}

/** May someone with `role` do `action` ('chat' | 'tasks') under `policy`? */
export function mayAct(policy, role, action) {
  const rule = policy[action];
  if (!role || rule === 'off') return false;
  if (rule === 'moderators') return role === 'moderator';
  return true;
}

/** The circles a user belongs to (MEMBER_OF), as node rows { id, path }. */
export async function circlesOf(userNodeId) {
  if (!userNodeId) return [];
  const rows = await sqlRows(
    `SELECT id, path, node_type FROM NEIGHBORS($1, 'OUT', '${MEMBER_OF}')`,
    [userNodeId],
  );
  return rows.filter((r) => r && r.node_type === 'raisin:EntityCircle');
}
