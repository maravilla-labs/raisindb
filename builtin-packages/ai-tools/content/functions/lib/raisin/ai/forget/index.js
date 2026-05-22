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

  // "Nothing to forget" is not an error — return a graceful, idempotent result
  // so the model treats it as done instead of retrying in a loop.
  if (!contextNode) {
    return { success: true, found: false, key, message: 'No stored memory yet — nothing to forget.' };
  }

  // Read current content
  const rawContent = contextNode.properties?.content;
  if (typeof rawContent !== 'string' || !rawContent.trim()) {
    return { success: true, found: false, key, message: 'Memory is empty — nothing to forget.' };
  }

  const lines = rawContent.split('\n');

  // Match lines like "- key: ..." (with optional whitespace)
  const escapedKey = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const pattern = new RegExp(`^-\\s+${escapedKey}\\s*:`);
  const filtered = lines.filter(line => !pattern.test(line));
  const found = filtered.length < lines.length;

  if (!found) {
    return { success: true, found: false, key, message: `No memory entry for "${key}" — nothing to forget.` };
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
