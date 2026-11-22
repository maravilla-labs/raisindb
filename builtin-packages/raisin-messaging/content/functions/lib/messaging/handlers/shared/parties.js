/**
 * Parties of a message: who sent it (decided by WHERE it lives) and who it is for.
 *
 * Shared by every handler of the inbox (chat, group chat, task assignment), so a sender is resolved,
 * checked and refused the same way everywhere. Moved out of handle-chat unchanged.
 *
 * Stable person keys: a user's home is `/users/internal/<user_key>`, where user_key is `u_` + 22 base62
 * characters minted by the engine (homes created before keys keep an email-slug path and have no key).
 * A copy a PERSON reads carries ids and keys, never a home path (`readableBy`).
 *
 * The `raisin` global is injected at runtime -- do NOT import it.
 */

/** A user key: `u_` + 22 base62 (raisin_models::auth::user_key). */
const USER_KEY = /^u_[0-9A-Za-z]{22}$/;
export const USER_HOMES_ROOT = '/users/internal';

export function isUserKey(value) {
  return typeof value === 'string' && USER_KEY.test(value);
}

/** The key of a keyed home (`/users/internal/u_…`), else null. */
export function userKeyFromHome(path) {
  if (typeof path !== 'string') return null;
  const last = path.replace(/\/+$/, '').split('/').pop();
  return isUserKey(last) ? last : null;
}

export function isAgentId(id) {
  return typeof id === 'string' && id.startsWith('agent:');
}

export function extractAgentName(id, path) {
  if (id && isAgentId(id)) return id.slice('agent:'.length);
  if (path) {
    const parts = path.split('/');
    if (parts.length >= 3 && parts[1] === 'agents') return parts[2];
  }
  return null;
}

/** Fields that name a home path: never in a copy a person reads. */
const PATH_FIELDS = ['sender_path', 'recipient_path', 'sender_message_path', 'participant_paths'];

/**
 * The message properties as `reader` may see them: a person's copy (a user or visitor home) carries ids
 * and keys, no home path; an agent's copy keeps the paths the agent runtime works with.
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

/**
 * The home whose outbox directly holds the message at `nodePath`, or null. Only these exact shapes are
 * outboxes (every segment non-empty, the message a direct child of `outbox`):
 *
 *   /users/<slug>/outbox/<name>               -> /users/<slug>
 *   /users/internal/<key|slug>/outbox/<name>  -> /users/internal/<key|slug>
 *   /agents/<name>/outbox/<name>              -> /agents/<name>
 *   /visitors/<key>/outbox/<name>             -> /visitors/<key>
 *
 * The position decides, not a search for the word: a home literally named `outbox`
 * (`/users/outbox/outbox/m`) is that home, and a node nested deeper under an outbox is no message.
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
export async function resolveSenderHome(triggerWorkspace, home) {
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
export function claimsMatch(sender, home, { ids, paths, emails, keys }) {
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
export async function refuse(workspace, nodePath, reason, tag = 'messaging') {
  console.warn(`[${tag}] refused ${nodePath}: ${reason}`);
  if (nodePath) {
    await raisin.nodes.updateProperty(workspace, nodePath, 'status', 'failed');
    await raisin.nodes.updateProperty(workspace, nodePath, 'failure_reason', reason);
  }
  return { success: false, reason };
}

export async function resolveRecipient(hint) {
  if (isVisitorId(hint.id) || isVisitorPath(hint.path)) {
    return resolveVisitor(hint.path, hint.id);
  }
  // Try user first
  const user = await resolveUser('raisin:access_control', hint.path, hint.id, hint.email, hint.key);
  if (user) return user;
  // Fall back to agent
  return resolveAgent(hint.path, hint.id);
}

export async function resolveAgent(agentPath, agentId) {
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

export async function resolveUser(workspace, userPath, userId, email, userKey) {
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

export function userNodeToEntity(node, workspace) {
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

export function isVisitorId(id) {
  return typeof id === 'string' && id.startsWith(VISITOR_ID_PREFIX);
}

export function isVisitorPath(path) {
  return typeof path === 'string' && path.startsWith(VISITOR_ROOT + '/');
}

/**
 * The visitor session named by a path and/or id. Both, when given, must name the SAME session: a message
 * that claims one visitor's id from another's home is refused rather than delivered to either.
 */
export async function resolveVisitor(path, id) {
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

// ─── Folders and SQL ────────────────────────────────────────────────────────

export async function ensureFolderExists(workspace, parentPath, slug, title) {
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

export async function ensureChatsFolder(workspace, entityPath) {
  const inboxPath = await ensureFolderExists(workspace, entityPath, 'inbox', 'Inbox');
  return ensureFolderExists(workspace, inboxPath, 'chats', 'Chats');
}

/** Create a node, treating "already exists" as done (idempotent redelivery). */
export async function createIdempotent(workspace, parentPath, nodeData) {
  try {
    await raisin.nodes.create(workspace, parentPath, nodeData);
    return true;
  } catch (e) {
    if (!String(e?.message || '').includes('already exists')) throw e;
    return false;
  }
}

export async function sqlRows(sql, params) {
  const result = await raisin.sql.query(sql, params);
  return Array.isArray(result) ? result : (result?.rows ?? []);
}

/** A node id or home path as a SQL literal condition. Only ids and paths the engine minted reach this. */
export function idCondition(alias, entityId) {
  const isPath = typeof entityId === 'string' && entityId.startsWith('/');
  const safe = String(entityId).replace(/'/g, "''");
  return isPath ? `${alias}.path = '${safe}'` : `${alias}.id = '${safe}'`;
}

/** The node id of a user named by id or home path (NEIGHBORS starts from an id). */
export async function nodeIdOf(entityId) {
  if (typeof entityId !== 'string' || !entityId) return null;
  if (!entityId.startsWith('/')) return entityId;
  const node = await raisin.nodes.get('raisin:access_control', entityId);
  return node?.id ?? null;
}
