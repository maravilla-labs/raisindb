/**
 * read-user-context — Reads the stored memory for the current user as markdown.
 *
 * Returns the content from /agents/{agentName}/memory/{userId}.
 * Call this before using remember so you can merge new information.
 *
 * Execution mode: async
 */
async function handler(input) {
  const { __raisin_context } = input;
  const agentName = __raisin_context?.agent_name;
  const userId = __raisin_context?.sender_id;

  if (!agentName || !userId) {
    throw new Error('Missing agent_name or sender_id in execution context');
  }

  const workspace = 'ai';
  const safeName = userId.replace(/[^a-zA-Z0-9_-]/g, '_');
  const contextNodePath = `/agents/${agentName}/memory/${safeName}`;

  try {
    const contextNode = await raisin.nodes.get(workspace, contextNodePath);
    if (!contextNode) {
      return { content: '' };
    }

    const rawContent = contextNode.properties?.content;
    if (typeof rawContent === 'string' && rawContent.trim()) {
      return { content: rawContent };
    }

    return { content: '' };
  } catch (err) {
    // Node not found is expected for new users
    if (String(err?.message || '').includes('not found')) {
      return { content: '' };
    }
    throw err;
  }
}
