/**
 * Handle Inbox Reply to Flow Chat
 *
 * When a user sends a message in an inbox conversation that was created
 * by a flow's chat step, this function resumes the waiting flow instance
 * with the user's message content.
 *
 * Detection: The parent conversation node has a `flow_instance_id` property
 * set by the flow runtime's conversation_persistence module.
 *
 * @param {Object} context - Trigger context
 */
async function handleInboxReply(context) {
  const { event, workspace } = context.flow_input;
  const ACCESS_CONTROL = 'raisin:access_control';

  console.log('[inbox-reply] Trigger fired for:', event.node_path);

  // 1. Get the message node
  const message = await raisin.nodes.get(workspace, event.node_path);
  if (!message) return { success: false, error: 'Message not found' };

  // Skip messages created by the flow itself (prevent loops)
  if (message.properties._source === 'flow') {
    console.log('[inbox-reply] Skipping flow-originated message');
    return { success: true, skipped: true };
  }

  // 2. Get the parent conversation to check for flow_instance_id
  const pathParts = event.node_path.split('/');
  pathParts.pop(); // Remove message name to get conversation path
  const conversationPath = pathParts.join('/');

  const conversation = await raisin.nodes.get(ACCESS_CONTROL, conversationPath);
  if (!conversation) {
    console.log('[inbox-reply] Parent conversation not found:', conversationPath);
    return { success: false, error: 'Parent conversation not found' };
  }

  const flowInstanceId = conversation.properties.flow_instance_id;
  if (!flowInstanceId) {
    // Not a flow-originated conversation, nothing to do
    console.log('[inbox-reply] No flow_instance_id on conversation, skipping');
    return { success: true, skipped: true };
  }

  // 3. Extract user message content
  const content = message.properties.body?.content
    || message.properties.body?.message_text
    || '';

  if (!content) {
    console.log('[inbox-reply] Empty message content, skipping');
    return { success: true, skipped: true };
  }

  // 4. Resume the waiting flow with the user's message
  try {
    console.log('[inbox-reply] Resuming flow:', flowInstanceId, 'with message:', content.substring(0, 100));

    await raisin.flows.resume(flowInstanceId, {
      message: content,
      sender_id: message.properties.sender_id,
      conversation_path: conversationPath,
    });

    console.log('[inbox-reply] Flow resumed successfully:', flowInstanceId);
    return { success: true };
  } catch (err) {
    console.error('[inbox-reply] Failed to resume flow:', err);
    return { success: false, error: String(err?.message || err) };
  }
}
