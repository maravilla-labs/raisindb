/**
 * Shared Messaging Utilities
 *
 * Common functions used across messaging handlers for user resolution,
 * message copying, and status management.
 *
 * The `raisin` global is injected at runtime -- do NOT import it.
 */

/**
 * Resolve a user node from path, id (global user UUID), or email.
 *
 * Supports multiple lookup strategies:
 * 1. By global user_id property (identity UUID stored in properties.user_id)
 * 2. By direct path (e.g., /users/alice)
 * 3. By workspace node ID (fallback)
 * 4. By email address
 *
 * @param {string} workspace - The workspace to search in
 * @param {object} opts - Options: { user_path, user_id, email }
 * @returns {object|null} User node dict or null if not found
 */
export async function resolve_user(workspace, { user_path = null, user_id = null, email = null } = {}) {
  // Handle case where user_id is actually a path
  if (user_id && typeof user_id === "string" && user_id.startsWith("/users/")) {
    user_path = user_id;
    user_id = null;
  }

  // Search by global user_id property first (identity UUID)
  if (user_id) {
    const sql = `
      SELECT id, path, name, node_type, properties
      FROM '${workspace}'
      WHERE node_type = 'raisin:User'
        AND properties->>'user_id'::String = $1
      LIMIT 1
    `;
    console.log("info", `[resolve_user] Searching by user_id: ${user_id} in workspace: ${workspace}`);
    const result = await raisin.sql.query(sql, [user_id]);
    console.log("info", `[resolve_user] user_id query result: ${JSON.stringify(result)}`);

    const rows = Array.isArray(result) ? result : (result?.rows ?? []);

    if (rows.length > 0) {
      const row = rows[0];
      return {
        id: row.id,
        path: row.path,
        name: row.name,
        node_type: row.node_type,
        properties: row.properties,
      };
    }
  }

  // Try by path
  if (user_path) {
    const node = await raisin.nodes.get(workspace, user_path);
    if (node) {
      return node;
    }
  }

  // Try direct node ID lookup (workspace node ID) as fallback
  if (user_id) {
    const node = await raisin.nodes.getById(workspace, user_id);
    if (node) {
      return node;
    }
  }

  // Try by email
  if (email) {
    const sql = `
      SELECT id, path, name, node_type, properties
      FROM '${workspace}'
      WHERE node_type = 'raisin:User'
        AND properties->>'email'::String = $1
      LIMIT 1
    `;
    console.log("info", `[resolve_user] Searching by email: ${email}`);
    const result = await raisin.sql.query(sql, [email]);
    console.log("info", `[resolve_user] email query result: ${JSON.stringify(result)}`);

    const rows = Array.isArray(result) ? result : (result?.rows ?? []);

    if (rows.length > 0) {
      const row = rows[0];
      return {
        id: row.id,
        path: row.path,
        name: row.name,
        node_type: row.node_type,
        properties: row.properties,
      };
    }
  }

  return null;
}

/**
 * Copy outbox message to sender's sent folder.
 *
 * Extracts the sender from the message path and creates a copy
 * in their sent folder with updated status.
 *
 * @param {string} workspace - The workspace name
 * @param {object} node - The message node dict with path and properties
 * @returns {string|null} Path to the sent message, or null if copy failed
 */
export async function copy_to_sent_folder(workspace, node) {
  const node_path = node?.path ?? "";
  const node_props = node?.properties ?? {};

  // Extract sender from path (e.g., /users/alice/outbox/msg-001)
  const path_parts = node_path.split("/");
  if (path_parts.length < 5 || path_parts[1] !== "users" || path_parts[3] !== "outbox") {
    return null;
  }

  const sender_username = path_parts[2];
  const message_slug = path_parts[4];

  // Use sent folder (created automatically by raisin:User initial_structure)
  const sent_folder_path = `/users/${sender_username}/sent`;

  // Copy with updated status
  const sent_props = { ...node_props };
  sent_props.status = "sent";
  sent_props.sent_at = new Date().toISOString();

  await raisin.nodes.create(workspace, sent_folder_path, {
    slug: message_slug,
    name: message_slug,
    node_type: node?.node_type ?? "raisin:Message",
    properties: sent_props,
  });

  // Update original outbox message status
  await raisin.nodes.updateProperty(workspace, node_path, "status", "sent");
  await raisin.nodes.updateProperty(workspace, node_path, "sent_at", new Date().toISOString());

  return `${sent_folder_path}/${message_slug}`;
}

/**
 * Update message status and timestamp.
 *
 * @param {string} workspace - The workspace name
 * @param {string} path - Path to the message node
 * @param {string} status - The new status value (pending, sent, delivered, read, failed)
 */
export async function update_message_status(workspace, path, status) {
  await raisin.nodes.updateProperty(workspace, path, "status", status);
  await raisin.nodes.updateProperty(workspace, path, `${status}_at`, new Date().toISOString());
}

/**
 * Extract user path from a message path.
 *
 * @param {string} node_path - Path like /users/alice/outbox/msg-001
 * @returns {string|null} User path like /users/alice, or null
 */
export function user_path_from_message_path(node_path) {
  const parts = node_path.split("/");
  if (parts.length > 2 && parts[1] === "users") {
    return `/users/${parts[2]}`;
  }
  return null;
}

/**
 * Ensure a child folder exists and return its path.
 *
 * @param {string} workspace - The workspace name
 * @param {string} parent_path - Path to the parent node
 * @param {string} slug - Slug for the folder
 * @param {string} title - Display title for the folder
 * @returns {string} The full path to the folder
 */
export async function ensure_folder(workspace, parent_path, slug, title) {
  const folder_path = `${parent_path}/${slug}`;
  const folder = await raisin.nodes.get(workspace, folder_path);
  if (!folder) {
    await raisin.nodes.create(workspace, parent_path, {
      slug,
      name: slug,
      node_type: "raisin:Folder",
      properties: { title },
    });
  }
  return folder_path;
}

/**
 * Get the parent path of a given path.
 *
 * @param {string} path - A path like /users/alice/inbox/msg-001
 * @returns {string} The parent path, or empty string for root
 */
export function parent_path(path) {
  const parts = path.split("/");
  if (parts.length <= 1) {
    return "";
  }
  return parts.slice(0, -1).join("/");
}
