"""
Handle Relationship Response Handler

Handles relationship response messages by creating or rejecting relationships.
This handler is triggered when a message with type "relationship_response" is sent.
"""

def handle_relationship_response(input):
    """Handle a relationship response message.

    If accepted, creates the graph relation between users.
    Creates notification for the original requestor.

    Args:
        input: dict with node (the message) and event metadata

    Returns:
        dict with success, relationship_created, or error
    """
    # Support both direct input and flow_input wrapper (for trigger invocation)
    flow_input = input.get("flow_input", input)
    node = flow_input.get("node")
    workspace = flow_input.get("workspace", "raisin:access_control")

    if not node:
        fail("No message node provided")

    node_props = node.get("properties", {})
    node_path = node.get("path", "")

    # Validate this is a relationship response
    message_type = node_props.get("message_type")
    if message_type != "relationship_response":
        fail("Not a relationship response message")

    # Copy to sent folder before processing
    _copy_to_sent_folder(workspace, node)

    # Get response details
    body = node_props.get("body", {}) or {}
    accepted = node_props.get("accepted")
    if accepted == None:
        response = body.get("response")
        if type(response) == "string":
            normalized = response.lower().strip()
            if normalized in ["accept", "accepted", "approve", "approved"]:
                accepted = True
            elif normalized in ["reject", "rejected", "decline", "declined"]:
                accepted = False
        if accepted == None:
            accepted = False

    original_request_id = node_props.get("original_request_id") or body.get("original_request_id")
    original_request_path = node_props.get("original_request_path") or body.get("original_request_path")

    if not original_request_id and not original_request_path:
        fail("Missing original request reference")

    # Get the original request to get relationship details
    original_request = None
    if original_request_id:
        original_request = raisin.nodes.get_by_id(workspace, original_request_id)
    elif original_request_path:
        original_request = raisin.nodes.get(workspace, original_request_path)

    if not original_request:
        fail("Original request not found")

    original_props = original_request.get("properties", {})
    sender_id = original_props.get("sender_id")
    recipient_id = original_props.get("recipient_id")
    sender_path = original_props.get("sender_path")  # The original requestor (legacy)
    recipient_path = original_props.get("recipient_path")  # The person responding (legacy)
    relation_type = original_props.get("relation_type")

    if not (sender_id or sender_path) or not (recipient_id or recipient_path) or not relation_type:
        fail("Original request missing required fields")

    # Get user nodes
    sender = _resolve_user(workspace, sender_path, sender_id)
    recipient = _resolve_user(workspace, recipient_path, recipient_id)

    if not sender or not recipient:
        fail("Sender or recipient not found")

    sender_path = sender.get("path")
    recipient_path = recipient.get("path")
    sender_id = sender.get("id")
    recipient_id = recipient.get("id")

    relationship_created = False

    if accepted:
        # Create the graph relation: (sender)-[:RELATION_TYPE]->(recipient)
        relate_sql = "RELATE FROM path='" + sender_path + "' IN WORKSPACE '" + workspace + "' TO path='" + recipient_path + "' IN WORKSPACE '" + workspace + "' TYPE '" + relation_type + "'"
        raisin.sql.execute(relate_sql, [])
        relationship_created = True

        # Check if relation is bidirectional or has an inverse
        rel_type_path = "/relation-types/" + relation_type.lower().replace("_", "-")
        rel_type_node = raisin.nodes.get(workspace, rel_type_path)

        if rel_type_node and rel_type_node.get("properties"):
            rel_type_props = rel_type_node.get("properties", {})
            bidirectional = rel_type_props.get("bidirectional", False)
            inverse_type = rel_type_props.get("inverse_relation_name")

            # If bidirectional, create the same relation in reverse
            if bidirectional:
                reverse_sql = "RELATE FROM path='" + recipient_path + "' IN WORKSPACE '" + workspace + "' TO path='" + sender_path + "' IN WORKSPACE '" + workspace + "' TYPE '" + relation_type + "'"
                raisin.sql.execute(reverse_sql, [])
                log.info("Created bidirectional relationship: " + relation_type)
            # If has inverse, create the inverse relation
            elif inverse_type:
                inverse_sql = "RELATE FROM path='" + recipient_path + "' IN WORKSPACE '" + workspace + "' TO path='" + sender_path + "' IN WORKSPACE '" + workspace + "' TYPE '" + inverse_type + "'"
                raisin.sql.execute(inverse_sql, [])
                log.info("Created inverse relationship: " + relation_type + " / " + inverse_type)

        # Update original request status
        raisin.nodes.update_property(workspace, original_request.get("path", original_request_path), "status", "accepted")
        raisin.nodes.update_property(workspace, original_request.get("path", original_request_path), "accepted_at", _get_current_timestamp())

        log.info("Relationship created: " + sender_path + " -[" + relation_type + "]-> " + recipient_path)

    else:
        # Update original request status to rejected
        raisin.nodes.update_property(workspace, original_request.get("path", original_request_path), "status", "rejected")
        raisin.nodes.update_property(workspace, original_request.get("path", original_request_path), "rejected_at", _get_current_timestamp())
        raisin.nodes.update_property(workspace, original_request.get("path", original_request_path), "rejection_reason",
                                      node_props.get("rejection_reason", "Request declined by recipient"))

        log.info("Relationship request rejected: " + original_request_path)

    # Create notification inbox message for the original requestor
    _notify_requestor(sender_id, recipient_id, relation_type, accepted, workspace)

    # Update response message status to processed
    raisin.nodes.update_property(workspace, node_path, "status", "processed")
    raisin.nodes.update_property(workspace, node_path, "processed_at", _get_current_timestamp())

    return {
        "success": True,
        "relationship_created": relationship_created
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


def _notify_requestor(sender_id, recipient_id, relation_type, accepted, workspace):
    """Create a notification message for the original requestor.

    Args:
        sender_id: ID of the original requestor
        recipient_id: ID of the person who responded
        relation_type: The type of relationship
        accepted: Whether the request was accepted
        workspace: The workspace name
    """
    sender = _resolve_user(workspace, None, sender_id)
    recipient = _resolve_user(workspace, None, recipient_id)
    if not sender or not recipient:
        return

    sender_path = sender.get("path")
    recipient_path = recipient.get("path")
    if not sender_path or not recipient_path:
        return

    # Use inbox folder (created automatically by raisin:User initial_structure)
    inbox_folder_path = sender_path + "/inbox"

    # Get recipient display info
    recipient_props = recipient.get("properties", {})
    recipient_name = recipient_props.get("display_name") or recipient_id

    # Get relation type title
    rel_type_path = "/relation-types/" + relation_type.lower().replace("_", "-")
    rel_type_node = raisin.nodes.get(workspace, rel_type_path)
    relation_title = relation_type
    if rel_type_node and rel_type_node.get("properties"):
        relation_title = rel_type_node.get("properties", {}).get("title", relation_type)

    # Create notification message
    notification_slug = "response-" + str(_get_current_timestamp()).replace(":", "-").replace(" ", "-")[:20]

    if accepted:
        title = recipient_name + " accepted your " + relation_title + " request"
        message_text = "Your relationship request has been accepted."
    else:
        title = recipient_name + " declined your " + relation_title + " request"
        message_text = "Your relationship request was declined."

    notification_data = {
        "slug": notification_slug,
        "name": notification_slug,
        "node_type": "raisin:Message",
        "properties": {
            "title": title,
            "body": message_text,
            "message_type": "relationship_response_notification",
            "status": "unread",
            "sender_id": recipient_id,
            "recipient_id": sender_id,
            "sender_display_name": recipient_name,
            "relation_type": relation_type,
            "relation_title": relation_title,
            "accepted": accepted,
            "received_at": _get_current_timestamp()
        }
    }

    raisin.nodes.create(workspace, inbox_folder_path, notification_data)

    # Send real-time notification to the original requestor
    raisin.notify({
        "title": title,
        "body": message_text,
        "recipient": sender_path,
        "priority": 2,
        "link": "/friends",
        "data": {
            "responder_id": recipient_id,
            "relation_type": relation_type,
            "accepted": accepted
        }
    })


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
    if user_id:
        sql = """
            SELECT id, path, name, node_type, properties
            FROM '""" + workspace + """'
            WHERE node_type = 'raisin:User'
              AND properties->>'user_id'::String = $1
            LIMIT 1
        """
        result = raisin.sql.query(sql, [user_id])
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


def _get_current_timestamp():
    """Get the current timestamp as an ISO string.

    Returns:
        Current timestamp string
    """
    return raisin.date.now()
