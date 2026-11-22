/**
 * Messaging Permissions Module
 *
 * May a sender message (or assign a task to) a recipient? The same rules every handler applies
 * (../shared/rules.js): `/config/messaging` permissions, blocks, relationships, circles, groups, roles.
 * Kept as a callable function for applications that want to ask before they offer "Message" or
 * "Assign a task" in their UI.
 *
 * The `raisin` global is injected at runtime -- do NOT import it.
 */

import { checkPermission } from '../handlers/shared/rules.js';

/**
 * @param {object} input - { sender_id, recipient_id, message_type }
 * @returns {object} { allowed: boolean, reason?: string }
 */
export async function check_messaging_permission(input) {
  const sender_id = input.sender_id;
  const recipient_id = input.recipient_id;
  if (!sender_id || !recipient_id) {
    throw new Error("Missing sender_id or recipient_id");
  }
  return checkPermission(sender_id, recipient_id, input.message_type ?? "chat");
}
