/**
 * Handle Direct Message
 *
 * Triggered when a user sends a direct message.
 * Verifies that sender and recipient are friends before delivering.
 * 
 * New Architecture:
 * - Delivery (Transaction): Creates message and conversation in Recipient's Inbox.
 * - Archive (Transaction): Moves original message to Sender's Conversation bucket.
 *
 * @param {Object} context - Trigger context
 */
async function handleDirectMessage(context) {
  const { event, workspace } = context.flow_input;
  const ACCESS_CONTROL = 'raisin:access_control';

  console.log('[direct-message] ========== TRIGGER START ==========');
  console.log('[direct-message] Trigger fired for:', event.node_path);
  console.log('[direct-message] Event:', JSON.stringify(event, null, 2));

  // 1. Get the message (Initial read)
  const message = await raisin.nodes.get(workspace, event.node_path);
  if (!message) return { success: false, error: 'Message not found' };

  const { body, sender_id } = message.properties;
  const recipientEmail = body?.recipient_email;
  const recipientPathArg = body?.recipient_path;
  const senderPath = body?.sender_path;

  if (!senderPath) {
    await raisin.nodes.update(workspace, event.node_path, { properties: { ...message.properties, status: 'error', error: 'Missing senderPath' } });
    return { success: false, error: 'Missing sender path' };
  }

  let tx = null;
  let txFinalized = false;

  try {
    tx = await raisin.nodes.beginTransaction();

    // 2. Find recipient
    let recipient = null;
    if (recipientEmail) {
        const recipientResult = await raisin.sql.query(`SELECT id, path FROM '${ACCESS_CONTROL}' WHERE node_type = 'raisin:User' AND properties->>'email' LIKE $1`, [recipientEmail]);
        const rows = Array.isArray(recipientResult) ? recipientResult : (recipientResult?.rows || []);
        if (rows.length > 0) recipient = rows[0];
    }
    if (!recipient && recipientPathArg) {
        recipient = await tx.getByPath(ACCESS_CONTROL, recipientPathArg);
    }

    if (!recipient) {
      await tx.update(workspace, event.node_path, { properties: { ...message.properties, status: 'error', error: 'Recipient not found' } });
      await tx.commit();
      txFinalized = true;
      return { success: false, error: 'Recipient not found' };
    }

    const recipientPath = recipient.path;
    const finalRecipientEmail = recipientEmail || recipient.properties?.email;
    const recipientDisplayName = recipient.properties?.display_name || finalRecipientEmail?.split('@')[0] || 'User';

    // 3. Verify friendship
    const friendshipCheck = await raisin.sql.query(`SELECT * FROM GRAPH_TABLE(MATCH (sender)-[:FRIENDS_WITH]->(recipient) WHERE sender.path = '${senderPath}' AND recipient.path = '${recipientPath}' COLUMNS (recipient.id AS id)) AS g LIMIT 1`);
    const friendshipRows = Array.isArray(friendshipCheck) ? friendshipCheck : (friendshipCheck?.rows || []);
    if (!friendshipRows.length) {
      await tx.update(workspace, event.node_path, { properties: { ...message.properties, status: 'error', error: 'Not friends' } });
      await tx.commit();
      txFinalized = true;
      return { success: false, error: 'Friendship verification failed' };
    }

    // 4. Generate IDs
    const sorted = [senderPath, recipientPath].sort();
    const combined = sorted.join(':');
    let hash = 0;
    for (let i = 0; i < combined.length; i++) {
      const char = combined.charCodeAt(i);
      hash = ((hash << 5) - hash) + char;
      hash = hash & hash;
    }
    const conversationId = message.properties.conversation_id || `conv-${Math.abs(hash).toString(36)}`;

    const recipientChatsPath = `${recipientPath}/inbox/chats`;
    const conversationPath = `${recipientChatsPath}/${conversationId}`;
    const senderConversationPath = `${senderPath}/inbox/chats/${conversationId}`;
    const senderMessagePath = `${senderConversationPath}/${message.name}`;

    // Build participant_details for both sides
    const participantDetails = {
        [senderPath]: {
            email: body.sender_email,
            display_name: body.sender_display_name || body.sender_email?.split('@')[0] || 'User'
        },
        [recipientPath]: {
            email: finalRecipientEmail,
            display_name: recipientDisplayName
        }
    };

    // --- RECIPIENT SIDE ---
    console.log('[direct-message] Processing RECIPIENT side:', {
        recipientPath,
        recipientEmail: finalRecipientEmail,
        conversationPath,
        senderPath
    });

    // Check if conversation exists - be defensive about the check
    let conversationNode = null;
    try {
        conversationNode = await tx.getByPath(ACCESS_CONTROL, conversationPath);
        console.log('[direct-message] getByPath result:', conversationNode ? 'found' : 'null');
    } catch(e) {
        // getByPath might throw if not found
        console.log('[direct-message] getByPath threw:', e.message);
        conversationNode = null;
    }

    // Create conversation if it doesn't exist (check for actual node with id/path)
    if (!conversationNode || !conversationNode.id) {
        console.log('[direct-message] Creating new conversation:', conversationPath);
        conversationNode = await tx.createDeep(ACCESS_CONTROL, recipientChatsPath, {
            name: conversationId,
            node_type: 'launchpad:Conversation',
            properties: {
                subject: body.subject || 'Direct Message',
                participants: message.properties.participant_paths || sorted,
                participant_details: participantDetails,
                unread_count: 0
            }
        });
        console.log('[direct-message] Created conversation:', conversationNode?.path);
    } else {
        console.log('[direct-message] Conversation exists:', conversationNode.path);
    }

    // Create message as child of conversation
    console.log('[direct-message] Creating message in:', conversationPath);
    const recipientMsg = await tx.createDeep(ACCESS_CONTROL, conversationPath, {
      name: `msg-${Date.now()}`,
      node_type: 'raisin:Message',
      properties: { 
          ...message.properties, 
          status: 'delivered',
          sender_message_path: senderMessagePath 
      }
    });
    console.log('[direct-message] Created recipient message:', recipientMsg?.path);

    // DEFENSIVE: Don't rely on createDeep return value having properties
    // Build update properties explicitly from known values
    const currentUnread = ((conversationNode.properties || {}).unread_count || 0) + 1;
    await tx.update(ACCESS_CONTROL, conversationPath, {
        properties: {
            subject: (conversationNode.properties || {}).subject || body.subject || 'Direct Message',
            participants: (conversationNode.properties || {}).participants || message.properties.participant_paths || sorted,
            participant_details: participantDetails,
            last_message: {
                content: body.content,
                sender_display_name: body.sender_display_name || body.sender_email,
                sender_email: body.sender_email,
                sender_path: senderPath,
                recipient_email: finalRecipientEmail,
                recipient_path: recipientPath,
                recipient_display_name: recipientDisplayName,
                created_at: new Date().toISOString()
            },
            unread_count: currentUnread
        }
    });

    // Create notification (Always notify, frontend handles suppression)
    await tx.createDeep(ACCESS_CONTROL, `${recipientPath}/inbox/notifications`, {
        name: `notif-${Date.now()}`,
        node_type: 'launchpad:Notification',
        properties: {
            type: 'message',
            title: `New message from ${body.sender_display_name || 'Friend'}`,
            body: body.content?.substring(0, 50) || 'New message',
            link: conversationPath,
            read: false
        }
    });

    // --- SENDER SIDE ---
    let senderConv = null;
    try {
        senderConv = await tx.getByPath(ACCESS_CONTROL, senderConversationPath);
    } catch(e) {
        senderConv = null;
    }

    if (!senderConv || !senderConv.id) {
        console.log('[direct-message] Creating sender conversation:', senderConversationPath);
        senderConv = await tx.createDeep(ACCESS_CONTROL, `${senderPath}/inbox/chats`, {
            name: conversationId,
            node_type: 'launchpad:Conversation',
            properties: {
                subject: body.subject || 'Direct Message',
                participants: message.properties.participant_paths || sorted,
                participant_details: participantDetails,
                unread_count: 0
            }
        });
        console.log('[direct-message] Created sender conversation:', senderConv?.path);
    }

    // "Move" message from outbox to sender's conversation
    // Note: We delete the original and create a copy (new ID) since tx.move() isn't available
    // and reusing the same ID within a transaction causes conflicts
    await tx.delete(workspace, event.node_path);
    await tx.createDeep(ACCESS_CONTROL, senderConversationPath, {
        name: message.name,
        node_type: 'raisin:Message',
        properties: { ...message.properties, status: 'delivered' }
    });

    // Update Sender's Conversation snippet (include full info for both participants)
    // DEFENSIVE: Don't rely on createDeep return value having properties
    await tx.update(ACCESS_CONTROL, senderConversationPath, {
        properties: {
            subject: (senderConv.properties || {}).subject || body.subject || 'Direct Message',
            participants: (senderConv.properties || {}).participants || message.properties.participant_paths || sorted,
            participant_details: participantDetails,
            unread_count: (senderConv.properties || {}).unread_count || 0,
            last_message: {
                content: body.content,
                sender_display_name: body.sender_display_name || body.sender_email,
                sender_email: body.sender_email,
                sender_path: senderPath,
                recipient_email: finalRecipientEmail,
                recipient_path: recipientPath,
                recipient_display_name: recipientDisplayName,
                created_at: new Date().toISOString()
            }
        }
    });

    console.log('[direct-message] Committing transaction...');
    await tx.commit();
    txFinalized = true;
    console.log('[direct-message] ========== TRIGGER SUCCESS ==========');
    return { success: true };

  } catch (err) {
    console.error('[direct-message] Error:', err);

    // Only try to rollback if transaction exists and hasn't been finalized
    if (tx && !txFinalized) {
      try { await tx.rollback(); txFinalized = true; } catch(e) {
        console.error('[direct-message] Rollback failed:', e);
      }
    }

    // Attempt final global update for error
    // Note: This may fail if the node was moved during the transaction
    try {
      await raisin.nodes.update(workspace, event.node_path, {
          properties: { ...message.properties, status: 'error', error: String(err?.message || err) }
      });
    } catch(e) {
      console.error('[direct-message] Failed to update error status:', e);
    }

    return { success: false, error: String(err?.message || err) };
  }
}
