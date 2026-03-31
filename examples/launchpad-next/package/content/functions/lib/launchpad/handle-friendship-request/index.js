/**
 * Handle Friendship Request
 *
 * Triggered when a friendship_request message is created in a user's outbox.
 * Finds the recipient by email and creates a message in their inbox.
 */
async function handleFriendshipRequest(context) {
  const { event, workspace } = context.flow_input;
  const ACCESS_CONTROL = 'raisin:access_control';

  console.log('[friendship] Trigger fired for:', event.node_path);

  // 1. Get the message node
  const message = await raisin.nodes.get(workspace, event.node_path);
  if (!message) return { success: false, error: 'Message not found' };

  const recipientEmail = message.properties.body?.recipient_email;
  if (!recipientEmail) return { success: false, error: 'Missing recipient_email' };

  try {
    // 2. Find recipient
    const recipientResult = await raisin.sql.query(`
        SELECT id, path FROM '${ACCESS_CONTROL}'
        WHERE node_type = 'raisin:User' AND properties->>'email' LIKE $1
    `, [recipientEmail]);

    const rows = Array.isArray(recipientResult) ? recipientResult : (recipientResult?.rows || []);
    if (!rows.length) {
        await raisin.nodes.update(workspace, event.node_path, {
            properties: { ...message.properties, status: 'error', error: `User not found: ${recipientEmail}` }
        });
        return { success: false, error: 'User not found' };
    }

    const recipient = rows[0];
    const recipientPath = recipient.path;
    const senderPath = event.node_path.split('/outbox/')[0]; // "/users/abc123"

    // --- PHASE 1: DELIVERY (Transaction) ---
    const tx = await raisin.nodes.beginTransaction();
    let inboxMessagePath = "";
    try {
        const inboxMessage = await tx.createDeep(ACCESS_CONTROL, `${recipientPath}/inbox/requests`, {
          name: `friend-req-${Date.now()}`,
          node_type: 'raisin:Message',
          properties: {
            ...message.properties,
            body: {
              ...message.properties.body,
              sender_path: senderPath  // Required for accept/decline to work
            },
            status: 'delivered'
          }
        });
        inboxMessagePath = inboxMessage.path;

        await tx.createDeep(ACCESS_CONTROL, `${recipientPath}/inbox/notifications`, {
            name: `notif-req-${Date.now()}`,
            node_type: 'launchpad:Notification',
            properties: {
                type: 'friendship_request',
                title: `New friend request from ${message.properties.body?.sender_display_name || 'User'}`,
                body: message.properties.body?.message || 'Wants to be your friend',
                link: inboxMessage.path,
                read: false
            }
        });

        await tx.commit();
    } catch (e) {
        await tx.rollback();
        throw e;
    }

    // --- PHASE 2: ARCHIVE (Global API) ---
    const sentPath = `${senderPath}/sent`;
    
    // Update status BEFORE moving
    await raisin.nodes.update(workspace, event.node_path, {
        properties: { ...message.properties, status: 'sent', recipient_id: recipient.id }
    });

    // Move to /sent bucket
    await raisin.nodes.move(workspace, event.node_path, sentPath);

    return { success: true, inbox_message_path: inboxMessagePath };

  } catch (err) {
    console.error('[friendship] Global Error:', err);
    try {
      await raisin.nodes.update(workspace, event.node_path, {
          properties: { ...message.properties, status: 'error', error: err.message }
      });
    } catch(e) {}
    return { success: false, error: err.message };
  }
}
