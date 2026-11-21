/**
 * backfill-tool-steps — ADMIN ONLY.
 *
 * Conversations that ended before tool steps were mirrored into the person's
 * conversation have no `ai_tool_call` messages. This walks the agent side's
 * `raisin:AIToolCall` nodes, one bounded page per call, and sends each through
 * the agent outbox exactly as the live mirror does: the same collision-free
 * name (run id + call id) and the call node's own creation time as `created_at`,
 * so the step sorts where it happened and a re-run (or a live run that mirrored
 * it first) is a no-op.
 */

import { resolveAgentOutboxContext, sendAgentOutboxMessage } from '../agent-shared/outbox.js';
import { isMirroredCall, toolCallData } from '../agent-shared/run-tool-mirror.js';
import { toolCallMirrorSlug } from '../agent-shared/run-names.js';
import { log } from '../agent-shared/logger.js';

const WORKSPACE = 'ai';
const DEFAULT_LIMIT = 100;
const MAX_LIMIT = 500;
const CHAT_PATH = /^\/agents\/[^/]+\/inbox\/chats\/[^/]+$/;
const AGENT_PATH = /^\/agents\/[^/]+$/;

/** Only a system admin (the superadmin, an admin credential) may run this. */
function isAdminCaller() {
  const caller = typeof raisin !== 'undefined' && raisin.context ? raisin.context.caller : null;
  return !!caller && (caller.is_system_admin === true || caller.is_system === true);
}

const str = (v) => (typeof v === 'string' ? v.trim() : '');

function clampLimit(v) {
  const n = Math.floor(Number(v));
  if (!Number.isFinite(n) || n < 1) return DEFAULT_LIMIT;
  return Math.min(n, MAX_LIMIT);
}

/** The conversation path of a tool call path, or null for any other shape. */
export function chatPathOfCall(path) {
  const parts = String(path || '').split('/');
  // '', agents, <agent>, inbox, chats, <chat>, <run-op>, <tool-call>
  if (parts.length !== 8 || parts[1] !== 'agents' || parts[3] !== 'inbox' || parts[4] !== 'chats') return null;
  return parts.slice(0, 6).join('/');
}

const toIso = (v) => {
  const t = Date.parse(String(v || ''));
  return Number.isNaN(t) ? null : new Date(t).toISOString();
};

async function turnOf(row) {
  const parent = await raisin.nodes.get(WORKSPACE, row.path.split('/').slice(0, -1).join('/'));
  return (parent && parent.properties) || {};
}

async function runIdOf(row, props, turn) {
  if (props.run_id) return String(props.run_id);
  return turn.run_id ? String(turn.run_id) : null;
}

async function resultContentOf(row, props) {
  const res = await raisin.nodes.get(WORKSPACE, `${row.path}/result`);
  if (res && res.properties && res.properties.result !== undefined) return res.properties.result;
  return props.status === 'failed' ? { status: 'failed', error: 'The call failed.' } : { status: 'completed' };
}

export async function handler(input = {}) {
  if (!isAdminCaller()) {
    return { success: false, error: 'forbidden', message: 'backfill-tool-steps may only be run by a system administrator.' };
  }
  const chatPath = str(input.chat_path);
  const agentPath = str(input.agent_path);
  if (chatPath && !CHAT_PATH.test(chatPath)) return { success: false, error: 'invalid_input', message: 'chat_path must be /agents/<agent>/inbox/chats/<id>.' };
  if (agentPath && !AGENT_PATH.test(agentPath)) return { success: false, error: 'invalid_input', message: 'agent_path must be /agents/<agent>.' };
  const root = chatPath || agentPath || '/agents';
  const dryRun = input.dry_run === true;
  const limit = clampLimit(input.limit);
  const cursor = str(input.cursor) || root;

  // A path cursor keeps every page a bounded, ordered index range.
  const rows = await raisin.sql.query(
    `SELECT path, properties, created_at FROM '${WORKSPACE}' WHERE node_type = 'raisin:AIToolCall' AND DESCENDANT_OF($1) AND path > $2 ORDER BY path ASC LIMIT ${limit}`,
    [root, cursor],
  );
  const page = Array.isArray(rows) ? rows : [];
  const counts = { scanned: page.length, mirrored: 0, would_mirror: 0, already_mirrored: 0, skipped_internal: 0, skipped_no_run: 0, skipped_no_outbox: 0, errors: 0 };
  const outboxByChat = new Map();

  for (const row of page) {
    try {
      const props = row.properties || {};
      if (!isMirroredCall(props.function_name)) { counts.skipped_internal += 1; continue; }
      const chat = chatPathOfCall(row.path);
      if (!chat) { counts.skipped_no_outbox += 1; continue; }
      const turn = await turnOf(row);
      const runId = await runIdOf(row, props, turn);
      if (!runId) { counts.skipped_no_run += 1; continue; }
      if (!outboxByChat.has(chat)) {
        const chatNode = await raisin.nodes.get(WORKSPACE, chat);
        outboxByChat.set(chat, chatNode ? await resolveAgentOutboxContext(WORKSPACE, chat, chatNode) : null);
      }
      const outbox = outboxByChat.get(chat);
      if (!outbox) { counts.skipped_no_outbox += 1; continue; }
      const slug = toolCallMirrorSlug(runId, props.tool_call_id);
      if (await raisin.nodes.get(WORKSPACE, `${outbox.agentHomePath}/outbox/${slug}`)) { counts.already_mirrored += 1; continue; }
      if (dryRun) { counts.would_mirror += 1; continue; }
      const call = { call_id: props.tool_call_id, name: props.function_name, args: props.arguments || {} };
      const tr = { call_id: props.tool_call_id, content: await resultContentOf(row, props) };
      await sendAgentOutboxMessage(WORKSPACE, outbox, call.name, 'ai_tool_call', toolCallData(call, tr, runId, { includeResult: turn.run_stream_results === true }), {
        slug,
        created_at: toIso(row.created_at) || new Date().toISOString(),
      });
      counts.mirrored += 1;
    } catch (err) {
      counts.errors += 1;
      log.warn('backfill-tool-steps', 'Could not mirror one call', { path: row.path, error: String(err && err.message) });
    }
  }

  const last = page.length ? page[page.length - 1].path : null;
  const done = page.length < limit;
  return { success: true, dry_run: dryRun, root, ...counts, next_cursor: done ? null : last, done };
}
