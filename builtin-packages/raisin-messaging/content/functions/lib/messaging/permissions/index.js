/**
 * Messaging Permissions Module
 *
 * Provides permission checking for messaging based on relationships, group membership,
 * roles, and other configurable criteria.
 *
 * The `raisin` global is injected at runtime -- do NOT import it.
 */

/**
 * Check if a sender can send a message to a recipient.
 *
 * @param {object} input - { sender_id, recipient_id, message_type, workspace }
 * @returns {object} { allowed: boolean, reason?: string }
 */
export async function check_messaging_permission(input) {
  const sender_id = input.sender_id;
  const recipient_id = input.recipient_id;
  const message_type = input.message_type ?? "chat";
  const workspace = input.workspace ?? "raisin:access_control";

  if (!sender_id || !recipient_id) {
    throw new Error("Missing sender_id or recipient_id");
  }

  // Auto-allow messaging to/from agents (agent IDs start with "agent:")
  if (_is_agent_id(sender_id) || _is_agent_id(recipient_id)) {
    return { allowed: true };
  }

  // Get messaging config
  const config = await _get_messaging_config(workspace);

  // Check if messaging is enabled
  if (!(config.enabled ?? true)) {
    return {
      allowed: false,
      reason: "Messaging is disabled",
    };
  }

  // Check if recipient has blocked sender
  if (config.blocked_users_prevent_messaging ?? true) {
    if (await _is_blocked(recipient_id, sender_id, workspace)) {
      return {
        allowed: false,
        reason: "You have been blocked by this user",
      };
    }
  }

  // Get the appropriate permission rules based on message type
  let permissions;
  if (message_type === "chat" || message_type === "direct_message") {
    permissions = config.chat_permissions ?? { mode: "any_of", rules: [{ type: "always" }] };
  } else if (message_type === "task_assignment") {
    permissions = config.task_permissions ?? { mode: "any_of", rules: [{ type: "always" }] };
  } else if (message_type === "system_notification") {
    permissions = config.notification_permissions ?? { mode: "any_of", rules: [{ type: "always" }] };
  } else {
    // Default to chat permissions for unknown types
    permissions = config.chat_permissions ?? { mode: "any_of", rules: [{ type: "always" }] };
  }

  // Evaluate permission rules
  return await _evaluate_permissions(sender_id, recipient_id, permissions, workspace);
}

/**
 * Get the messaging configuration from the config node.
 * @private
 */
async function _get_messaging_config(workspace) {
  const config_node = await raisin.nodes.get(workspace, "/config/messaging");
  if (config_node && config_node.properties) {
    return config_node.properties;
  }

  // Return defaults if no config exists
  return {
    enabled: true,
    chat_permissions: {
      mode: "any_of",
      rules: [{ type: "always" }],
    },
    task_permissions: {
      mode: "any_of",
      rules: [{ type: "always" }],
    },
    notification_permissions: {
      mode: "any_of",
      rules: [{ type: "always" }],
    },
    blocked_users_prevent_messaging: true,
  };
}

/**
 * Check if user_id has blocked blocked_user_id.
 *
 * Uses GRAPH_TABLE with Cypher MATCH to query the BLOCKS relationship.
 * Note: BLOCKS is directional -- only checks if user_id blocked blocked_user_id.
 * @private
 */
async function _is_blocked(user_id, blocked_user_id, workspace) {
  const user_is_path = typeof user_id === "string" && user_id.startsWith("/");
  const blocked_is_path = typeof blocked_user_id === "string" && blocked_user_id.startsWith("/");

  const blocker_cond = user_is_path
    ? `blocker.path = '${user_id}'`
    : `blocker.id = '${user_id}'`;

  const blockee_cond = blocked_is_path
    ? `blockee.path = '${blocked_user_id}'`
    : `blockee.id = '${blocked_user_id}'`;

  const where_clause = `${blocker_cond} AND ${blockee_cond}`;

  const sql = `SELECT * FROM GRAPH_TABLE(MATCH (blocker)-[:BLOCKS]->(blockee) WHERE ${where_clause} COLUMNS (blockee.id AS id)) AS g LIMIT 1`;

  const result = await raisin.sql.query(sql, []);
  const rows = Array.isArray(result) ? result : (result?.rows ?? []);

  return rows.length > 0;
}

