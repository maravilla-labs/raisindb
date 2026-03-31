/**
 * Handle Unfriend
 *
 * Triggered when a user wants to remove a friend.
 * Removes bidirectional FRIENDS_WITH graph edges.
 */
async function handleUnfriend(context) {
  const { event, workspace } = context.flow_input;
  const ACCESS_CONTROL = 'raisin:access_control';

  console.log('[unfriend] Trigger fired for:', event.node_path);

  const tx = await raisin.nodes.beginTransaction();
  try {
    // 1. Get the unfriend message
    const message = await tx.getByPath(workspace, event.node_path);
    if (!message) {
      console.error('[unfriend] Message not found:', event.node_path);
      await tx.rollback();
      return { success: false, error: 'Message not found' };
    }

    console.log('[unfriend] Processing unfriend request:', JSON.stringify(message.properties, null, 2));

    const { body } = message.properties;
    const userPath = body?.user_path; 
    const friendPath = body?.friend_path;

    if (!userPath || !friendPath) {
      console.error('[unfriend] Missing user_path or friend_path');
      await tx.update(workspace, event.node_path, {
        properties: { ...message.properties, status: 'error', error: 'Missing paths' }
      });
      await tx.commit();
      return { success: false, error: 'Missing user_path or friend_path' };
    }

    console.log('[unfriend] Removing friendship between:', userPath, 'and', friendPath);

    // Remove bidirectional FRIENDS_WITH edges
    await raisin.sql.execute(`
      UNRELATE
        FROM path='${userPath}' IN WORKSPACE '${ACCESS_CONTROL}'
        TO path='${friendPath}' IN WORKSPACE '${ACCESS_CONTROL}'
        TYPE 'FRIENDS_WITH'
    `);

    await raisin.sql.execute(`
      UNRELATE
        FROM path='${friendPath}' IN WORKSPACE '${ACCESS_CONTROL}'
        TO path='${userPath}' IN WORKSPACE '${ACCESS_CONTROL}'
        TYPE 'FRIENDS_WITH'
    `);

    console.log('[unfriend] Friendship edges removed');

    const senderPath = event.node_path.split('/outbox/')[0];
    const sentPath = `${senderPath}/sent`;

    // "Move" from /outbox to /sent (delete original, create copy at new location)
    await tx.delete(workspace, event.node_path);
    await tx.createDeep(ACCESS_CONTROL, sentPath, {
      name: message.name,
      node_type: message.node_type,
      properties: { ...message.properties, status: 'sent' }
    });
    console.log('[unfriend] Moved message to sent folder');

    await tx.commit();
    return { success: true };

  } catch (err) {
    console.error('[unfriend] Error processing unfriend:', err);
    if (tx) {
      try { await tx.rollback(); } catch(e) {}
    }

    // Attempt global update for error status if transaction failed
    try {
      await raisin.nodes.update(workspace, event.node_path, {
          properties: { status: 'error', error: err.message }
      });
    } catch(e) {}

    return { success: false, error: err.message };
  }
}