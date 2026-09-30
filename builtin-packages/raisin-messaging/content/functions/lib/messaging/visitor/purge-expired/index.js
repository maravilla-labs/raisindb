/**
 * Purge expired anonymous visitor sessions.
 *
 * A visitor session (/visitors/<key>, raisin:VisitorSession) is the ephemeral
 * home of one anonymous chat. The server moves its `expires_at` forward on
 * every message (the agent's `anonymous.session_ttl_hours`, default 24 h after
 * the last activity); once it has passed, the session and everything under it
 * (its conversations and their delivered messages) are deleted here.
 *
 * The agent's side of each conversation stays, as for every other chat: the
 * cost records (raisin:AICostRecord) and tool-call audit live in the `ai`
 * workspace under /agents/<name>/inbox/chats/<id>.
 *
 * Deletes one session at a time: one failure (a node that moved, a race with
 * a message landing right now) must not stop the rest of the sweep.
 */

const WORKSPACE = 'raisin:access_control';
const DEFAULT_LIMIT = 500;

export async function purge_expired(input) {
  const args = (input && (input.flow_input ?? input)) || {};
  const now = typeof args.now === 'string' && args.now ? args.now : new Date().toISOString();
  const limit = Number(args.limit) > 0 ? Math.floor(Number(args.limit)) : DEFAULT_LIMIT;

  const rows = await rowsOf(await raisin.sql.query(
    `SELECT path, properties FROM '${WORKSPACE}'
     WHERE node_type = 'raisin:VisitorSession' AND DESCENDANT_OF($1)`,
    ['/visitors'],
  ));

  const expired = rows
    .filter((row) => isExpired(row.properties, now))
    .slice(0, limit);

  let purged = 0;
  let failed = 0;
  for (const row of expired) {
    if (!/^\/visitors\/[0-9a-f]{16,128}$/.test(String(row.path))) continue;
    try {
      await raisin.nodes.delete(WORKSPACE, row.path);
      purged += 1;
    } catch (err) {
      failed += 1;
      console.warn('[purge-expired-visitors] could not delete', row.path, String(err && err.message));
    }
  }
  return { purged, failed };
}

/**
 * A session without `expires_at` is treated as expired only once it is older
 * than a day by `last_activity_at`/`created_at`: the server always writes
 * `expires_at`, so a session without one was written by something else and
 * must not live forever either.
 */
export function isExpired(props, nowIso) {
  const p = props || {};
  const now = Date.parse(nowIso);
  const at = (v) => (typeof v === 'string' ? Date.parse(v) : NaN);
  const expires = at(p.expires_at);
  if (Number.isFinite(expires)) return expires <= now;
  const last = Math.max(at(p.last_activity_at) || 0, at(p.created_at) || 0);
  return last > 0 ? last + 24 * 3600 * 1000 <= now : true;
}

function rowsOf(result) {
  return Array.isArray(result) ? result : (result && result.rows) || [];
}
