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
 * @returns {Array} History entries: { role, content, tool_calls?, tool_call_id?, name? }
 */
async function buildHistoryFromChat(workspace, chatPath, systemPrompt, currentAssistantMsgPath = null, aggregatedToolResults = null) {
  const history = [];

  if (systemPrompt) {
    history.push({ role: 'system', content: systemPrompt });
  }

  // ── Fetch all relevant descendants in a single query ──
  const t0 = log.time();
  const allNodes = await raisin.sql.query(`
    SELECT path, name, node_type, properties, created_at
    FROM "${workspace}"
    WHERE DESCENDANT_OF($1)
      AND node_type IN ('raisin:Message', 'raisin:AIToolCall', 'raisin:AIToolResult', 'raisin:AIToolSingleCallResult')
    ORDER BY created_at ASC
  `, [chatPath]);

  // ── Build parent→children index ──
  const childrenByParent = new Map();
  for (const node of allNodes) {
    const parentPath = node.path.split('/').slice(0, -1).join('/');
    if (!childrenByParent.has(parentPath)) {
      childrenByParent.set(parentPath, []);
    }
    childrenByParent.get(parentPath).push(node);
  }

  const directMessages = (childrenByParent.get(chatPath) || [])
    .filter(n => n.node_type === 'raisin:Message')
    .sort((a, b) => new Date(a.created_at) - new Date(b.created_at));

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
  if (history.length > MAX_HISTORY_MESSAGES + 1) {
    const systemMsg = history[0]?.role === 'system' ? history[0] : null;
    const recent = history.slice(-MAX_HISTORY_MESSAGES);
    log.debug('history', 'History built', { total_entries: recent.length + (systemMsg ? 1 : 0), truncated: true });
    return systemMsg ? [systemMsg, ...recent] : recent;
  }

  log.debug('history', 'History built', { total_entries: history.length, truncated: false });
  return history;
}

export {
  MAX_HISTORY_MESSAGES,
  stripInternalContext,
  buildHistoryFromChat,
};
