"""
Gets all stewards for a given ward user.

Returns users who have stewardship relationships (PARENT_OF, GUARDIAN_OF) with
the specified ward. For PARENT_OF relationships, only returns parents if the ward
is a minor (based on birth_date and minor_age_threshold configuration).
"""

def get_stewards(input):
    """Get all stewards for a given ward user.

    Args:
        input: dict with ward_user_path and optional workspace

    Returns:
        dict with success, stewards array, or error
    """
    ward_user_path = input.get("ward_user_path")
    workspace = input.get("workspace", "raisin:access_control")

    if not ward_user_path:
        fail("ward_user_path is required")

    # Get the ward user node
    ward_user = raisin.nodes.get(workspace, ward_user_path)
    if not ward_user:
        fail("Ward user not found at path: " + ward_user_path)

    # Get stewardship configuration
    config = raisin.nodes.get(workspace, "/config/stewardship")
    if not config or not config.get("properties"):
        fail("Stewardship configuration not found")

    config_props = config.get("properties", {})
    stewardship_relation_types = config_props.get("stewardship_relation_types", ["PARENT_OF", "GUARDIAN_OF"])
    minor_age_threshold = config_props.get("minor_age_threshold", 18)

    # Check if ward is a minor (for PARENT_OF filtering)
    is_minor = _calculate_is_minor(ward_user, minor_age_threshold)

    # Build relation types filter for SQL
    relation_types_filter = ", ".join(["'" + t + "'" for t in stewardship_relation_types])

    # Query incoming relationships using SQL NEIGHBORS function
    sql = """
        SELECT
            e.src_id as steward_id,
            e.edge_label as relation_type
        FROM NEIGHBORS($1, 'IN', NULL) AS e
        WHERE e.edge_label IN (""" + relation_types_filter + """)
    """

    ward_id = ward_user.get("id")
    result = raisin.sql.query(sql, [ward_id])
    # Handle both list results and dict with "rows" key
    if type(result) == "list":
        rows = result
    else:
        rows = result.get("rows", []) if result else []

    # Filter out PARENT_OF relationships if ward is not a minor
    filtered_relations = []
    for rel in rows:
        relation_type = rel.get("relation_type")
        if relation_type == "PARENT_OF" and not is_minor:
            continue
        filtered_relations.append(rel)

    # Get steward user nodes and relation type metadata
    stewards = []
    for rel in filtered_relations:
        steward_id = rel.get("steward_id")
        relation_type = rel.get("relation_type")

        steward_node = raisin.nodes.get_by_id(workspace, steward_id)
        if steward_node and steward_node.get("node_type") == "raisin:User":
            steward_props = steward_node.get("properties", {})

            # Get relation type node for metadata
            rel_type_path = "/relation-types/" + relation_type.lower().replace("_", "-")
            rel_type_node = raisin.nodes.get(workspace, rel_type_path)
            relation_title = relation_type
            if rel_type_node and rel_type_node.get("properties"):
                relation_title = rel_type_node.get("properties", {}).get("title", relation_type)

            stewards.append({
                "user_id": steward_node.get("id"),
                "user_path": steward_node.get("path"),
                "email": steward_props.get("email"),
                "display_name": steward_props.get("display_name"),
                "relation_type": relation_type,
                "relation_title": relation_title
            })

    return {
        "success": True,
        "stewards": stewards
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
