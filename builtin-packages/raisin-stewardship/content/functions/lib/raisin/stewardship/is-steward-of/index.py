"""
Checks if user A is a steward of user B.

Returns true if there is a stewardship-implying relationship (PARENT_OF, GUARDIAN_OF)
from A to B. For PARENT_OF relationships, considers the minor status of B based on
birth_date and minor_age_threshold configuration.
"""

def is_steward_of(input):
    """Check if user A is a steward of user B.

    Args:
        input: dict with steward_user_path, ward_user_path, and optional workspace

    Returns:
        dict with success, is_steward, relation_type, relation_title, or error
    """
    steward_user_path = input.get("steward_user_path")
    ward_user_path = input.get("ward_user_path")
    workspace = input.get("workspace", "raisin:access_control")

    if not steward_user_path or not ward_user_path:
        fail("Both steward_user_path and ward_user_path are required")

    # Get the steward user node
    steward_user = raisin.nodes.get(workspace, steward_user_path)
    if not steward_user:
        fail("Steward user not found at path: " + steward_user_path)

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

    # Query for a relationship from steward to ward
    sql = """
        SELECT
            e.edge_label as relation_type
        FROM NEIGHBORS($1, 'OUT', NULL) AS e
        WHERE e.dst_id = $2
          AND e.edge_label IN (""" + relation_types_filter + """)
        LIMIT 1
    """

    steward_id = steward_user.get("id")
    ward_id = ward_user.get("id")

    result = raisin.sql.query(sql, [steward_id, ward_id])
    # Handle both list results and dict with "rows" key
    if type(result) == "list":
        rows = result
    else:
        rows = result.get("rows", []) if result else []

    if len(rows) == 0:
        return {
            "success": True,
            "is_steward": False
        }

    relation = rows[0]
    relation_type = relation.get("relation_type")

    # For PARENT_OF relationships, only valid if ward is a minor
    if relation_type == "PARENT_OF" and not is_minor:
        return {
            "success": True,
            "is_steward": False
        }

    # Get relation type node for metadata
    relation_title = relation_type
    rel_type_path = "/relation-types/" + relation_type.lower().replace("_", "-")
    rel_type_node = raisin.nodes.get(workspace, rel_type_path)
    if rel_type_node and rel_type_node.get("properties"):
        relation_title = rel_type_node.get("properties", {}).get("title", relation_type)

    return {
        "success": True,
        "is_steward": True,
        "relation_type": relation_type,
        "relation_title": relation_title
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
