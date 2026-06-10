/**
 * Conversation auto-compaction for agent handlers.
 *
 * When an agent enables `auto_compact` and the number of (non-compacted)
 * messages in a conversation exceeds `compact_threshold_messages`, the older
 * messages are summarized with ONE extra AI call and the result is persisted
 * as a raisin:AICompaction child of the conversation node. History building
 * (agent-shared/history.js) then replaces the summarized messages with the
 * stored summary — the summary is never recomputed per turn.
 *
 * Later compactions supersede earlier ones (latest by created_at wins) and
 * chain context by feeding the previous summary into the new one.
 */

import { log } from './logger.js';
import { createCostRecord } from './utils.js';

const DEFAULT_COMPACT_THRESHOLD = 30;
/** Max chars of a single message fed into the summarization transcript. */
const TRANSCRIPT_MESSAGE_CHAR_LIMIT = 1500;

const SUMMARIZE_SYSTEM_PROMPT =
  'You are an AI assistant compacting your own conversation memory. '
  + 'The user message contains a transcript of the conversation so far. '
  + 'Summarize it for your own future context: preserve every concrete fact, '
  + 'name, number, decision, and open question stated by either side, even '
  + 'ones that seem trivial. Respond with the summary only, no preamble.';

function extractText(props) {
  if (!props) return '';
  if (typeof props.content === 'string' && props.content.trim()) return props.content.trim();
  if (typeof props.body === 'string' && props.body.trim()) return props.body.trim();
  if (props.body && typeof props.body === 'object') {
    const fromBody = props.body.content || props.body.message_text || '';
    if (typeof fromBody === 'string') return fromBody.trim();
  }
  return '';
}

/** Fetch the latest raisin:AICompaction node for a conversation, or null. */
async function getLatestCompaction(workspace, chatPath) {
  const rows = await raisin.sql.query(`
    SELECT path, properties, created_at
    FROM '${workspace}'
    WHERE CHILD_OF($1)
      AND node_type = 'raisin:AICompaction'
    ORDER BY created_at DESC
    LIMIT 1
  `, [chatPath]);
  return Array.isArray(rows) && rows.length > 0 ? rows[0] : null;
}

/**
 * Compact the conversation if the agent enables it and the active (not yet
 * compacted) message count exceeds the threshold.
 *
 * Agent properties:
 *   auto_compact (bool, default false)
 *   compact_threshold_messages (number, default 30)
 *   compact_keep_messages (number, optional floor of recent messages to keep;
 *                          defaults to max(2, threshold / 3))
 *
 * Never throws — on any failure the previous compaction state is returned and
 * the turn proceeds with uncompacted history.
 */
async function maybeCompactConversation(workspace, chatPath, agentProps, modelId) {
  if (agentProps?.auto_compact !== true) return null;

  let existing = null;
  try {
    const threshold = Number(agentProps.compact_threshold_messages) > 0
      ? Math.floor(Number(agentProps.compact_threshold_messages))
      : DEFAULT_COMPACT_THRESHOLD;

    const rows = await raisin.sql.query(`
      SELECT path, properties, created_at
      FROM '${workspace}'
      WHERE CHILD_OF($1)
        AND node_type = 'raisin:Message'
      ORDER BY created_at ASC
    `, [chatPath]);
    const messages = Array.isArray(rows) ? rows : [];

    existing = await getLatestCompaction(workspace, chatPath);

    // Only messages after the last compaction cutoff count toward the threshold
    let active = messages;
    const prevCutoff = existing?.properties?.cutoff_message_path;
    if (prevCutoff) {
      const idx = messages.findIndex(m => m.path === prevCutoff);
      if (idx >= 0) active = messages.slice(idx + 1);
    }

    if (active.length <= threshold) return existing;

    const keepFloor = Number(agentProps.compact_keep_messages) > 0
      ? Math.floor(Number(agentProps.compact_keep_messages))
      : Math.max(2, Math.floor(threshold / 3));
    const keep = Math.max(keepFloor, Math.floor(active.length / 3));
    const toCompact = active.slice(0, active.length - keep);
    if (toCompact.length === 0) return existing;

    // Build a plain-text transcript; chain the previous summary so context
    // survives successive compactions.
    const lines = ['Conversation transcript to summarize:'];
    if (existing?.properties?.summary) {
      lines.push(`[summary of even earlier messages] ${existing.properties.summary}`);
    }
    for (const m of toCompact) {
      const role = m.properties?.role || 'user';
      const text = extractText(m.properties);
      if (text) lines.push(`${role}: ${text.slice(0, TRANSCRIPT_MESSAGE_CHAR_LIMIT)}`);
    }

    const t0 = log.time();
    const raw = await raisin.ai.completion({
      messages: [
        { role: 'system', content: SUMMARIZE_SYSTEM_PROMPT },
        { role: 'user', content: lines.join('\n') },
      ],
      model: modelId,
      stream: false,
    });
    const summary = typeof raw?.content === 'string'
      ? raw.content.trim()
      : String(raw?.content ?? '').trim();
    if (!summary) {
      log.warn('compaction', 'Summarization returned empty content, skipping compaction');
      return existing;
    }

    const cutoff = toCompact[toCompact.length - 1];
    const node = await raisin.nodes.create(workspace, chatPath, {
      name: `compaction-${Date.now()}`,
      node_type: 'raisin:AICompaction',
      properties: {
        messages_compacted: toCompact.length,
        messages_kept: active.length - toCompact.length,
        summary,
        summary_preview: summary.slice(0, 200),
        cutoff_message_path: cutoff.path,
        cutoff_created_at: cutoff.created_at || cutoff.properties?.created_at || null,
        created_at: new Date().toISOString(),
      },
    });

    // Account for the summarization call itself
    await createCostRecord(workspace, chatPath, node.path, raw, agentProps.provider, log.since(t0));

    log.info('compaction', 'Compacted conversation', {
      chat: chatPath,
      compacted: toCompact.length,
      kept: active.length - toCompact.length,
      cutoff: cutoff.path,
    });
    return node;
  } catch (err) {
    log.warn('compaction', 'Compaction failed, continuing without it', {
      chat: chatPath,
      error: err?.message || String(err),
    });
    return existing;
  }
}

export {
  getLatestCompaction,
  maybeCompactConversation,
};
