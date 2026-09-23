import { runEnvelope } from '../agent-shared/tool-envelope.js';
import { TOOL_META } from '../agent-shared/tool-meta.js';
/**
 * search-documents — retrieval, as a tool an agent can call.
 *
 * Wraps `HYBRID_SEARCH(..., granularity => 'chunk')`: one vector leg, one
 * full-text leg, fused by rank. The engine does the work; this function's job
 * is to hand back PASSAGES with stable citation handles and to be honest about
 * how much of each passage it actually has.
 *
 * # Why chunk granularity, and not the default
 *
 * The default, `granularity => 'node'`, returns one row per DOCUMENT. `LIMIT 8`
 * is then eight documents, which is the right answer for a search results page
 * and the wrong one for filling a context window: the answer to a question
 * usually lives in one clause of one document, and eight whole documents do not
 * fit anywhere. `'chunk'` returns one row per passage — several rows may share a
 * `node_id`, and that is the point.
 *
 * # `text_is_exact` is not decoration
 *
 * `chunk_text_source` is `exact` only when the engine could slice the passage
 * back out of the stored `__extracted_text` AND the hash matched. Otherwise the
 * caller is holding a 200-character PREVIEW. A model handed a preview will
 * happily quote it as if it were the whole clause, so the distinction is
 * surfaced rather than flattened — see the `ask` function, which refuses to
 * quote a non-exact passage verbatim.
 *
 * Execution mode: async
 */

/** Passages to return when the caller doesn't say. */
const DEFAULT_LIMIT = 8;

/**
 * Hard ceiling. Not politeness: every leg over-fetches 20x the limit before
 * row-level security filters, so a caller asking for 500 passages is asking the
 * engine for 10,000 candidates to throw most of away.
 */
const MAX_LIMIT = 50;

/**
 * The only breadth spelling the engine accepts. `'*'` and `'ALL'` are rejected
 * on purpose, so don't "helpfully" translate them here.
 */
const DEFAULT_SCOPE = 'ALL READABLE';

export async function handler(input) {
  const enveloped = await runEnvelope(input, TOOL_META.searchDocuments, handler); if (enveloped) return enveloped;
  const { query, workspaces, limit, max_distance } = input || {};

  if (!query || typeof query !== 'string' || !query.trim()) {
    throw new Error('A non-empty `query` is required');
  }

  const scope = typeof workspaces === 'string' && workspaces.trim()
    ? workspaces.trim()
    : DEFAULT_SCOPE;

  const requested = Number.isFinite(limit) ? Math.floor(limit) : DEFAULT_LIMIT;
  const effectiveLimit = Math.min(Math.max(requested, 1), MAX_LIMIT);

  // Built by concatenation, and that is safe here ONLY because every value
  // interpolated below is a number this function produced. `query` and `scope`
  // are bound as parameters: they are caller-controlled, and `scope` reaches a
  // named argument, which the engine substitutes before parsing.
  const distanceClause = Number.isFinite(max_distance)
    ? `, max_distance => ${Number(max_distance)}`
    : '';

  const sql = `
    SELECT node_id, path, name, score, chunk_index, chunk_text, chunk_text_source
    FROM HYBRID_SEARCH($1, ${effectiveLimit},
                       workspaces => $2,
                       granularity => 'chunk'${distanceClause})
  `;

  const rows = await raisin.sql.query(sql, [query.trim(), scope]);

  const results = (rows || []).map((row) => ({
    path: row.path,
    node_id: row.node_id,
    title: row.name,
    // NULL for an unchunked document; 0 is a real chunk index, so don't
    // collapse the two with `||`.
    chunk_index: row.chunk_index == null ? 0 : row.chunk_index,
    text: row.chunk_text || '',
    text_is_exact: row.chunk_text_source === 'exact',
    score: row.score,
  })).filter((r) => r.text);

  console.log(
    `[search-documents] "${query.trim().slice(0, 80)}" in ${scope} → ` +
    `${results.length} passage(s)`
  );

  return { results, count: results.length };
}
