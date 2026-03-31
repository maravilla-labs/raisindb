"""
Handle Stewardship Request Handler

Handles stewardship delegation request messages.
This handler is triggered when a message with type "stewardship_request" is sent.
It validates stewardship permissions, creates delegation records, and notifies both parties.
"""

def handle_stewardship_request(input):
    """Handle a stewardship delegation request message.

    Creates an inbox message for the ward to accept/reject the delegation,
    and creates notifications for both steward and ward.

    Args:
        input: dict with node (the message) and event metadata

    Returns:
        dict with success, inbox_message_path, notification_path, or error
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

    # Validate this is a stewardship request
    message_type = node_props.get("message_type")
    if message_type != "stewardship_request":
        fail("Not a stewardship request message")

    # Copy to sent folder before processing
    _copy_to_sent_folder(workspace, node)

    # Get request details from message body
    body = node_props.get("body", {})
    steward_path = body.get("steward_path")
    ward_path = body.get("ward_path")
    delegation_type = body.get("delegation_type", "general")
    scope = body.get("scope", [])
    expires_in_days = body.get("expires_in_days")
    message_text = body.get("message", "")

    if not steward_path or not ward_path:
        fail("Missing required fields: steward_path or ward_path in body")

    # Validate steward exists
    steward = raisin.nodes.get(workspace, steward_path)
    if not steward:
        fail("Steward not found: " + steward_path)

    # Validate ward exists
    ward = raisin.nodes.get(workspace, ward_path)
    if not ward:
        fail("Ward not found: " + ward_path)

    # Get stewardship config for validation
    config = raisin.nodes.get(workspace, "/config/stewardship")
    if config and config.get("properties"):
        config_props = config.get("properties", {})
        allowed_delegation_types = config_props.get("allowed_delegation_types", [])

        if allowed_delegation_types and delegation_type not in allowed_delegation_types:
            raisin.nodes.update_property(workspace, node_path, "status", "rejected")
            raisin.nodes.update_property(workspace, node_path, "rejection_reason", "Invalid delegation type")
            fail("Invalid delegation type: " + delegation_type)

    # Extract ward username from path
    ward_parts = ward_path.split("/")
    if len(ward_parts) < 3:
        fail("Invalid ward path format")
    ward_username = ward_parts[2]

    # Use folders (created automatically by raisin:User initial_structure)
    inbox_folder_path = "/users/" + ward_username + "/inbox"
    notifications_folder_path = "/users/" + ward_username + "/notifications"

    # Generate slugs
    inbox_message_slug = "stewardship-req-" + str(message_id)[-8:]
    notification_slug = "notif-steward-" + str(message_id)[-8:]

    # Get steward name for display
    steward_props = steward.get("properties", {})
    steward_name = steward_props.get("display_name") or steward_props.get("email") or steward_path

    # Build expiration date if specified
    expires_at = None
    if expires_in_days:
        current_ts = raisin.date.timestamp()
        expires_ts = current_ts + (int(expires_in_days) * 86400)
        expires_at = raisin.date.format(expires_ts)

    # Create inbox message for ward to accept/reject
    inbox_message_data = {
        "slug": inbox_message_slug,
        "name": inbox_message_slug,
        "node_type": "raisin:Message",
        "properties": {
            "title": "Stewardship Request from " + steward_name,
            "body": message_text or (steward_name + " wants to act on your behalf"),
            "message_type": "stewardship_request_received",
            "status": "pending",
            "sender_path": steward_path,
            "sender_name": steward_name,
            "recipient_path": ward_path,
            "original_request_path": node_path,
            "original_request_id": message_id,
            "data": {
                "steward_path": steward_path,
                "steward_name": steward_name,
                "ward_path": ward_path,
                "delegation_type": delegation_type,
                "scope": scope,
                "expires_at": expires_at
            },
            "received_at": raisin.date.now()
        }
    }

    raisin.nodes.create(workspace, inbox_folder_path, inbox_message_data)
    inbox_message_path = inbox_folder_path + "/" + inbox_message_slug

    # Create Notification for ward
    notification_data = {
        "slug": notification_slug,
        "name": notification_slug,
        "node_type": "raisin:Notification",
        "properties": {
            "type": "stewardship_request",
            "title": "Stewardship request from " + steward_name,
            "body": message_text or (steward_name + " wants to act on your behalf"),
            "link": inbox_message_path,
            "read": False,
            "priority": 4,
            "data": {
                "steward_path": steward_path,
                "steward_name": steward_name,
                "delegation_type": delegation_type,
                "source_message_path": node_path
            }
        }
    }

    raisin.nodes.create(workspace, notifications_folder_path, notification_data)
    notification_path = notifications_folder_path + "/" + notification_slug

    # Update original message status to "delivered"
    raisin.nodes.update_property(workspace, node_path, "status", "delivered")
    raisin.nodes.update_property(workspace, node_path, "delivered_at", raisin.date.now())

    log.info("Stewardship request delivered to ward: " + inbox_message_path)

    return {
        "success": True,
        "inbox_message_path": inbox_message_path,
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
