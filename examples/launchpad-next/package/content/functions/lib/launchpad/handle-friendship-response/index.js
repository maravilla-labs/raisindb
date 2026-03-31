/**
 * Handle Friendship Response
 *
 * Triggered when a user responds to a friendship request (accept/decline).
 * When accepted, creates bidirectional FRIENDS_WITH graph edges.
 */
async function handleFriendshipResponse(context) {
  const { event, workspace } = context.flow_input;
  const ACCESS_CONTROL = 'raisin:access_control';

  console.log('[friendship-response] Trigger fired for:', event.node_path);

  const tx = await raisin.nodes.beginTransaction();
  try {
    // 1. Get the response message
    const responseMessage = await tx.getByPath(workspace, event.node_path);
    if (!responseMessage) {
      console.error('[friendship-response] Response message not found:', event.node_path);
      await tx.rollback();
      return { success: false, error: 'Response message not found' };
    }

    console.log('[friendship-response] Processing response:', JSON.stringify(responseMessage.properties, null, 2));

    const { body } = responseMessage.properties;
    const response = body?.response; 
    const originalRequestPath = body?.original_request_path;
    const senderPath = body?.sender_path; 
    const responderPath = body?.responder_path; 

    if (!response || !originalRequestPath || !senderPath || !responderPath) {
      console.error('[friendship-response] Missing required fields in body');
      await tx.update(workspace, event.node_path, {
        properties: { ...responseMessage.properties, status: 'error', error: 'Missing fields' }
      });
      await tx.commit();
      return { success: false, error: 'Missing required fields in response body' };
    }

    if (response === 'accepted') {
      console.log('[friendship-response] Creating friendship between:', senderPath, 'and', responderPath);

      // Create bidirectional FRIENDS_WITH edges
      await raisin.sql.execute(`
        RELATE
          FROM path='${senderPath}' IN WORKSPACE '${ACCESS_CONTROL}'
          TO path='${responderPath}' IN WORKSPACE '${ACCESS_CONTROL}'
          TYPE 'FRIENDS_WITH'
      `);

      await raisin.sql.execute(`
        RELATE
          FROM path='${responderPath}' IN WORKSPACE '${ACCESS_CONTROL}'
          TO path='${senderPath}' IN WORKSPACE '${ACCESS_CONTROL}'
          TYPE 'FRIENDS_WITH'
      `);

      console.log('[friendship-response] Friendship edges created');

      // Update original request status to 'accepted'
      const originalRequest = await tx.getByPath(ACCESS_CONTROL, originalRequestPath);
      if (originalRequest) {
        await tx.update(ACCESS_CONTROL, originalRequestPath, {
          properties: {
            ...originalRequest.properties,
            status: 'accepted'
          }
        });
      }

      // Create notification in requester's inbox
      const senderInboxPath = senderPath + '/inbox';
      // Use createDeep for safety
      await tx.createDeep(ACCESS_CONTROL, senderInboxPath, {
        name: `friend-accepted-${Date.now()}`,
        node_type: 'raisin:Message',
        properties: {
          message_type: 'notification',
          subject: 'Friend Request Accepted',
          body: {
            notification_type: 'friendship_accepted',
            friend_email: body.responder_email,
            friend_display_name: body.responder_display_name,
            message: `${body.responder_display_name || body.responder_email} accepted your friend request!`
          },
          status: 'delivered',
          created_at: new Date().toISOString()
        }
      });

      // "Move" from /outbox to /sent (delete original, create copy at new location)
      const sentPath = responderPath + '/sent';
      await tx.delete(workspace, event.node_path);
      await tx.createDeep(ACCESS_CONTROL, sentPath, {
        name: responseMessage.name,
        node_type: responseMessage.node_type,
        properties: { ...responseMessage.properties, status: 'sent' }
      });

      await tx.commit();
      return { success: true, friendship_created: true };

    } else if (response === 'declined') {
      console.log('[friendship-response] Friendship declined');

      // Update original request status to 'declined'
      const originalRequest = await tx.getByPath(ACCESS_CONTROL, originalRequestPath);
      if (originalRequest) {
        await tx.update(ACCESS_CONTROL, originalRequestPath, {
          properties: {
            ...originalRequest.properties,
            status: 'declined'
          }
        });
      }

      // "Move" to /sent (delete original, create copy at new location)
      const sentPath = responderPath + '/sent';
      await tx.delete(workspace, event.node_path);
      await tx.createDeep(ACCESS_CONTROL, sentPath, {
        name: responseMessage.name,
        node_type: responseMessage.node_type,
        properties: { ...responseMessage.properties, status: 'sent' }
      });

      await tx.commit();
      return { success: true, friendship_created: false };

    } else {
      await tx.rollback();
      return { success: false, error: 'Invalid response value' };
    }

  } catch (err) {
    console.error('[friendship-response] Error:', err);
    if (tx) {
      try { await tx.rollback(); } catch(e) {}
    }
    return { success: false, error: err.message };
  }
} 