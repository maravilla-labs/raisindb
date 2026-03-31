"""
Handle System Notification Handler

Handles system notification messages by creating Notification nodes for recipients.
This handler is triggered when a message with type "system_notification" is sent.
"""

def handle_system_notification(input):
    """Handle a system notification message.

    Creates a Notification node in the recipient's inbox/notifications folder.

    Args:
        input: dict with node (the message) and event metadata

    Returns:
        dict with success, notification_path, or error
    """
    # Support both direct input and flow_input wrapper (for trigger invocation)
    flow_input = input.get("flow_input", input)
    node = flow_input.get("node")
    workspace = flow_input.get("workspace", "raisin:access_control")

    if not node:
        fail("No message node provided")

    node_props = node.get("properties", {})
    node_path = node.get("path", "")
    message_id = node.get("id")

    # Validate this is a system notification
    message_type = node_props.get("message_type")
    if message_type != "system_notification":
        fail("Not a system notification message")

    # Copy to sent folder before processing
    _copy_to_sent_folder(workspace, node)

    # Get notification details from message
    body = node_props.get("body", {})
    recipient_path = node_props.get("recipient_path") or body.get("recipient_path")
    recipient_id = node_props.get("recipient_id") or body.get("recipient_id")
    subject = node_props.get("subject", "System Notification")

    notification_type = body.get("notification_type", "system")
    notification_title = body.get("title", subject)
    notification_body = body.get("body", "")
    notification_link = body.get("link")
    notification_priority = body.get("priority", 3)
    notification_data = body.get("data", {})
    expires_in_seconds = body.get("expires_in_seconds")

    if not recipient_path and not recipient_id:
        fail("Missing required field: recipient_path or recipient_id")

    # Validate recipient exists
    recipient = _resolve_user(workspace, recipient_path, recipient_id)
    if not recipient:
        fail("Recipient not found")

    recipient_path = recipient.get("path")

    # Use folders (created automatically by raisin:User initial_structure)
    inbox_folder_path = recipient_path + "/inbox"
    notifications_folder_path = recipient_path + "/notifications"

    # Generate notification slug
    notification_slug = "notif-sys-" + str(message_id)[-8:]

    # Build notification properties
    notification_props = {
        "type": notification_type,
        "title": notification_title,
        "read": False,
        "priority": notification_priority,
        "data": {
            "source_message_path": node_path,
            "source_message_id": message_id
        }
    }

    if notification_body:
        notification_props["body"] = notification_body

    if notification_link:
        notification_props["link"] = notification_link

    if notification_data:
        # Merge additional data
        notification_props["data"].update(notification_data)

    if expires_in_seconds:
        current_ts = raisin.date.timestamp()
        expires_ts = current_ts + int(expires_in_seconds)
        notification_props["expires_at"] = raisin.date.format(expires_ts)

    # Create Notification
    notification_data_node = {
        "slug": notification_slug,
        "name": notification_slug,
        "node_type": "raisin:Notification",
        "properties": notification_props
    }

    raisin.nodes.create(workspace, notifications_folder_path, notification_data_node)
    notification_path = notifications_folder_path + "/" + notification_slug

    # Update original message status to "delivered"
    raisin.nodes.update_property(workspace, node_path, "status", "delivered")
    raisin.nodes.update_property(workspace, node_path, "delivered_at", raisin.date.now())

    log.info("System notification created: " + notification_path)

    # Optionally call placeholder push notification function
    # This is a placeholder that logs the notification
    _log_push_notification(recipient_path, notification_title, notification_body)

    return {
        "success": True,
        "notification_path": notification_path
    }


def _copy_to_sent_folder(workspace, node):
    """Copy outbox message to sender's sent folder.

    Args:
        workspace: The workspace name
        node: The message node

    Returns:
        The path to the sent message, or None if copy failed
    """
    node_path = node.get("path", "")
    node_props = node.get("properties", {})

    # Extract sender from path (e.g., /users/alice/outbox/msg-001)
    path_parts = node_path.split("/")
    if len(path_parts) < 5 or path_parts[1] != "users" or path_parts[3] != "outbox":
        return None

    sender_username = path_parts[2]
    message_slug = path_parts[4]

    # Use sent folder (created automatically by raisin:User initial_structure)
    sent_folder_path = "/users/" + sender_username + "/sent"

    # Copy with updated status
    sent_props = dict(node_props)
    sent_props["status"] = "sent"
    sent_props["sent_at"] = raisin.date.now()

    raisin.nodes.create(workspace, sent_folder_path, {
        "slug": message_slug,
        "name": message_slug,
        "node_type": node.get("node_type", "raisin:Message"),
        "properties": sent_props
    })

    # Update original outbox message status
    raisin.nodes.update_property(workspace, node_path, "status", "sent")
    raisin.nodes.update_property(workspace, node_path, "sent_at", raisin.date.now())

    return sent_folder_path + "/" + message_slug


def _resolve_user(workspace, user_path=None, user_id=None):
    """Resolve a user node from path or id (global user UUID).

    Args:
        workspace: The workspace to search in
        user_path: Optional path like /users/alice
        user_id: Optional global user UUID (identity ID) or workspace node ID

    Returns:
        User node dict or None if not found
    """
    # Handle case where user_id is actually a path
    if user_id and type(user_id) == "string" and user_id.startswith("/users/"):
        user_path = user_id
        user_id = None

    # Search by global user_id property first (identity UUID)
    # This handles the case where user_id is the global identity UUID,
    # which is stored in properties.user_id when a User node is created
    if user_id:
        log.info("[_resolve_user] Searching for user_id: " + str(user_id) + " in workspace: " + str(workspace))
        sql = """
            SELECT id, path, name, node_type, properties
            FROM '""" + workspace + """'
            WHERE node_type = 'raisin:User'
              AND properties->>'user_id'::String = $1
            LIMIT 1
        """
        result = raisin.sql.query(sql, [user_id])
        log.info("[_resolve_user] SQL result: " + str(result))
        # Handle both list results and dict with "rows" key
        if type(result) == "list":
            rows = result
        else:
            rows = result.get("rows", []) if result else []
        if rows:
            row = rows[0]
            return {
                "id": row.get("id"),
                "path": row.get("path"),
                "name": row.get("name"),
                "node_type": row.get("node_type"),
                "properties": row.get("properties")
            }

    # Try by path
    if user_path:
        node = raisin.nodes.get(workspace, user_path)
        if node:
            return node

    # Try direct node ID lookup (workspace node ID) as fallback
    if user_id:
        node = raisin.nodes.get_by_id(workspace, user_id)
        if node:
            return node

    return None


def _log_push_notification(recipient_path, title, body):
    """Log a placeholder push notification.

    In a real implementation, this would call an external push notification service.

    Args:
        recipient_path: Path to the recipient user
        title: Notification title
        body: Notification body
    """
    log.info("[PUSH] Would send notification to user: " + recipient_path)
    log.info("[PUSH] Title: " + str(title))
    log.info("[PUSH] Body: " + str(body))
