/**
 * Handle Read Receipt
 *
 * Triggered when a read_receipt or batch_read_receipt message is created in a user's outbox.
 * Updates the original sender's message with read_by information.
 */
async function handleReadReceipt(context) {
  const { event, workspace } = context.flow_input;
  const ACCESS_CONTROL = 'raisin:access_control';

  console.log('[read-receipt] Trigger fired for:', event.node_path);

  const tx = await raisin.nodes.beginTransaction();
  try {
    // 1. Get the read receipt message
    const receipt = await tx.getByPath(workspace, event.node_path);
    if (!receipt) {
      console.error('[read-receipt] Receipt not found:', event.node_path);
      await tx.rollback();
      return { success: false, error: 'Receipt not found' };
    }

    const messageType = receipt.properties.message_type;
    console.log('[read-receipt] Processing receipt type:', messageType);

    // Handle batch read receipts
    if (messageType === 'batch_read_receipt') {
      return await handleBatchReadReceipt(tx, workspace, event.node_path, receipt);
    }

    // Handle individual read receipts (existing logic)
    console.log('[read-receipt] Processing individual receipt:', JSON.stringify(receipt.properties, null, 2));

    const { body } = receipt.properties;
    const readerId = body?.reader_id;
    const readAt = body?.read_at;
    const originalMessageId = body?.original_message_id;

    if (!originalMessageId) {
      console.error('[read-receipt] Missing original_message_id in body');
      await tx.delete(workspace, event.node_path);
      await tx.commit();
      return { success: false, error: 'Missing original_message_id' };
    }
    if (!readerId) {
      console.error('[read-receipt] Missing reader_id in body');
      await tx.delete(workspace, event.node_path);
      await tx.commit();
      return { success: false, error: 'Missing reader_id' };
    }

    // 2. Find the RECIPIENT'S copy of the message by ID
    // (The receipt references the message the recipient saw)
    console.log('[read-receipt] Looking up recipient message by ID:', originalMessageId);
    const recipientMessage = await tx.get(ACCESS_CONTROL, originalMessageId);

    if (!recipientMessage) {
      console.log('[read-receipt] Recipient message not found (it might have been deleted)');
      await tx.delete(workspace, event.node_path);
      await tx.commit();
      return { success: true, note: 'Recipient message not found' };
    }

    // 3. Find the SENDER'S copy using the link
    const senderMessagePath = recipientMessage.properties.sender_message_path;
    let senderMessage = null;

    if (senderMessagePath) {
        console.log('[read-receipt] Found link to sender message:', senderMessagePath);
        senderMessage = await tx.getByPath(ACCESS_CONTROL, senderMessagePath);
    } else {
        // Fallback: This might be an old message or sender copy was deleted/moved differently
        console.log('[read-receipt] No sender_message_path link found on recipient message');
    }

    if (!senderMessage) {
        console.log('[read-receipt] Sender message not found');
        // We still mark the receipt as processed
        await tx.delete(workspace, event.node_path);
        await tx.commit();
        return { success: true, note: 'Sender message not found' };
    }

    console.log('[read-receipt] Updating sender message at:', senderMessage.path);

    // 4. Update the SENDER'S message with read_by info
    const currentReadBy = senderMessage.properties.read_by || [];
    const alreadyRead = currentReadBy.some(r => r.id === readerId);
    
    if (alreadyRead) {
      console.log('[read-receipt] Already marked as read');
      await tx.delete(workspace, event.node_path);
      await tx.commit();
      return { success: true };
    }

    const updatedReadBy = [...currentReadBy, { id: readerId, read_at: readAt }];

    // Update Sender's message
    await tx.update(ACCESS_CONTROL, senderMessage.path, {
      properties: {
        ...senderMessage.properties,
        status: 'read',
        read_by: updatedReadBy
      }
    });

    // Optionally update Recipient's message too (so they see blue ticks if they look at their own message? 
    // No, usually meaningless for recipient, but keeps state consistent)
    await tx.update(ACCESS_CONTROL, recipientMessage.path, {
        properties: {
            ...recipientMessage.properties,
            status: 'read'
        }
    });

    // 5. Mark read receipt as processed and delete it
    await tx.delete(workspace, event.node_path);

    await tx.commit();
    console.log('[read-receipt] Successfully processed receipt and updated message');
    return { success: true };

  } catch (err) {
    console.error('[read-receipt] Error:', err);
    if (tx) {
      try { await tx.rollback(); } catch(e) {}
    }
    return { success: false, error: err.message };
  }
}

/**
 * Handle Batch Read Receipt
 *
 * Processes a batch of read receipts in a single transaction.
 * More efficient than processing N individual receipts.
 */
async function handleBatchReadReceipt(tx, workspace, receiptPath, receipt) {
  const ACCESS_CONTROL = 'raisin:access_control';
  const { body } = receipt.properties;

  const messageIds = body?.message_ids || [];
  const readerId = body?.reader_id;
  const readAt = body?.read_at;

  console.log(`[read-receipt] Processing batch receipt with ${messageIds.length} messages`);

  if (messageIds.length === 0) {
    console.log('[read-receipt] No messages in batch, deleting receipt');
    await tx.delete(workspace, receiptPath);
    await tx.commit();
    return { success: true, note: 'Empty batch' };
  }
  if (!readerId) {
    console.log('[read-receipt] Missing reader_id, deleting receipt');
    await tx.delete(workspace, receiptPath);
    await tx.commit();
    return { success: false, error: 'Missing reader_id' };
  }

  let updatedCount = 0;
  let skippedCount = 0;

  for (const messageId of messageIds) {
    try {
      // 1. Get the recipient's message by ID
      const recipientMessage = await tx.get(ACCESS_CONTROL, messageId);
      if (!recipientMessage) {
        console.log(`[read-receipt] Message ${messageId} not found (may have been deleted)`);
        skippedCount++;
        continue;
      }

      // 2. Get the sender's copy using the link
      const senderMessagePath = recipientMessage.properties.sender_message_path;
      if (!senderMessagePath) {
        console.log(`[read-receipt] No sender_message_path for message ${messageId}`);
        skippedCount++;
        continue;
      }

      const senderMessage = await tx.getByPath(ACCESS_CONTROL, senderMessagePath);
      if (!senderMessage) {
        console.log(`[read-receipt] Sender message not found at ${senderMessagePath}`);
        skippedCount++;
        continue;
      }

      // 3. Update sender's message with read_by info
      const currentReadBy = senderMessage.properties.read_by || [];
      const alreadyRead = currentReadBy.some(r => r.id === readerId);

      if (alreadyRead) {
        console.log(`[read-receipt] Message ${messageId} already marked as read by ${readerId}`);
        skippedCount++;
        continue;
      }

      const updatedReadBy = [...currentReadBy, { id: readerId, read_at: readAt }];

      await tx.update(ACCESS_CONTROL, senderMessage.path, {
        properties: {
          ...senderMessage.properties,
          status: 'read',
          read_by: updatedReadBy
        }
      });

      updatedCount++;
    } catch (msgErr) {
      console.error(`[read-receipt] Error processing message ${messageId}:`, msgErr);
      skippedCount++;
    }
  }

  // Delete the batch receipt
  await tx.delete(workspace, receiptPath);
  await tx.commit();

  console.log(`[read-receipt] Batch complete: ${updatedCount} updated, ${skippedCount} skipped`);
  return { success: true, updated: updatedCount, skipped: skippedCount };
}
