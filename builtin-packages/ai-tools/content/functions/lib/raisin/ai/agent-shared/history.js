/**
 * Conversation history builder for agent handlers.
 *
 * Uses a single DESCENDANT_OF() SQL query to fetch all messages, tool calls,
 * and tool results in one shot, then resolves parent-child relationships via
 * in-memory Map lookups — O(1) per node instead of 40+ sequential getChildren().
 */

import { log } from './logger.js';

const MAX_HISTORY_MESSAGES = 50;

/**
 * Remove the __raisin_context key from tool arguments before feeding them
 * into the AI history.  This internal metadata is injected at dispatch time
 * and must never leak into model context.
 */
function stripInternalContext(args) {
  if (!args || typeof args !== 'object') return args;
  const cleaned = { ...args };
  delete cleaned.__raisin_context;
  return cleaned;
}

/**
 * Build a chronological conversation history array suitable for
 * chat-completion APIs (OpenAI / Anthropic format).
 *
 * @param {string} workspace       Workspace containing the conversation
 * @param {string} chatPath        Path to the raisin:Conversation node
 * @param {string} systemPrompt    Optional system prompt (prepended as first entry)
 * @param {string|null} currentAssistantMsgPath  Path to the in-flight assistant message
 * @param {Array|null}  aggregatedToolResults    Pre-aggregated results for the current message
 * @param {Object} [options]
 * @param {number} [options.maxHistoryMessages]  Per-agent history window override
 *                 (agent property max_history_messages); defaults to MAX_HISTORY_MESSAGES.
 * @returns {Array} History entries: { role, content, tool_calls?, tool_call_id?, name? }
 */
