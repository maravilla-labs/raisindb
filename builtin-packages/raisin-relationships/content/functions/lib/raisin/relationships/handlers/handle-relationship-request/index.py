"""
Handle Relationship Request Handler

Handles relationship request messages by creating inbox messages for recipients.
This handler is triggered when a message with type "relationship_request" is sent.
"""

def handle_relationship_request(input):
    """Handle a relationship request message.

    Creates an inbox message for the recipient to accept or reject the relationship.

    Args:
        input: dict with node (the message) and event metadata

    Returns:
        dict with success, inbox_message_path, or error
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

    # Validate this is a relationship request
    message_type = node_props.get("message_type")
    if message_type != "relationship_request":
        fail("Not a relationship request message")

    # Copy to sent folder before processing
    _copy_to_sent_folder(workspace, node)

    # Get sender and recipient information
    body = node_props.get("body", {}) or {}
    sender_id = node_props.get("sender_id") or body.get("sender_id")
    recipient_id = node_props.get("recipient_id") or body.get("recipient_id")
    sender_path = node_props.get("sender_path") or body.get("sender_path")
    recipient_path = node_props.get("recipient_path") or body.get("recipient_path")
    recipient_email = node_props.get("recipient_email") or body.get("recipient_email")
    relation_type = (
        node_props.get("relation_type") or
        body.get("relation_type") or
        body.get("relationship_type")
    )

    if not (sender_id or sender_path) or not (recipient_id or recipient_path or recipient_email) or not relation_type:
        fail("Missing required fields: sender_id/recipient_id or relation_type")

    log.info("[handle_relationship_request] Resolving sender - path: " + str(sender_path) + ", id: " + str(sender_id))
    log.info("[handle_relationship_request] Resolving recipient - path: " + str(recipient_path) + ", id: " + str(recipient_id) + ", email: " + str(recipient_email))

    # Validate sender exists
    sender = _resolve_user(workspace, sender_path, sender_id, None)
    if not sender:
        fail("Sender not found")

    # Validate recipient exists
    recipient = _resolve_user(workspace, recipient_path, recipient_id, recipient_email)
    if not recipient:
        fail("Recipient not found")

    sender_path = sender.get("path")
    recipient_path = recipient.get("path")
    sender_id = sender.get("id")
    recipient_id = recipient.get("id")

    # Propagate resolved recipient info back to the outbox message
    # so the response handler can read it when the request is accepted/rejected.
    # Guards:
    #   1. Only update if the outbox message belongs to the sender (path check)
    #   2. Only update if fields are not already set (prevent overwrite/replay)
    if not node_path.startswith(sender_path + "/"):
        fail("Message does not belong to sender")
    existing_recipient_id = node_props.get("recipient_id")
    existing_recipient_path = node_props.get("recipient_path")
    if existing_recipient_id or existing_recipient_path:
        fail("Recipient already resolved on this request — possible replay")
    raisin.nodes.update_property(workspace, node_path, "recipient_id", recipient_id)
    raisin.nodes.update_property(workspace, node_path, "recipient_path", recipient_path)

    # Get config to validate limits
    config = raisin.nodes.get(workspace, "/config/stewardship")
    if config and config.get("properties"):
        config_props = config.get("properties", {})
        max_pending_requests = config_props.get("max_pending_requests", 10)

        # Check if recipient has too many pending requests
        pending_count = _count_pending_requests(recipient_path, workspace)
        if pending_count >= max_pending_requests:
            # Update original message status to "rejected"
            raisin.nodes.update_property(workspace, node_path, "status", "rejected")
            raisin.nodes.update_property(workspace, node_path, "rejection_reason", "Too many pending requests")
            fail("Recipient has too many pending requests")

    # Use inbox folder (created automatically by raisin:User initial_structure)
    inbox_folder_path = recipient_path + "/inbox"

    # Generate inbox message slug
    inbox_message_slug = "request-" + str(message_id)[-8:]

    # Get relation type metadata
    rel_type_path = "/relation-types/" + relation_type.lower().replace("_", "-")
    rel_type_node = raisin.nodes.get(workspace, rel_type_path)
    relation_title = relation_type
    if rel_type_node and rel_type_node.get("properties"):
        relation_title = rel_type_node.get("properties", {}).get("title", relation_type)

    sender_props = sender.get("properties", {})
    sender_name = sender_props.get("display_name") or sender_id

    # Create inbox message for recipient
    message_text = node_props.get("message") or body.get("message", "")
    inbox_message_data = {
        "slug": inbox_message_slug,
        "name": inbox_message_slug,
        "node_type": "raisin:Message",
        "properties": {
            "title": "Relationship Request from " + sender_name,
            "body": message_text or (sender_name + " wants to connect with you as " + relation_title),
            "message_type": "relationship_request_received",
            "status": "pending",
            "sender_id": sender_id,
            "recipient_id": recipient_id,
            "sender_display_name": sender_name,
            "relation_type": relation_type,
            "relation_title": relation_title,
            "original_request_id": message_id,
            "received_at": _get_current_timestamp()
        }
    }

    raisin.nodes.create(workspace, inbox_folder_path, inbox_message_data)
    inbox_message_path = inbox_folder_path + "/" + inbox_message_slug

    # Update original message status to "delivered"
    raisin.nodes.update_property(workspace, node_path, "status", "delivered")
    raisin.nodes.update_property(workspace, node_path, "delivered_at", _get_current_timestamp())

    # Send real-time notification to recipient
    raisin.notify({
        "title": "New Relationship Request",
        "body": sender_name + " wants to connect with you as " + relation_title,
        "recipient": recipient_path,
        "priority": 2,
        "link": "/friends/requests",
        "data": {
            "request_id": message_id,
            "sender_id": sender_id,
            "relation_type": relation_type,
            "inbox_message_path": inbox_message_path
        }
    })

    log.info("Relationship request delivered to inbox: " + inbox_message_path)

    return {
        "success": True,
        "inbox_message_path": inbox_message_path
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


def _count_pending_requests(recipient_path, workspace):
    """Count the number of pending relationship requests for a recipient.

    Args:
        recipient_path: Path to the recipient user
        workspace: The workspace name

    Returns:
        Number of pending requests
    """
    # Use full recipient path directly
    inbox_path = recipient_path + "/inbox"

    # Query for pending requests
    sql = """
        SELECT COUNT(*) as count
        FROM nodes
        WHERE path LIKE $1
          AND node_type = 'raisin:Message'
          AND properties->>'message_type'::String = 'relationship_request_received'
          AND properties->>'status'::String = 'pending'
    """

    result = raisin.sql.query(sql, [inbox_path + "/%"])
    # Handle both list results and dict with "rows" key
    if type(result) == "list":
        rows = result
    else:
        rows = result.get("rows", []) if result else []

    if len(rows) > 0:
        return int(rows[0].get("count", 0))

    return 0


def _resolve_user(workspace, user_path=None, user_id=None, email=None):
    """Resolve a user node from path, id (global user UUID), or email.

    Args:
        workspace: The workspace to search in
        user_path: Optional path like /users/alice
        user_id: Optional global user UUID (identity ID) or workspace node ID
        email: Optional email address to lookup

    Returns:
        User node dict or None if not found
    """
    log.info("[_resolve_user] Called with: workspace=" + str(workspace) + ", user_path=" + str(user_path) + ", user_id=" + str(user_id) + ", email=" + str(email))

    # Handle case where user_id is actually a path
    if user_id and type(user_id) == "string" and user_id.startswith("/users/"):
        log.info("[_resolve_user] user_id looks like a path, converting: " + str(user_id))
        user_path = user_id
        user_id = None

    # Search by global user_id property first (identity UUID)
    if user_id:
        sql = """
            SELECT id, path, name, node_type, properties
            FROM '""" + workspace + """'
            WHERE node_type = 'raisin:User'
              AND properties->>'user_id'::String = $1
            LIMIT 1
        """
        log.info("[_resolve_user] Searching by user_id: " + str(user_id) + " in workspace: " + str(workspace))
        log.info("[_resolve_user] SQL: " + sql)
        result = raisin.sql.query(sql, [user_id])
        log.info("[_resolve_user] user_id query result: " + str(result))
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
        log.info("[_resolve_user] Trying by path: " + str(user_path))
        node = raisin.nodes.get(workspace, user_path)
        log.info("[_resolve_user] path lookup result: " + str(node))
        if node:
            return node

    # Try direct node ID lookup (workspace node ID) as fallback
    if user_id:
        log.info("[_resolve_user] Trying by node ID: " + str(user_id))
        node = raisin.nodes.get_by_id(workspace, user_id)
        log.info("[_resolve_user] node ID lookup result: " + str(node))
        if node:
            return node

    # Try by email
    if email:
        sql = """
            SELECT id, path, name, node_type, properties
            FROM '""" + workspace + """'
            WHERE node_type = 'raisin:User'
              AND properties->>'email'::String = $1
            LIMIT 1
        """
        log.info("[_resolve_user] Searching by email: " + str(email) + " in workspace: " + str(workspace))
        log.info("[_resolve_user] SQL: " + sql)
        result = raisin.sql.query(sql, [email])
        log.info("[_resolve_user] email query result: " + str(result))
        # Handle both list results and dict with "rows" key
        if type(result) == "list":
            rows = result
        else:
            rows = result.get("rows", []) if result else []
        if rows:
            row = rows[0]
            log.info("[_resolve_user] Found by email!")
            return {
                "id": row.get("id"),
                "path": row.get("path"),
                "name": row.get("name"),
                "node_type": row.get("node_type"),
                "properties": row.get("properties")
            }

    log.info("[_resolve_user] User not found by any method!")
    return None


def _get_current_timestamp():
    """Get the current timestamp as an ISO string.

    Returns:
        Current timestamp string
    """
    return raisin.date.now()
