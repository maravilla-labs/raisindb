/**
 * Creates a raisin:User node in the access_control workspace for an identity.
 *
 * This function is called when:
 * - A new user registers
 * - An existing identity is granted access to a repository
 *
 * @param {Object} input - The function input
 * @param {string} input.identity_id - The identity ID from the auth system
 * @param {string} input.email - User's email address
 * @param {string} [input.display_name] - Optional display name
 * @param {string[]} [input.default_roles] - Role IDs to assign (defaults to ["authenticated_user"])
 * @returns {Promise<{success: boolean, user_node_id?: string, user_path?: string, error?: string}>}
 */
async function createUserNode(input) {
    const {
        identity_id,
        email,
        display_name,
        default_roles = ["viewer"]
    } = input;

    if (!identity_id || !email) {
        throw new Error("identity_id and email are required");
    }

    const workspace = "raisin:access_control";
    const usersPath = "/users/internal";

    // Generate a safe node name from email (replace special chars)
    const nodeName = email
        .toLowerCase()
        .replace(/@/g, "-at-")
        .replace(/\./g, "-")
        .replace(/[^a-z0-9-]/g, "-");

    const userPath = `${usersPath}/${nodeName}`;

    try {
        // Check if user node already exists
        const existingUser = await raisin.nodes.get(workspace, userPath);

        if (existingUser) {
            // User already exists - optionally update roles
            console.log(`User node already exists at ${userPath}`);
            return {
                success: true,
                user_node_id: existingUser.id,
                user_path: userPath,
                already_exists: true
            };
        }

        // Resolve role paths for the roles relation
        const rolePaths = [];
        for (const roleId of default_roles) {
            const rolePath = `/roles/${roleId}`;
            const role = await raisin.nodes.get(workspace, rolePath);
            if (role) {
                rolePaths.push(rolePath);
            } else {
                console.warn(`Role not found: ${roleId}`);
            }
        }

        // Create the user node
        const userNode = await raisin.nodes.create(workspace, usersPath, {
            name: nodeName,
            node_type: "raisin:User",
            properties: {
                user_id: identity_id,
                email: email,
                display_name: display_name || email.split("@")[0],
                status: "active",
                roles: rolePaths,
                created_at: new Date().toISOString()
            }
        });

        console.log(`Created user node: ${userPath} with roles: ${rolePaths.join(", ")}`);

        return {
            success: true,
            user_node_id: userNode.id,
            user_path: userPath
        };

    } catch (error) {
        console.error(`Failed to create user node: ${error.message}`);
        throw error;
    }
}

// Export for RaisinDB function runtime
module.exports = { createUserNode };
