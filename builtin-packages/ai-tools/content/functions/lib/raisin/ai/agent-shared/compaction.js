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
import { stripInternalContext } from './history.js';

const DEFAULT_COMPACT_THRESHOLD = 30;
/** Max chars of a single message fed into the summarization transcript. */
const TRANSCRIPT_MESSAGE_CHAR_LIMIT = 1500;
/* A tool line is a REMINDER that the work happened and roughly what came back,
 * not a replay of it. Kept short deliberately: a catalogue search can return
 * hundreds of rows, and the summarizer needs "searched for X, found Y and Z",
 * not the rows. */
const TOOL_ARGS_CHAR_LIMIT = 300;
const TOOL_RESULT_CHAR_LIMIT = 600;

const SUMMARIZE_SYSTEM_PROMPT =
  'You are an AI assistant compacting your own conversation memory. '
  + 'The user message contains a transcript of the conversation so far. '
  + 'Summarize it for your own future context: preserve every concrete fact, '
  + 'name, number, decision, and open question stated by either side, even '
  + 'ones that seem trivial. Respond with the summary only, no preamble.';

function tokensSinceLatestCompaction(totalTokens, compaction) {
  const total = Math.max(0, Number(totalTokens) || 0);
  const checkpoint = Math.max(0, Number(compaction?.properties?.token_checkpoint) || 0);
  return Math.max(0, total - checkpoint);
}

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
async function maybeCompactConversation(workspace, chatPath, agentProps, modelId, options = {}) {
  // `options.always`: a run compacts when its context is over budget, whether
  // or not the agent opted into threshold-based auto-compaction.
  if (agentProps?.auto_compact !== true && options.always !== true) return null;

  let existing = null;
  try {
    const threshold = Number(agentProps.compact_threshold_messages) > 0
      ? Math.floor(Number(agentProps.compact_threshold_messages))
      : DEFAULT_COMPACT_THRESHOLD;

    /* DESCENDANTS, and every node type a turn is made of.
     *
     * This asked for `CHILD_OF` + `raisin:Message` only. Tool calls and their
     * results are `raisin:AIToolCall` / `raisin:AIToolResult` nodes parented to
     * the assistant message, so the summarizer never saw a single one —
     * compare `history.js`, which has always used `DESCENDANT_OF` and the full
     * list. The originals are DELETED behind the cutoff, so everything the
     * agent had discovered was replaced by a summary that never contained it.
     * An agent that searched the catalogue last turn genuinely could not
     * remember doing it, which is one of the ways it ends up searching again. */
    const rows = await raisin.sql.query(`
      SELECT path, properties, created_at, node_type
      FROM '${workspace}'
      WHERE DESCENDANT_OF($1)
        AND node_type IN ('raisin:Message', 'raisin:AIToolCall', 'raisin:AIToolResult', 'raisin:AIToolSingleCallResult')
      ORDER BY created_at ASC
    `, [chatPath]);
    const all = Array.isArray(rows) ? rows : [];

    // The threshold and the cutoff are about MESSAGES, as they always were.
    const messages = all.filter(
      (n) => n.node_type === 'raisin:Message'
        && n.path.split('/').slice(0, -1).join('/') === chatPath,
    );

    // Tool nodes, indexed by the message they hang under, so a message's own
    // tool work can be folded into its transcript line.
    const toolsByMessage = new Map();
    for (const node of all) {
      if (node.node_type === 'raisin:Message') continue;
      // A result hangs under its call, which hangs under the message.
      const parent = node.path.split('/').slice(0, -1).join('/');
      const owner = node.node_type === 'raisin:AIToolCall'
        ? parent
        : parent.split('/').slice(0, -1).join('/');
      if (!toolsByMessage.has(owner)) toolsByMessage.set(owner, []);
      toolsByMessage.get(owner).push(node);
    }

    existing = await getLatestCompaction(workspace, chatPath);

    // Only messages after the last compaction cutoff count toward the threshold
    let active = messages;
    const prevCutoff = existing?.properties?.cutoff_message_path;
    if (prevCutoff) {
      const idx = messages.findIndex(m => m.path === prevCutoff);
      if (idx >= 0) active = messages.slice(idx + 1);
    }

    const force = options.force === true;
    if (!force && active.length <= threshold) return existing;

    const keepFloor = Number(agentProps.compact_keep_messages) > 0
      ? Math.floor(Number(agentProps.compact_keep_messages))
      : Math.max(2, Math.floor(threshold / 3));
    const desiredKeep = Math.max(keepFloor, Math.floor(active.length / 3));
    const keep = force
      ? Math.min(desiredKeep, Math.max(1, active.length - 1))
      : desiredKeep;
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

      /* WHAT THE TURN ACTUALLY DID.
       *
       * An assistant message that only made a tool call has empty content, so
       * the line above skipped it and a tool-calling round compacted to
       * literally nothing — the most important turns summarised to the least.
       * The call and what came back are the part worth keeping: it is the
       * difference between "I looked that up" and looking it up again. */
      const toolNodes = toolsByMessage.get(m.path) || [];
      const calls = toolNodes.filter((n) => n.node_type === 'raisin:AIToolCall');
      for (const call of calls) {
        const props = call.properties || {};
        const ref = props.function_ref;
        const name = props.function_name
          || (typeof ref === 'object' ? String(ref['raisin:path'] || '').split('/').pop() : ref)
          || 'tool';
        const args = JSON.stringify(stripInternalContext(props.arguments || {}));
        const result = toolNodes.find(
          (n) => n.path.startsWith(`${call.path}/`)
            && (n.node_type === 'raisin:AIToolResult' || n.node_type === 'raisin:AIToolSingleCallResult'),
        );
        const rp = result?.properties || {};
        const outcome = rp.error
          ? `error: ${String(rp.error)}`
          : JSON.stringify(rp.result ?? '');
        lines.push(
          `${role} called ${name}(${args.slice(0, TOOL_ARGS_CHAR_LIMIT)}) -> `
            + String(outcome).slice(0, TOOL_RESULT_CHAR_LIMIT),
        );
      }
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
    /* A run names its compaction after the operation that made it, so a
     * replayed operation cannot write a second one; its structured checkpoint
     * rides along (options.checkpoint). */
    const nodeName = typeof options.nodeName === 'string' && options.nodeName ? options.nodeName : `compaction-${Date.now()}`;
    if (options.nodeName) {
      const prior = await raisin.nodes.get(workspace, `${chatPath}/${nodeName}`);
      if (prior) return prior;
    }
    const node = await raisin.nodes.create(workspace, chatPath, {
      name: nodeName,
      node_type: 'raisin:AICompaction',
      properties: {
        messages_compacted: toCompact.length,
        messages_kept: active.length - toCompact.length,
        summary,
        summary_preview: summary.slice(0, 200),
        cutoff_message_path: cutoff.path,
        cutoff_created_at: cutoff.created_at || cutoff.properties?.created_at || null,
        token_checkpoint: Number(options.tokenCheckpoint) || 0,
        created_at: new Date().toISOString(),
        ...(options.checkpoint && typeof options.checkpoint === 'object' ? { checkpoint: options.checkpoint, run_id: options.checkpoint.run_id || null } : {}),
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
  tokensSinceLatestCompaction,
};
