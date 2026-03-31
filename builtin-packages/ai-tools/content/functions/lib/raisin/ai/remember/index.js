/**
 * remember — Saves the user's memory as a complete markdown document.
 *
 * Stores the content at /agents/{agentName}/memory/{userId} in the AI workspace.
 * The caller should first read existing memory with read-user-context, merge in
 * new information, and pass the full updated markdown here.
 *
 * Execution mode: async
 */
async function handler(input) {
  const { content, __raisin_context } = input;
  const agentName = __raisin_context?.agent_name;
  const userId = __raisin_context?.sender_id;

  if (!content || typeof content !== 'string' || !content.trim()) {
    throw new Error('Non-empty markdown content is required');
  }
  if (!agentName || !userId) {
    throw new Error('Missing agent_name or sender_id in execution context');
  }

  const workspace = 'ai';
  const safeName = userId.replace(/[^a-zA-Z0-9_-]/g, '_');
  const memoryPath = `/agents/${agentName}/memory`;
  const contextNodePath = `${memoryPath}/${safeName}`;
  const now = new Date().toISOString();

  // Ensure the memory folder exists
  await ensureFolder(workspace, `/agents/${agentName}`, 'memory', 'User Memory');

  // Upsert the context node
  let existing = null;
  try {
    existing = await raisin.nodes.get(workspace, contextNodePath);
  } catch (_) {
    // Node doesn't exist yet
  }

  if (existing) {
    await raisin.nodes.update(workspace, contextNodePath, {
      properties: {
        content: content.trim(),
        updated_at: now,
      },
    });
  } else {
    await raisin.nodes.create(workspace, memoryPath, {
      name: safeName,
      node_type: 'raisin:AgentUserContext',
      properties: {
        user_id: userId,
        content: content.trim(),
        updated_at: now,
      },
    });
  }

  console.log(`[remember] Saved memory for user=${userId} agent=${agentName} (${content.length} chars)`);
  return { success: true };
}

async function ensureFolder(workspace, parentPath, folderName, title) {
  const folderPath = `${parentPath}/${folderName}`;
  try {
    const node = await raisin.nodes.get(workspace, folderPath);
    if (node) return;
  } catch (_) {
    // Doesn't exist, create it
  }
  try {
    await raisin.nodes.create(workspace, parentPath, {
      name: folderName,
      node_type: 'raisin:Folder',
      properties: { title },
    });
  } catch (err) {
    // Race condition — another call may have created it already
    console.log(`[remember] Folder creation note: ${err.message}`);
  }
}
