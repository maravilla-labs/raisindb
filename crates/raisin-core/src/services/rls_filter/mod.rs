//! Row-Level Security (RLS) filtering for NodeService operations.
//!
//! This module provides filtering functions that apply RLS rules to query results
//! based on the user's permissions. It uses REL (Raisin Expression Language) for
//! condition evaluation.

mod context;
mod matching;

use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, PermissionScope};

use context::evaluate_rel_condition;
use matching::{apply_field_filter, find_matching_permission};

/// Filter a single node based on RLS rules.
///
/// Returns Some(node) if the user can read it, None otherwise.
/// Also applies field-level filtering if the user has partial field access.
pub fn filter_node(node: Node, auth: &AuthContext, scope: &PermissionScope) -> Option<Node> {
    if auth.is_system {
        return Some(node);
    }

    let permissions = match auth.permissions() {
        Some(p) => {
            tracing::debug!(
                node_id = %node.id,
                node_path = %node.path,
                workspace = ?node.workspace,
                permissions_count = p.permissions.len(),
                is_system_admin = p.is_system_admin,
                user_id = %p.user_id,
                "RLS filter_node: checking permissions"
            );
            p
        }
        None => {
            tracing::debug!(
                "No permissions resolved for user, denying access to node {}",
                node.id
            );
            return None;
        }
    };

    if permissions.is_system_admin {
        return Some(node);
    }

    let matching_permission =
        find_matching_permission(&node, &permissions.permissions, scope, Operation::Read);

    match matching_permission {
        Some(permission) => {
            tracing::debug!(
                node_path = %node.path,
                permission_path = %permission.path,
                "RLS: Found matching permission, allowing access"
            );

            if let Some(condition) = &permission.condition {
                if !evaluate_rel_condition(condition, &node, auth) {
                    tracing::debug!("REL condition not satisfied for node {}", node.id);
                    return None;
                }
            }

            let filtered_node = apply_field_filter(node, permission);
            Some(filtered_node)
        }
        None => {
            tracing::info!(
                node_path = %node.path,
                node_workspace = ?node.workspace,
                "RLS: No matching permission found, DENYING access"
            );
            None
        }
    }
}

/// Filter multiple nodes based on RLS rules.
///
/// Returns only the nodes the user can read, with field filtering applied.
pub fn filter_nodes(nodes: Vec<Node>, auth: &AuthContext, scope: &PermissionScope) -> Vec<Node> {
    if auth.is_system {
        return nodes;
    }

    nodes
        .into_iter()
        .filter_map(|node| filter_node(node, auth, scope))
        .collect()
}

/// Check if a user can perform an operation on a node.
pub fn can_perform(
    node: &Node,
    operation: Operation,
    auth: &AuthContext,
    scope: &PermissionScope,
) -> bool {
    if auth.is_system {
        return true;
    }

    let permissions = match auth.permissions() {
        Some(p) => p,
        None => return false,
    };

    if permissions.is_system_admin {
        return true;
    }

    let matching_permission =
        find_matching_permission(node, &permissions.permissions, scope, operation);

    match matching_permission {
        Some(permission) => {
            if let Some(condition) = &permission.condition {
                if !evaluate_rel_condition(condition, node, auth) {
                    return false;
                }
            }
            true
        }
        None => false,
    }
}

/// Check if user can create a node at a path with a given type.
pub fn can_create_at_path(
    path: &str,
    node_type: &str,
    auth: &AuthContext,
    scope: &PermissionScope,
) -> bool {
    tracing::warn!(
        path = path,
        node_type = node_type,
        is_system = auth.is_system,
        user_id = ?auth.user_id,
        is_anonymous = auth.is_anonymous,
        "RLS: checking create permission"
    );

    if auth.is_system {
        tracing::warn!("RLS: system context - allowing create");
        return true;
    }

    let permissions = match auth.permissions() {
        Some(p) => p,
        None => {
            tracing::warn!("RLS: no permissions in auth context - denying create");
            return false;
        }
    };

    if permissions.is_system_admin {
        tracing::warn!("RLS: system_admin permission - allowing create");
        return true;
    }

    for permission in &permissions.permissions {
        if !permission.applies_to_scope(scope) {
            continue;
        }

        if !permission.matches_path(path) {
            continue;
        }

        if !permission.operations.contains(&Operation::Create) {
            continue;
        }

        if let Some(types) = &permission.node_types {
            if !types.contains(&node_type.to_string()) {
                continue;
            }
        }

        return true;
    }

    false
}

#[cfg(test)]
mod tests;