async function buildHistoryFromChat(workspace, chatPath, systemPrompt, currentAssistantMsgPath = null, aggregatedToolResults = null, options = {}) {
  const maxHistoryMessages = Number(options?.maxHistoryMessages) > 0
    ? Math.floor(Number(options.maxHistoryMessages))
    : MAX_HISTORY_MESSAGES;

  const history = [];

  if (systemPrompt) {
    history.push({ role: 'system', content: systemPrompt });
  }

  /* ── Fetch the descendants this turn can still USE ──────────────────────
   *
   * This asked for every descendant of the chat, every round, with no lower
   * bound — and compaction never removed one, because applying it is an
   * in-memory `slice` of the message array. So the cost of building history
   * grew with the age of the conversation and nothing ever brought it down.
   *
   * Measured on a working Studio Builder chat: `total_nodes=671`,
   * `messages=12`, `duration_ms=9648`. Five to ten seconds PER TOOL ROUND,
   * spent fetching and indexing six hundred nodes in order to use twelve. A
   * twenty-round turn pays two to three minutes for nothing, which reads to
   * the user as an agent that cannot finish.
   *
   * The cutoff is already recorded on the compaction node, so it belongs in
   * the WHERE clause rather than in a filter afterwards. Two queries: a tiny
   * one for the newest compaction, then the window it defines. Older nodes
   * stay in storage — they are the conversation's record and deleting them is
   * a different decision — they are simply not fetched to be discarded. */
  const t0 = log.time();
  const compactionRows = await raisin.sql.query(`
    SELECT properties, created_at
    FROM "${workspace}"
    WHERE CHILD_OF($1) AND node_type = 'raisin:AICompaction'
    ORDER BY created_at DESC
    LIMIT 1
  `, [chatPath]);
  const latestCompaction = Array.isArray(compactionRows) ? compactionRows[0] : null;
  const cutoffAt = latestCompaction?.properties?.cutoff_created_at || null;

  const TYPES = "'raisin:Message', 'raisin:AIToolCall', 'raisin:AIToolResult', 'raisin:AIToolSingleCallResult', 'raisin:AICompaction'";
  const allNodes = cutoffAt
    ? await raisin.sql.query(`
        SELECT path, name, node_type, properties, created_at
        FROM "${workspace}"
        WHERE DESCENDANT_OF($1)
          AND node_type IN (${TYPES})
          AND created_at > $2
        ORDER BY created_at ASC
      `, [chatPath, cutoffAt])
    : await raisin.sql.query(`
        SELECT path, name, node_type, properties, created_at
        FROM "${workspace}"
        WHERE DESCENDANT_OF($1)
          AND node_type IN (${TYPES})
        ORDER BY created_at ASC
      `, [chatPath]);

  /* The compaction node itself sits AT the cutoff, so the window above
   * excludes it — and without it the summary of everything before is lost and
   * the turn starts amnesiac. Put it back. */
  if (cutoffAt && latestCompaction) {
    allNodes.push({
      path: `${chatPath}/__latest_compaction`,
      name: '__latest_compaction',
      node_type: 'raisin:AICompaction',
      properties: latestCompaction.properties,
      created_at: latestCompaction.created_at,
    });
  }

  // ── Build parent→children index ──
  const childrenByParent = new Map();
  for (const node of allNodes) {
    const parentPath = node.path.split('/').slice(0, -1).join('/');
    if (!childrenByParent.has(parentPath)) {
      childrenByParent.set(parentPath, []);
    }
    childrenByParent.get(parentPath).push(node);
  }

  /* A run keeps a user message that is still a QUEUED steer out of context
   * until the runtime consumes it (options.excludeMessagePaths). */
  const excluded = options?.excludeMessagePaths instanceof Set ? options.excludeMessagePaths : null;
  let directMessages = (childrenByParent.get(chatPath) || [])
    .filter(n => n.node_type === 'raisin:Message')
    .filter(n => !excluded || !excluded.has(n.path))
    .sort((a, b) => new Date(a.created_at) - new Date(b.created_at));

  // ── Apply latest compaction (if any): drop summarized messages and
  //    inject the persisted summary as a synthetic system entry ──
  const compactions = (childrenByParent.get(chatPath) || [])
    .filter(n => n.node_type === 'raisin:AICompaction')
    .sort((a, b) =>
      new Date(a.properties?.created_at || a.created_at || 0)
      - new Date(b.properties?.created_at || b.created_at || 0));
  if (compactions.length > 0) {
    const cProps = compactions[compactions.length - 1].properties || {};
    if (cProps.summary && cProps.cutoff_message_path) {
      const cutoffIdx = directMessages.findIndex(m => m.path === cProps.cutoff_message_path);
      let applied = false;
      if (cutoffIdx >= 0) {
        directMessages = directMessages.slice(cutoffIdx + 1);
        applied = true;
      } else if (cProps.cutoff_created_at) {
        directMessages = directMessages.filter(
          m => new Date(m.created_at) > new Date(cProps.cutoff_created_at),
        );
        applied = true;
      }
      if (applied) {
        history.push({
          role: 'system',
          content: `Earlier conversation summary (older messages were compacted):\n${cProps.summary}`,
        });
        /* A run's compaction also stores a STRUCTURED checkpoint: the facts
         * the runtime held at that point, which no summary can drop. */
        if (cProps.checkpoint && typeof cProps.checkpoint === 'object') {
          history.push({
            role: 'system',
            content: `Structured checkpoint at compaction (from the runtime):\n${JSON.stringify(cProps.checkpoint).slice(0, 8000)}`,
          });
        }
        log.debug('history', 'Applied compaction', {
          cutoff: cProps.cutoff_message_path,
          remaining_messages: directMessages.length,
        });
      }
    }
  }

  log.debug('history', 'Queried descendants', {
    total_nodes: allNodes.length,
    messages: directMessages.length,
    duration_ms: log.since(t0),
  });

  // ── Walk messages and assemble history entries ──
  for (const msg of directMessages) {
    const props = msg.properties || {};

    // Extract text content — body can be string or object
    let content;
    if (typeof props.body === 'string') {
      content = props.body;
    } else if (props.body && typeof props.body === 'object') {
      content = props.body.content || props.body.message_text || '';
    } else {
      content = props.content || '';
    }

    const entry = { role: props.role, content };
    const toolResultEntries = [];

    if (props.role === 'assistant') {
      const msgChildren = childrenByParent.get(msg.path) || [];
      const toolCallNodes = msgChildren.filter(c => c.node_type === 'raisin:AIToolCall');

      const isCurrentMsg = msg.path === currentAssistantMsgPath;
      const useAggregated = isCurrentMsg && aggregatedToolResults && aggregatedToolResults.length > 0;

      if (useAggregated) {
        // Fast path: caller already collected results for the in-flight message
        entry.tool_calls = [];
        for (const agg of aggregatedToolResults) {
          const callId = agg.tool_call_id;
          const funcName = agg.function_name || 'unknown';
          const tcNode = toolCallNodes.find(t =>
            t.properties?.tool_call_id === callId || t.id === callId
          );
          entry.tool_calls.push({
            id: callId,
            type: 'function',
            function: {
              name: funcName,
              arguments: JSON.stringify(stripInternalContext(tcNode?.properties?.arguments || {})),
            },
          });
          toolResultEntries.push({
            role: 'tool',
            content: JSON.stringify(agg.result || agg.error || ''),
            tool_call_id: callId,
            name: funcName,
          });
        }
      } else if (toolCallNodes.length > 0) {
        // Standard path: pair each tool call with its result child
        entry.tool_calls = [];
        for (const tc of toolCallNodes) {
          const tcProps = tc.properties || {};
          const tcChildren = childrenByParent.get(tc.path) || [];
          const resultNode = tcChildren.find(r =>
            r.node_type === 'raisin:AIToolResult' || r.node_type === 'raisin:AIToolSingleCallResult'
          );

          // OpenAI requires every tool_call to have a matching tool result —
          // only include calls whose result has already arrived.
          if (!resultNode) continue;

          const funcRef = tcProps.function_ref;
          const funcName = tcProps.function_name
            || (typeof funcRef === 'object'
              ? (funcRef['raisin:path'] || '').split('/').pop()
              : funcRef);

          const callId = tcProps.tool_call_id || tc.id;
          entry.tool_calls.push({
            id: callId,
            type: 'function',
            function: {
              name: funcName,
              arguments: JSON.stringify(stripInternalContext(tcProps.arguments || {})),
            },
          });

          const resProps = resultNode.properties || {};
          toolResultEntries.push({
            role: 'tool',
            content: JSON.stringify(resProps.result || resProps.error || ''),
            tool_call_id: callId,
            name: funcName,
          });
        }
      }

      // Remove empty tool_calls array (some providers reject it)
      if (entry.tool_calls && entry.tool_calls.length === 0) {
        delete entry.tool_calls;
      }
    }

    history.push(entry);
    for (const tr of toolResultEntries) {
      history.push(tr);
    }
  }

  // ── Truncate to keep context window bounded ──
  // Preserve the leading system entries (system prompt + compaction summary).
  let leadingSystem = 0;
  while (leadingSystem < history.length && history[leadingSystem].role === 'system') {
    leadingSystem++;
  }
  if (history.length - leadingSystem > maxHistoryMessages) {
    const head = history.slice(0, leadingSystem);
    const recent = history.slice(history.length - maxHistoryMessages);
    log.debug('history', 'History built', { total_entries: head.length + recent.length, truncated: true });
    return [...head, ...recent];
  }

  log.debug('history', 'History built', { total_entries: history.length, truncated: false });
  return history;
}

export {
  MAX_HISTORY_MESSAGES,
  stripInternalContext,
  buildHistoryFromChat,
};
