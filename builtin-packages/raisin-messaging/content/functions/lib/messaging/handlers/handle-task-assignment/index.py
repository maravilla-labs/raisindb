"""
Handle Task Assignment Handler

Handles task assignment messages by creating InboxTask and Notification nodes for assignees.
This handler is triggered when a message with type "task_assignment" is sent.
"""

def handle_task_assignment(input):
    """Handle a task assignment message.

    Creates an InboxTask and Notification node in the assignee's inbox.

    Args:
        input: dict with node (the message) and event metadata

    Returns:
        dict with success, task_path, notification_path, or error
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

    # Validate this is a task assignment
    message_type = node_props.get("message_type")
    if message_type != "task_assignment":
        fail("Not a task assignment message")

    # Copy to sent folder before processing
    _copy_to_sent_folder(workspace, node)

    # Get task details from message body
    body = node_props.get("body", {})
    assignee_path = body.get("assignee_path")
    assignee_id = body.get("assignee_id")
    task_type = body.get("task_type", "action")
    task_title = body.get("title", node_props.get("subject", "New Task"))
    task_description = body.get("description", "")
    task_options = body.get("options")
    task_input_schema = body.get("input_schema")
    task_priority = body.get("priority", 3)
    due_in_seconds = body.get("due_in_seconds")
    flow_instance_ref = body.get("flow_instance_ref")
    step_id = body.get("step_id")

    sender_path = node_props.get("sender_path")
    sender_id = node_props.get("sender_id")

    if not assignee_path and not assignee_id:
        fail("Missing required field: assignee_path or assignee_id in body")

    # Validate task_type
    valid_task_types = ["approval", "input", "review", "action"]
    if task_type not in valid_task_types:
        fail("Invalid task_type: " + str(task_type) + ". Must be one of: " + ", ".join(valid_task_types))

    # Validate assignee exists
    assignee = _resolve_user(workspace, assignee_path, assignee_id)
    if not assignee:
        fail("Assignee not found")

    assignee_path = assignee.get("path")

    # Use folders (inbox created automatically by raisin:User initial_structure)
    inbox_folder_path = assignee_path + "/inbox"
    # Use root notifications folder (created automatically by raisin:User initial_structure)
    notifications_folder_path = assignee_path + "/notifications"

    # Generate slugs
    task_slug = "task-" + str(message_id)[-8:]
    notification_slug = "notif-task-" + str(message_id)[-8:]

    # Build task properties
    task_props = {
        "task_type": task_type,
        "title": task_title,
        "status": "pending",
        "priority": task_priority,
        "source_message_path": node_path,
        "source_message_id": message_id
    }

    if task_description:
        task_props["description"] = task_description

    if task_options:
        task_props["options"] = task_options

    if task_input_schema:
        task_props["input_schema"] = task_input_schema

    if flow_instance_ref:
        task_props["flow_instance_ref"] = flow_instance_ref

    if step_id:
        task_props["step_id"] = step_id

    if due_in_seconds:
        # Calculate due date
        current_ts = raisin.date.timestamp()
        due_ts = current_ts + int(due_in_seconds)
        task_props["due_date"] = raisin.date.format(due_ts)

    # Create InboxTask
    task_data = {
        "slug": task_slug,
        "name": task_slug,
        "node_type": "raisin:InboxTask",
        "properties": task_props
    }

    raisin.nodes.create(workspace, inbox_folder_path, task_data)
    task_path = inbox_folder_path + "/" + task_slug

    # Get sender name for notification
    sender_name = "System"
    sender = _resolve_user(workspace, sender_path, sender_id)
    if sender and sender.get("properties"):
        sender_props = sender.get("properties", {})
        sender_name = sender_props.get("display_name") or sender_id or sender_path

    # Create Notification
    notification_data = {
        "slug": notification_slug,
        "name": notification_slug,
        "node_type": "raisin:Notification",
        "properties": {
            "type": "task_assignment",
            "title": "New task: " + task_title,
            "body": task_description or "You have been assigned a new task",
            "link": task_path,
            "read": False,
            "priority": task_priority,
            "data": {
                "task_type": task_type,
                "task_path": task_path,
                "sender_id": sender_id,
                "sender_name": sender_name
            }
        }
    }

    raisin.nodes.create(workspace, notifications_folder_path, notification_data)
    notification_path = notifications_folder_path + "/" + notification_slug

    # Update original message status to "delivered"
    raisin.nodes.update_property(workspace, node_path, "status", "delivered")
    raisin.nodes.update_property(workspace, node_path, "delivered_at", raisin.date.now())

    log.info("Task assigned and notification created: " + task_path)

    return {
        "success": True,
        "task_path": task_path,
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
