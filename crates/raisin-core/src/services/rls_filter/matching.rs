//! Permission matching and field filtering logic for RLS.

use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, Permission, PermissionScope};

/// Find the most specific permission that matches a node's path, type, scope, AND operation.
///
/// Checks permissions in order of:
/// 1. Scope match (workspace and branch patterns) - fail-fast
/// 2. Path pattern match
/// 3. Operation match - permission must include the required operation
/// 4. Node type filter match
/// 5. Specificity scoring (most specific wins)
pub(super) fn find_matching_permission<'a>(
    node: &Node,
    permissions: &'a [Permission],
    scope: &PermissionScope,
    operation: Operation,
) -> Option<&'a Permission> {
    let mut best_match: Option<(&Permission, usize)> = None;

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

        // Score by specificity using cached value
        let specificity = permission.path_specificity();

        match &best_match {
            None => best_match = Some((permission, specificity)),
            Some((_, current_score)) if specificity > *current_score => {
                best_match = Some((permission, specificity));
            }
            _ => {}
        }
    }

    best_match.map(|(p, _)| p)
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
