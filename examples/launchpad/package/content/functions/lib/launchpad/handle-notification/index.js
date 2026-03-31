/**
 * Handle Notification
 *
 * Triggered when a launchpad:Notification is created.
 * Simulates sending a push notification to the user's device.
 */
async function handleNotification(context) {
  const { event, workspace } = context.flow_input;
  
  console.log('[notification] New notification created:', event.node_path);

  const tx = await raisin.nodes.beginTransaction();
  try {
    const notification = await tx.getByPath(workspace, event.node_path);
    if (!notification) {
        console.error('[notification] Node not found');
        await tx.rollback();
        return { success: false };
    }

    const { title, body, type } = notification.properties;

    // Simulate Push Notification
    console.log('---------------------------------------------------');
    console.log(`[PUSH NOTIFICATION] To: ${event.node_path.split('/inbox')[0]}`);
    console.log(`[TITLE] ${title}`);
    console.log(`[BODY] ${body}`);
    console.log('---------------------------------------------------');

    await tx.commit();
    return { success: true };

  } catch (err) {
    console.error('[notification] Error processing:', err);
    if (tx) {
      try { await tx.rollback(); } catch(e) {}
    }
    return { success: false, error: err.message };
  }
}