/**
 * Evaluate permission rules.
 * @private
 */
async function _evaluate_permissions(sender_id, recipient_id, permissions, workspace) {
  const mode = permissions.mode ?? "any_of";
  const rules = permissions.rules ?? [];

  if (!rules.length) {
    // No rules means allow by default
    return { allowed: true };
  }

  for (const rule of rules) {
    const rule_type = rule.type;
    const result = await _evaluate_rule(sender_id, recipient_id, rule, workspace);

    // Short-circuit evaluation
    if (mode === "any_of" && result) {
      return { allowed: true };
    } else if (mode === "all_of" && !result) {
      return {
        allowed: false,
        reason: `Permission rule not satisfied: ${rule_type}`,
      };
    }
  }

  if (mode === "any_of") {
    // None of the rules matched
    return {
      allowed: false,
      reason: "No permission rule satisfied",
    };
  }
  // all_of mode and all rules passed
  return { allowed: true };
}

/**
 * Evaluate a single permission rule.
 * @private
 */
async function _evaluate_rule(sender_id, recipient_id, rule, workspace) {
  const rule_type = rule.type;

  if (rule_type === "always") {
    return true;
  }

  if (rule_type === "never") {
    return false;
  }

  if (rule_type === "relationship") {
    const relation = rule.relation;
    if (!relation) {
      return false;
    }
    return await _has_relationship(sender_id, recipient_id, relation);
  }

  if (rule_type === "same_group") {
    const group_type = rule.group_type;
    return await _in_same_group(sender_id, recipient_id, group_type, workspace);
  }

  if (rule_type === "same_role") {
    const role = rule.role;
    return await _has_same_role(sender_id, recipient_id, role, workspace);
  }

  if (rule_type === "sender_has_role") {
    const role = rule.role;
    return await _user_has_role(sender_id, role, workspace);
  }

  if (rule_type === "recipient_has_role") {
    const role = rule.role;
    return await _user_has_role(recipient_id, role, workspace);
  }

  // Unknown rule type, default to false
  return false;
}

/**
 * Check if user_a has a relationship with user_b (both directions).
 * @private
 */
async function _has_relationship(user_a_id, user_b_id, relation_type) {
  // Normalize relation type to uppercase with underscores for edge label
  const relation_upper = relation_type.toUpperCase().replace(/-/g, "_");

  const user_a_is_path = typeof user_a_id === "string" && user_a_id.startsWith("/");
  const user_b_is_path = typeof user_b_id === "string" && user_b_id.startsWith("/");

  // Build the appropriate WHERE clause based on path vs ID
  let where_clause;
  if (user_a_is_path && user_b_is_path) {
    where_clause = `a.path = '${user_a_id}' AND b.path = '${user_b_id}'`;
  } else if (!user_a_is_path && !user_b_is_path) {
    where_clause = `a.id = '${user_a_id}' AND b.id = '${user_b_id}'`;
  } else {
    const a_cond = user_a_is_path ? `a.path = '${user_a_id}'` : `a.id = '${user_a_id}'`;
    const b_cond = user_b_is_path ? `b.path = '${user_b_id}'` : `b.id = '${user_b_id}'`;
    where_clause = `${a_cond} AND ${b_cond}`;
  }

  // Check A -> B direction
  const sql_forward = `SELECT * FROM GRAPH_TABLE(MATCH (a)-[:${relation_upper}]->(b) WHERE ${where_clause} COLUMNS (b.id AS id)) AS g LIMIT 1`;

  let result = await raisin.sql.query(sql_forward, []);
  let rows = Array.isArray(result) ? result : (result?.rows ?? []);

  if (rows.length > 0) {
    return true;
  }

  // Check B -> A direction (for bidirectional relationships)
  let where_clause_rev;
  if (user_a_is_path && user_b_is_path) {
    where_clause_rev = `a.path = '${user_b_id}' AND b.path = '${user_a_id}'`;
  } else if (!user_a_is_path && !user_b_is_path) {
    where_clause_rev = `a.id = '${user_b_id}' AND b.id = '${user_a_id}'`;
  } else {
    const a_cond = user_b_is_path ? `a.path = '${user_b_id}'` : `a.id = '${user_b_id}'`;
    const b_cond = user_a_is_path ? `b.path = '${user_a_id}'` : `b.id = '${user_a_id}'`;
    where_clause_rev = `${a_cond} AND ${b_cond}`;
  }

  const sql_reverse = `SELECT * FROM GRAPH_TABLE(MATCH (a)-[:${relation_upper}]->(b) WHERE ${where_clause_rev} COLUMNS (b.id AS id)) AS g LIMIT 1`;

  result = await raisin.sql.query(sql_reverse, []);
  rows = Array.isArray(result) ? result : (result?.rows ?? []);

  return rows.length > 0;
}

