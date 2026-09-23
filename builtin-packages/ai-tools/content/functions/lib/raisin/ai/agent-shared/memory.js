/**
 * Per-user agent memory.
 *
 * Each agent keeps one markdown document per user at
 *   ai:/agents/{agentName}/memory/{sanitized_user_id}
 * (`raisin:AgentUserContext`, `content`; a legacy `entries` array is still
 * read). The model turn appends it to the system prompt; `remember`,
 * `forget` and `read-user-context` edit it.
 *
 * WHOSE memory is never taken from a tool's arguments: it is the run's
 * OWNER — the agent the run executes and the user it acts for, both read
 * from the run record core keeps (`memoryOwnerOf`). The memory tools run in
 * a system context (no user role grants the `ai` workspace) and prove their
 * run first, so a user can only ever reach their own memory with this agent.
 */

import { log } from './logger.js';
import { actingUser, runAgent, requireRunOperation } from './run-caller.js';
import { runContextOf, buildEnvelope, errorEnvelope, locator } from './tool-envelope.js';

export const MEMORY_WORKSPACE = 'ai';

const safeUser = (id) => String(id || '').replace(/[^a-zA-Z0-9_-]/g, '_');

/** Where one agent keeps one user's memory. */
export function memoryPath(agentName, userId) {
  return `/agents/${agentName}/memory/${safeUser(userId)}`;
}

/** `{ agentName, userId }` of a run record, or null when either is unknown. */
export function memoryOwnerOf(rec) {
  const agent = runAgent(rec).path;
  const m = /^\/agents\/([^/]+)$/.exec(String(agent || ''));
  const userId = actingUser(rec);
  if (!m || !userId) return null;
  return { agentName: m[1], userId };
}

/**
 * Load stored memory for a user from an agent's memory store.
 * Returns markdown, or '' when there is none.
 */
async function loadUserMemory(agentName, userId) {
  if (!agentName || !userId) return '';
  try {
    const node = await raisin.nodes.get(MEMORY_WORKSPACE, memoryPath(agentName, userId));
    if (!node) return '';
    const raw = node.properties?.content;
    if (typeof raw === 'string' && raw.trim()) return raw.trim();
    if (Array.isArray(node.properties?.entries)) {
      return node.properties.entries
        .filter((e) => e.key)
        .map((e) => `- ${e.key}: ${e.value || ''}`)
        .join('\n');
    }
  } catch (err) {
    log.debug('memory', 'No user memory', { error: String(err && err.message) });
  }
  return '';
}

/** Write (create or replace) one user's memory document. */
export async function saveUserMemory(agentName, userId, content) {
  const path = memoryPath(agentName, userId);
  const now = new Date().toISOString();
  const existing = await raisin.nodes.get(MEMORY_WORKSPACE, path);
  if (existing) {
    await raisin.nodes.update(MEMORY_WORKSPACE, path, { properties: { content, updated_at: now } });
    return { path, action: 'updated' };
  }
  const folder = `/agents/${agentName}/memory`;
  if (!(await raisin.nodes.get(MEMORY_WORKSPACE, folder))) {
    try {
      await raisin.nodes.create(MEMORY_WORKSPACE, `/agents/${agentName}`, {
        name: 'memory', node_type: 'raisin:Folder', properties: { title: 'User Memory' },
      });
    } catch (err) {
      if (!/already exists/i.test(String(err && err.message))) throw err;
    }
  }
  await raisin.nodes.create(MEMORY_WORKSPACE, folder, {
    name: safeUser(userId),
    node_type: 'raisin:AgentUserContext',
    properties: { user_id: userId, content, updated_at: now },
  });
  return { path, action: 'created' };
}

/**
 * Format memory content into a system prompt section.
 * Returns empty string if there is no memory to include.
 */
function formatMemoryForPrompt(memoryContent) {
  if (!memoryContent) return '';
  return [
    '',
    '[User Context Memory]',
    "The following are things you've been asked to remember about this user:",
    memoryContent,
    '',
    'You can update these with the remember/forget tools.',
  ].join('\n');
}

export {
  loadUserMemory,
  formatMemoryForPrompt,
};

/**
 * Run a memory tool for the PROVEN owner of the calling run. `body(owner,
 * input)` returns `{ payload, writes? }`. Outside a run it refuses: without a
 * run there is no trustworthy answer to "whose memory is this".
 */
export async function memoryTool(input, functionPath, body) {
  const ctx = runContextOf(input);
  if (!ctx) {
    return { success: false, error: 'Agent memory works only inside an agent run (the run says whose memory it is).' };
  }
  try {
    const run = await requireRunOperation(input, functionPath, { what: 'Agent memory' });
    const owner = memoryOwnerOf(run.rec);
    if (!owner) {
      throw Object.assign(new Error('This run acts for no user, so it has no user memory.'), { error_class: 'unsupported' });
    }
    const r = await body(owner, input || {});
    const writes = (r.writes || []).map((w) => ({ locator: locator(MEMORY_WORKSPACE, w.path), action: w.action }));
    return buildEnvelope({
      operationId: ctx.operation_id,
      status: 'succeeded',
      payload: r.payload,
      writes,
      artifactRefs: writes.map((w) => ({ kind: 'user_memory', locator: w.locator, role: 'primary' })),
      retryPolicy: { retryable: false, max_attempts: 1, backoff_ms: 0, reason: writes.length ? 'idempotent_by_operation_id' : 'read_only' },
    });
  } catch (err) {
    return errorEnvelope(ctx.operation_id, err);
  }
}
