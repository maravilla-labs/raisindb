//! Permission matching and field filtering logic for RLS.

use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, Permission, PermissionScope};

/// EVERY permission that matches a node's path, type, scope and operation, most
/// specific first.
///
/// A list, not a winner, and that is the whole point. Grants are ADDITIVE: a
/// role that may read `/users/**` and a role that may read its own user record
/// are two reasons to allow, not a contest. Taking only the most specific one
/// and denying when ITS condition failed turned the union into a lottery —
/// `authenticated_user` grants `raisin:User` with `node.id == auth.local_user_id`,
/// so an administrator holding an unconditional `/users/** read` saw exactly
/// one user: themselves. Every listing that offers people (a task assignee, a
/// chat participant, the Access app) showed a single candidate, while a direct
/// read of the same node succeeded — because that path resolved a different
/// permission. Measured on a local instance, 2026-08-29.
///
/// Ordering is by path specificity so a caller that must pick ONE (field
/// filtering) still gets the most specific rule that actually applied.
///
/// Checks, in order:
/// 1. Scope match (workspace and branch patterns) - fail-fast
/// 2. Path pattern match
/// 3. Operation match - permission must include the required operation
/// 4. Node type filter match
pub(super) fn matching_permissions<'a>(
    node: &Node,
    permissions: &'a [Permission],
    scope: &PermissionScope,
    operation: Operation,
) -> Vec<&'a Permission> {
    let mut matches: Vec<(&Permission, usize)> = Vec::new();

    for permission in permissions {
        // Check scope FIRST (fail-fast, O(1))
        if !permission.applies_to_scope(scope) {
            continue;
        }

        // Check path pattern using cached matcher
        if !permission.matches_path(&node.path) {
            continue;
        }

        // Check if permission includes the required operation
        if !permission.operations.contains(&operation) {
            continue;
        }

        // Check node type filter (if specified)
        if let Some(types) = &permission.node_types {
            if !types.contains(&node.node_type) {
                continue;
            }
        }

        matches.push((permission, permission.path_specificity()));
    }

    // Most specific first; a stable sort keeps declaration order among equals,
    // so a role's own ordering stays meaningful.
    matches.sort_by(|a, b| b.1.cmp(&a.1));
    matches.into_iter().map(|(p, _)| p).collect()
}

/// The most specific permission that matches, ignoring conditions.
///
/// Kept for callers that only need to know whether a rule EXISTS. Anything
/// deciding access must use [`matching_permissions`] and try each in turn —
/// see the note there.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn find_matching_permission<'a>(
    node: &Node,
    permissions: &'a [Permission],
    scope: &PermissionScope,
    operation: Operation,
) -> Option<&'a Permission> {
    matching_permissions(node, permissions, scope, operation)
        .into_iter()
        .next()
}

/// Apply field filtering to a node based on permission rules.
pub(super) fn apply_field_filter(mut node: Node, permission: &Permission) -> Node {
    // Whitelist takes precedence
    if let Some(allowed_fields) = &permission.fields {
        node.properties
            .retain(|key, _| allowed_fields.contains(key));
        return node;
    }

    // Apply blacklist
    if let Some(denied_fields) = &permission.except_fields {
        node.properties
            .retain(|key, _| !denied_fields.contains(key));
    }

    node
}
