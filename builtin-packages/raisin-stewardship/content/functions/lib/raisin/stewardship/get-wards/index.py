"""
Gets all wards for a given steward user.

Returns users for whom the steward has stewardship relationships (PARENT_OF, GUARDIAN_OF).
For PARENT_OF relationships, only includes wards who are minors (based on birth_date
and minor_age_threshold configuration).
"""

def get_wards(input):
    """Get all wards for a given steward user.

    Args:
        input: dict with steward_user_path and optional workspace

    Returns:
        dict with success, wards array, or error
    """
    steward_user_path = input.get("steward_user_path")
    workspace = input.get("workspace", "raisin:access_control")

    if not steward_user_path:
        fail("steward_user_path is required")

    # Get the steward user node
    steward_user = raisin.nodes.get(workspace, steward_user_path)
    if not steward_user:
        fail("Steward user not found at path: " + steward_user_path)

    # Get stewardship configuration
    config = raisin.nodes.get(workspace, "/config/stewardship")
    if not config or not config.get("properties"):
        fail("Stewardship configuration not found")

    config_props = config.get("properties", {})
    stewardship_relation_types = config_props.get("stewardship_relation_types", ["PARENT_OF", "GUARDIAN_OF"])
    minor_age_threshold = config_props.get("minor_age_threshold", 18)

    # Build relation types filter for SQL
    relation_types_filter = ", ".join(["'" + t + "'" for t in stewardship_relation_types])

    # Query outgoing relationships using SQL NEIGHBORS function
    sql = """
        SELECT
            e.dst_id as ward_id,
            e.edge_label as relation_type
        FROM NEIGHBORS($1, 'OUT', NULL) AS e
        WHERE e.edge_label IN (""" + relation_types_filter + """)
    """

    steward_id = steward_user.get("id")
    result = raisin.sql.query(sql, [steward_id])
    # Handle both list results and dict with "rows" key
    if type(result) == "list":
        rows = result
    else:
        rows = result.get("rows", []) if result else []

    # Get ward user nodes and check if they qualify
    wards = []
    for rel in rows:
        ward_id = rel.get("ward_id")
        relation_type = rel.get("relation_type")

        ward_node = raisin.nodes.get_by_id(workspace, ward_id)
        if ward_node and ward_node.get("node_type") == "raisin:User":
            is_minor = _calculate_is_minor(ward_node, minor_age_threshold)

            # For PARENT_OF relationships, only include if ward is a minor
            if relation_type == "PARENT_OF" and not is_minor:
                continue

            ward_props = ward_node.get("properties", {})

            # Get relation type node for metadata
            rel_type_path = "/relation-types/" + relation_type.lower().replace("_", "-")
            rel_type_node = raisin.nodes.get(workspace, rel_type_path)
            relation_title = relation_type
            if rel_type_node and rel_type_node.get("properties"):
                relation_title = rel_type_node.get("properties", {}).get("title", relation_type)

            wards.append({
                "user_id": ward_node.get("id"),
                "user_path": ward_node.get("path"),
                "email": ward_props.get("email"),
                "display_name": ward_props.get("display_name"),
                "relation_type": relation_type,
                "relation_title": relation_title,
                "is_minor": is_minor
            })

    return {
        "success": True,
        "wards": wards
    }


def _calculate_is_minor(user_node, minor_age_threshold):
    """Calculate if a user is a minor based on birth_date.

    Args:
        user_node: The user node dict
        minor_age_threshold: Age threshold for being considered a minor

    Returns:
        True if user is a minor, False otherwise
    """
    props = user_node.get("properties", {})
    birth_date_str = props.get("birth_date")

    if not birth_date_str:
        return False

    # Parse birth date (expected format: YYYY-MM-DD)
    birth_parts = birth_date_str.split("-")
    if len(birth_parts) != 3:
        return False

    birth_year = int(birth_parts[0])
    birth_month = int(birth_parts[1])
    birth_day = int(birth_parts[2])

    # Get current date using the date API
    now_str = raisin.date.now()
    date_parts = now_str.split("T")[0].split("-")
    current_year = int(date_parts[0])
    current_month = int(date_parts[1])
    current_day = int(date_parts[2])

    # Calculate age
    age = current_year - birth_year

    # Adjust if birthday hasn't occurred yet this year
    if current_month < birth_month:
        age = age - 1
    elif current_month == birth_month and current_day < birth_day:
        age = age - 1

    return age < minor_age_threshold