/**
 * Check if two users are in the same group.
 * @private
 */
async function _in_same_group(user_a_id, user_b_id, group_type, workspace) {
  const user_a = await _get_user_node(user_a_id);
  const user_b = await _get_user_node(user_b_id);

  if (!user_a || !user_b) {
    return false;
  }

  const props_a = user_a.properties ?? {};
  const props_b = user_b.properties ?? {};

  const groups_a = props_a.groups ?? [];
  const groups_b = props_b.groups ?? [];

  if (!groups_a.length || !groups_b.length) {
    return false;
  }

  let groups_a_list = Array.isArray(groups_a) ? groups_a : [];
  let groups_b_list = Array.isArray(groups_b) ? groups_b : [];

  // If group_type filter is specified, filter the groups
  if (group_type) {
    const group_type_lower = group_type.toLowerCase();
    groups_a_list = groups_a_list.filter((g) => g.toLowerCase().includes(group_type_lower));
    groups_b_list = groups_b_list.filter((g) => g.toLowerCase().includes(group_type_lower));
  }

  // Check for intersection
  for (const g of groups_a_list) {
    if (groups_b_list.includes(g)) {
      return true;
    }
  }
  return false;
}

/**
 * Check if two users have the same role.
 * @private
 */
async function _has_same_role(user_a_id, user_b_id, role, workspace) {
  if (role) {
    // Check if both users have the specific role
    return (await _user_has_role(user_a_id, role, workspace)) && (await _user_has_role(user_b_id, role, workspace));
  }

  // Check if users share any role
  const user_a = await _get_user_node(user_a_id);
  const user_b = await _get_user_node(user_b_id);

  if (!user_a || !user_b) {
    return false;
  }

  const props_a = user_a.properties ?? {};
  const props_b = user_b.properties ?? {};

  const roles_a = props_a.roles ?? [];
  const roles_b = props_b.roles ?? [];

  for (const r of roles_a) {
    if (roles_b.includes(r)) {
      return true;
    }
  }
  return false;
}

/**
 * Check if a user has a specific role.
 * @private
 */
async function _user_has_role(user_id, role, workspace) {
  if (!role) {
    return false;
  }

  const user = await _get_user_node(user_id);
  if (!user) {
    return false;
  }

  const props = user.properties ?? {};
  const roles = props.roles ?? [];

  if (!roles.length) {
    return false;
  }

  if (Array.isArray(roles)) {
    return roles.includes(role);
  }

  return false;
}

/**
 * Get a user node by ID.
 * @private
 */
async function _get_user_node(user_id) {
  // Try as a workspace-relative path (e.g., "/users/alice")
  if (typeof user_id === "string" && user_id.startsWith("/")) {
    const node = await raisin.nodes.get("raisin:access_control", user_id);
    if (node) {
      return node;
    }
  }

  // Try as a node ID (UUID)
  const sql = `
    SELECT id, path, node_type, properties
    FROM nodes
    WHERE id = $1
  `;
  const result = await raisin.sql.query(sql, [user_id]);
  const rows = Array.isArray(result) ? result : (result?.rows ?? []);

  if (rows.length > 0) {
    return rows[0];
  }

  return null;
}

/**
 * Check if an entity ID is an agent ID (starts with 'agent:').
 * @private
 */
function _is_agent_id(entity_id) {
  if (!entity_id) {
    return false;
  }
  return typeof entity_id === "string" && entity_id.startsWith("agent:");
}
