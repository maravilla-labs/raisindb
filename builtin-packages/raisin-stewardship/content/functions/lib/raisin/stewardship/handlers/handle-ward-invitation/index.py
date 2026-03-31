"""
Handle Ward Invitation Handler

Handles ward invitation messages by creating new ward accounts and establishing relationships.
This handler is triggered when a message with type "ward_invitation" is sent.
"""

def handle_ward_invitation(input):
    """Handle a ward invitation message.

    Creates a new user node for the ward and establishes the stewardship relationship.

    Args:
        input: dict with node (the message) and event metadata

    Returns:
        dict with success, ward_path, relationship_created, or error
    """
    # Support both direct input and flow_input wrapper (for trigger invocation)
    flow_input = input.get("flow_input", input)
    node = flow_input.get("node")
    workspace = flow_input.get("workspace", "raisin:access_control")

    if not node:
        fail("No message node provided")

    node_props = node.get("properties", {})
    node_path = node.get("path", "")

    # Validate this is a ward invitation
    message_type = node_props.get("message_type")
    if message_type != "ward_invitation":
        fail("Not a ward invitation message")

    # Copy to sent folder before processing
    _copy_to_sent_folder(workspace, node)

    # Check if ward creation is enabled
    config = raisin.nodes.get(workspace, "/config/stewardship")
    if config and config.get("properties"):
        config_props = config.get("properties", {})
        if not config_props.get("steward_creates_ward_enabled", False):
            raisin.nodes.update_property(workspace, node_path, "status", "rejected")
            raisin.nodes.update_property(workspace, node_path, "rejection_reason", "Ward creation is disabled")
            fail("Ward creation is disabled in stewardship configuration")

    # Get invitation details
    steward_path = node_props.get("steward_path")
    ward_display_name = node_props.get("ward_display_name")
    ward_email = node_props.get("ward_email")
    ward_birth_date = node_props.get("ward_birth_date")
    relation_type = node_props.get("relation_type", "PARENT_OF")

    if not steward_path:
        fail("Missing steward_path in invitation")

    if not ward_display_name and not ward_email:
        fail("Missing ward_display_name or ward_email in invitation")

    # Validate steward exists
    steward = raisin.nodes.get(workspace, steward_path)
    if not steward:
        fail("Steward not found: " + steward_path)

    # Generate ward username (slug)
    ward_slug = _generate_ward_slug(ward_display_name, ward_email)

    # Check if ward already exists
    ward_path = "/users/internal/" + ward_slug
    existing_ward = raisin.nodes.get(workspace, ward_path)
    if existing_ward:
        fail("User already exists at path: " + ward_path)

    # Create ward user node
    ward_properties = {
        "display_name": ward_display_name,
        "created_by_steward": steward_path,
        "created_at": _get_current_timestamp(),
        "is_managed_ward": True
    }

    if ward_email:
        ward_properties["email"] = ward_email

    if ward_birth_date:
        ward_properties["birth_date"] = ward_birth_date

    ward_data = {
        "slug": ward_slug,
        "name": ward_slug,
        "node_type": "raisin:User",
        "properties": ward_properties
    }

    # Create the ward user
    raisin.nodes.create(workspace, "/users/internal", ward_data)
    log.info("Created ward user: " + ward_path)

    # Verify ward was created
    ward = raisin.nodes.get(workspace, ward_path)
    if not ward:
        fail("Failed to create ward user")

    # Create the stewardship relationship
    relate_sql = "RELATE FROM path='" + steward_path + "' IN WORKSPACE '" + workspace + "' TO path='" + ward_path + "' IN WORKSPACE '" + workspace + "' TYPE '" + relation_type + "'"
    raisin.sql.execute(relate_sql, [])
    log.info("Created relationship: " + steward_path + " -[" + relation_type + "]-> " + ward_path)

    # Check if relation is bidirectional
    rel_type_path = "/relation-types/" + relation_type.lower().replace("_", "-")
    rel_type_node = raisin.nodes.get(workspace, rel_type_path)

    if rel_type_node and rel_type_node.get("properties"):
        rel_type_props = rel_type_node.get("properties", {})
        inverse_type = rel_type_props.get("inverse_relation_type")

        if inverse_type:
            inverse_sql = "RELATE FROM path='" + ward_path + "' IN WORKSPACE '" + workspace + "' TO path='" + steward_path + "' IN WORKSPACE '" + workspace + "' TYPE '" + inverse_type + "'"
            raisin.sql.execute(inverse_sql, [])
            log.info("Created inverse relationship: " + ward_path + " -[" + inverse_type + "]-> " + steward_path)

    # Create welcome inbox message for the new ward
    _create_welcome_message(ward_path, steward_path, steward, workspace)

    # Update invitation status
    raisin.nodes.update_property(workspace, node_path, "status", "processed")
    raisin.nodes.update_property(workspace, node_path, "processed_at", _get_current_timestamp())
    raisin.nodes.update_property(workspace, node_path, "created_ward_path", ward_path)

    return {
        "success": True,
        "ward_path": ward_path,
        "relationship_created": True
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


def _generate_ward_slug(display_name, email):
    """Generate a URL-safe slug for the ward user.

    Args:
        display_name: The ward's display name
        email: The ward's email (optional)

    Returns:
        A URL-safe slug string
    """
    base = display_name or email.split("@")[0] if email else "ward"

    # Convert to lowercase and replace spaces/special chars with hyphens
    slug = base.lower()
    slug = slug.replace(" ", "-")

    # Remove any characters that aren't alphanumeric or hyphens
    clean_slug = ""
    for char in slug:
        if char.isalnum() or char == "-":
            clean_slug = clean_slug + char

    # Add timestamp suffix for uniqueness
    suffix = str(raisin.date.timestamp_millis() % 100000)

    return clean_slug + "-" + suffix


def _create_welcome_message(ward_path, steward_path, steward_node, workspace):
    """Create a welcome inbox message for the new ward.

    Args:
        ward_path: Path to the new ward user
        steward_path: Path to the steward who created the ward
        steward_node: The steward node
        workspace: The workspace name
    """
    # Extract ward username
    ward_parts = ward_path.split("/")
    if len(ward_parts) < 4:
        return

    ward_username = ward_parts[3]
    inbox_folder_path = "/users/internal/" + ward_username + "/inbox"

    # Get steward display name
    steward_props = steward_node.get("properties", {})
    steward_name = steward_props.get("display_name") or steward_props.get("email") or steward_path

    # Create welcome message
    welcome_data = {
        "slug": "welcome",
        "name": "welcome",
        "node_type": "raisin:Message",
        "properties": {
            "title": "Welcome to Your Account",
            "body": "Welcome! Your account has been created by " + steward_name + ". They will help manage your account.",
            "message_type": "welcome",
            "status": "unread",
            "steward_path": steward_path,
            "steward_name": steward_name,
            "received_at": _get_current_timestamp()
        }
    }

    raisin.nodes.create(workspace, inbox_folder_path, welcome_data)
    log.info("Created welcome message for ward: " + ward_path)


def _get_current_timestamp():
    """Get the current timestamp as an ISO string.

    Returns:
        Current timestamp string
    """
    return raisin.date.now()
