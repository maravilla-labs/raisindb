/**
 * Handle Task Completed
 *
 * Triggered when a user completes a kanban task (moves card to Done).
 * Creates an AI conversation in the user's ai-chats folder so it
 * appears in the existing AIChatPopup — the AI agent behaves like
 * a regular chat participant.
 *
 * Flow:
 * 1. Frontend writes task_completed message to user's outbox
 * 2. This trigger fires
 * 3. We create an raisin:AIConversation + raisin:AIMessage under
 *    the user's /ai-chats/ path
 * 4. The AIChatPopup picks up the new conversation via real-time events
 *
 * @param {Object} context - Trigger context
 */
async function handleTaskCompleted(context) {
  const { event, workspace } = context.flow_input;
  const ACCESS_CONTROL = 'raisin:access_control';

  console.log('[task-completed] Trigger fired for:', event.node_path);

  // 1. Get the outbox message
  const message = await raisin.nodes.get(workspace, event.node_path);
  if (!message) return { success: false, error: 'Message not found' };

  const { body, sender_id } = message.properties;
  const cardTitle = body?.card_title;
  const cardDescription = body?.card_description || '';
  const boardTitle = body?.board_title || 'board';
  const senderPath = body?.sender_path;

  if (!senderPath || !cardTitle) {
    await raisin.nodes.update(workspace, event.node_path, {
      properties: { ...message.properties, status: 'error', error: 'Missing required fields' }
    });
    return { success: false, error: 'Missing sender_path or card_title' };
  }

  let tx = null;
  let txFinalized = false;

  try {
    tx = await raisin.nodes.beginTransaction();

    // 2. Build the AI message content
    const aiContent = cardDescription
      ? `Task "${cardTitle}" has been completed! Here's a quick summary:\n\n` +
        `**${cardTitle}**\n${cardDescription}\n\n` +
        `Great work on finishing this task on the "${boardTitle}" board.`
      : `Task "${cardTitle}" on the "${boardTitle}" board has been marked as done. Great work!`;

    // 3. Create AI conversation in user's ai-chats folder
    //    This matches the format expected by ai-chat.ts store
    const convName = `task-done-${Date.now()}`;
    const aiChatsPath = `${senderPath}/ai-chats`;
    const conversationPath = `${aiChatsPath}/${convName}`;
    const now = new Date().toISOString();

    // Create raisin:AIConversation node (matching ai-chat.ts expectations)
    await tx.createDeep(ACCESS_CONTROL, aiChatsPath, {
      name: convName,
      node_type: 'raisin:AIConversation',
      properties: {
        title: `Task Complete: ${cardTitle}`,
        status: 'active',
        agent_ref: {
          'raisin:ref': '',
          'raisin:workspace': 'functions',
          'raisin:path': '/agents/sample-assistant',
        },
        message_count: 1,
        trigger_source: 'task_completed',
        board_path: body?.board_path || '',
      },
    });

    // 4. Create raisin:AIMessage as child of conversation
    await tx.createDeep(ACCESS_CONTROL, conversationPath, {
      name: `msg-${Date.now()}`,
      node_type: 'raisin:AIMessage',
      properties: {
        role: 'assistant',
        content: aiContent,
        finish_reason: 'stop',
      },
    });

    // 5. Mark the original outbox message as processed
    await tx.delete(workspace, event.node_path);

    await tx.commit();
    txFinalized = true;

    console.log('[task-completed] AI conversation created at:', conversationPath);
    return { success: true };

  } catch (err) {
    console.error('[task-completed] Error:', err);

    if (tx && !txFinalized) {
      try { await tx.rollback(); txFinalized = true; } catch(e) {
        console.error('[task-completed] Rollback failed:', e);
      }
    }

    try {
      await raisin.nodes.update(workspace, event.node_path, {
        properties: { ...message.properties, status: 'error', error: String(err?.message || err) }
      });
    } catch(e) {
      console.error('[task-completed] Failed to update error status:', e);
    }

    return { success: false, error: String(err?.message || err) };
  }
}
