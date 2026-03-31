/**
 * User memory loading for agent handlers.
 *
 * Each agent stores per-user memory as a node at
 *   /agents/{agentName}/memory/{sanitized_user_id}
 * with either a markdown `content` property (current) or a legacy
 * `entries` array.  The loaded content is appended to the system prompt
 * so the AI retains user-specific context across conversations.
 */

import { log } from './logger.js';

/**
 * Load stored memory for a user from an agent's memory store.
 *
 * @param {string} agentName  Name segment of the agent (e.g. "sample-assistant")
 * @param {string} userId     User identifier
 * @returns {string} Memory content (markdown) or empty string
 */
async function loadUserMemory(agentName, userId) {
  log.debug('memory', 'Loading user memory', { agent: agentName, user: userId });
  if (!agentName || !userId) return '';

  const safeName = userId.replace(/[^a-zA-Z0-9_-]/g, '_');
  const contextPath = `/agents/${agentName}/memory/${safeName}`;

  try {
    const contextNode = await raisin.nodes.get('ai', contextPath);
    if (!contextNode) return '';

    // Preferred format: markdown string
    const rawContent = contextNode.properties?.content;
    if (typeof rawContent === 'string' && rawContent.trim()) {
      log.debug('memory', 'User memory loaded', { content_length: rawContent.trim().length });
      return rawContent.trim();
    }

    // Legacy format: array of { key, value } entries
    if (Array.isArray(contextNode.properties?.entries)) {
      const entries = contextNode.properties.entries;
      log.debug('memory', 'User memory loaded (legacy)', { entry_count: entries.length });
      return entries
        .filter(e => e.key)
        .map(e => `- ${e.key}: ${e.value || ''}`)
        .join('\n');
    }
  } catch (_) {
    // Node doesn't exist — not an error
  }

  log.debug('memory', 'No user memory found');
  return '';
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
