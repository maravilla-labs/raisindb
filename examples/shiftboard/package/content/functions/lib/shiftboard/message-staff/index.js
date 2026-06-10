/**
 * message-staff - send a chat message to a user (staff member or the manager)
 * addressed by email, FROM the shift-planner agent.
 *
 * The message is dropped into the agent's outbox in the `ai` workspace,
 * exactly mirroring the shape the agent-handler uses for its own replies
 * (see ai-tools agent-shared/outbox.js sendAgentOutboxMessage). The builtin
 * process-chat trigger picks it up and delivers it into the recipient's
 * inbox conversation - creating the user-side raisin:Conversation if needed.
 * When the user replies, the reply is delivered into the agent's inbox and
 * the agent-handler runs again in that conversation thread.
 *
 * Input:  { staff_email: string, message: string }
 *         (__raisin_context is injected by the agent-handler)
 * Output: { delivered_to, conversation_id }
 */
async function handler(input) {
  const { staff_email, message } = input || {};
  const ctx = (input && input.__raisin_context) || {};
  if (!staff_email || !message) {
    throw new Error('staff_email and message are both required');
  }

  const agentName = ctx.agent_name || 'shift-planner';
  const agentHomePath = '/agents/' + agentName;

  // 1. Resolve the recipient user by email in the access-control workspace.
  //    raisin.sql.query returns the row array directly in the function runtime.
  const users = await raisin.sql.query(
    "SELECT id, path, properties FROM 'raisin:access_control' " +
      "WHERE node_type = 'raisin:User' AND properties->>'email'::String = $1 LIMIT 1",
    [staff_email],
  );
  if (!users.length) {
    // Graceful error so the agent can pick another candidate instead of failing
    return {
      error:
        'No chat user found with email ' + staff_email +
        ' - this person is not reachable by chat. Pick another available candidate.',
    };
  }
  const user = users[0];
  // Conversations/participants use the raisin:User NODE id (see handle-chat).
  const userId = user.id;
  const userPath = user.path;

  // 2. Agent identity from its home folder in the `ai` workspace.
  const home = await raisin.nodes.get('ai', agentHomePath);
  const homeProps = (home && home.properties) || {};
  const agentUserId = homeProps.user_id || 'agent:' + agentName;
  const agentDisplayName = homeProps.display_name || agentName;

  // 3. Reuse an existing agent<->user conversation if one exists, so the
  //    whole exchange with one person stays in a single thread.
  //    Prefer the conversation this turn is RUNNING IN when the recipient is
  //    one of its participants (e.g. confirming back to the manager who asked
  //    here) - otherwise the reply lands in some older thread.
  let conversationId = null;
  if (ctx.conversation_path) {
    const current = await raisin.nodes.get('ai', ctx.conversation_path);
    const currentProps = (current && current.properties) || {};
    const currentParticipants = currentProps.participants || [];
    if (currentParticipants.indexOf(userId) !== -1) {
      conversationId =
        currentProps.conversation_id || ctx.conversation_path.split('/').pop();
    }
  }
  if (!conversationId) {
    // Most recently active thread with this user wins.
    const convs = await raisin.sql.query(
      "SELECT path, properties FROM 'ai' " +
        "WHERE CHILD_OF('" + agentHomePath + "/inbox/chats') " +
        "AND node_type = 'raisin:Conversation' " +
        "ORDER BY updated_at DESC",
      [],
    );
    for (const conv of convs) {
      const participants = (conv.properties || {}).participants || [];
      if (participants.indexOf(userId) !== -1) {
        conversationId =
          (conv.properties || {}).conversation_id || conv.path.split('/').pop();
        break;
      }
    }
  }
  if (!conversationId) {
    conversationId = 'chat-' + (await raisin.crypto.uuid());
  }

  // 4. Drop the message into the agent's outbox - the process-chat trigger
  //    delivers it to the user (and mirrors it into both conversations).
  const slug = 'msg-' + Date.now() + '-' + Math.floor(Math.random() * 100000);
  await raisin.nodes.create('ai', agentHomePath + '/outbox', {
    slug,
    name: slug,
    node_type: 'raisin:Message',
    properties: {
      role: 'assistant',
      message_type: 'chat',
      status: 'pending',
      sender_id: agentUserId,
      sender_path: agentHomePath,
      recipient_id: userId,
      recipient_path: userPath,
      created_at: new Date().toISOString(),
      body: {
        message_text: message,
        content: message,
        thread_id: conversationId,
        sender_display_name: agentDisplayName,
      },
      conversation_id: conversationId,
      data: {},
    },
  });

  console.log('[message-staff] queued message to', staff_email, 'conversation', conversationId);
  return { delivered_to: staff_email, conversation_id: conversationId };
}
