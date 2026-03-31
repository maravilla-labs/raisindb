/**
 * forget — Removes a specific memory entry by key from the user's stored memory.
 *
 * Matches bullet list entries formatted as "- key: value" and removes them.
 * The updated markdown is saved back.
 *
 * Execution mode: async
 */
async function handler(input) {
  const { key, __raisin_context } = input;
  const agentName = __raisin_context?.agent_name;
  const userId = __raisin_context?.sender_id;

  if (!key) throw new Error('Key is required');
  if (!agentName || !userId) {
    throw new Error('Missing agent_name or sender_id in execution context');
  }

  const workspace = 'ai';
  const safeName = userId.replace(/[^a-zA-Z0-9_-]/g, '_');
  const contextNodePath = `/agents/${agentName}/memory/${safeName}`;

  let contextNode = null;
  try {
    contextNode = await raisin.nodes.get(workspace, contextNodePath);
  } catch (_) {
    // Doesn't exist
  }

  if (!contextNode) {
    throw new Error('No stored memory found for this user');
  }

  // Read current content
  const rawContent = contextNode.properties?.content;
  if (typeof rawContent !== 'string' || !rawContent.trim()) {
    throw new Error('Memory is empty');
  }

  const lines = rawContent.split('\n');

  // Match lines like "- key: ..." (with optional whitespace)
  const escapedKey = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const pattern = new RegExp(`^-\\s+${escapedKey}\\s*:`);
  const filtered = lines.filter(line => !pattern.test(line));
  const found = filtered.length < lines.length;

  if (!found) {
    throw new Error(`Key "${key}" not found in memory`);
  }

  await raisin.nodes.update(workspace, contextNodePath, {
    properties: {
      content: filtered.join('\n'),
      updated_at: new Date().toISOString(),
    },
  });

  console.log(`[forget] Removed key="${key}" for user=${userId} agent=${agentName}`);
  return { success: true, key, found: true };
}
