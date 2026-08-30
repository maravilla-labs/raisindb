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
use raisin_rel::eval::RelationResolver;

use context::{evaluate_rel_condition, evaluate_rel_condition_async};
use matching::{apply_field_filter, matching_permissions};

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

    // EVERY matching grant is a reason to allow, most specific first. The one
    // that actually grants access is also the one whose field filter applies.
    let candidates = matching_permissions(&node, &permissions.permissions, scope, Operation::Read);
    for permission in &candidates {
        if let Some(condition) = &permission.condition {
            if !evaluate_rel_condition(condition, &node, auth) {
                tracing::debug!(
                    node_path = %node.path,
                    permission_path = %permission.path,
                    "RLS: condition not satisfied - trying the next matching permission"
                );
                continue;
            }
        }
        tracing::debug!(
            node_path = %node.path,
            permission_path = %permission.path,
            "RLS: Found matching permission, allowing access"
        );
        return Some(apply_field_filter(node, permission));
    }

    tracing::info!(
        node_path = %node.path,
        node_workspace = ?node.workspace,
        candidates = candidates.len(),
        "RLS: No matching permission allowed this node, DENYING access"
    );
    None
}

/// Which of `candidates` this caller may read *at all*, as an UPPER BOUND.
///
/// The search table functions need the readable workspace set before they query
/// an index, for two reasons that are not the same reason:
///
/// * **rank honesty** — Reciprocal Rank Fusion consumes *ranks*. A rank computed
///   over a pool containing rows the caller can never see is a wrong rank,
///   printed in a column named `fulltext_rank`. Over-fetching does not repair
///   it; narrowing the pool before ranking does.
/// * **recall** — every candidate slot spent on a workspace the caller cannot
///   read is a slot that produced no row.
///
/// It lives here, next to [`filter_node`], because its early returns must MIRROR
/// that function's exactly. They diverged once already in review: `permissions()
/// == None` looks like "no restrictions" and is in fact a DENY here, matching
/// `filter_node`. A resolver that got that backwards would hand a caller with
/// unresolved permissions the whole repository.
///
/// # This is not authorisation
///
/// Workspace is ONE of four RLS dimensions (workspace, path, node_type, REL
/// condition) plus field filtering. A workspace appearing in this set means only
/// that *some* node in it might be readable — never that any particular node is.
/// Every row still goes through [`filter_node`] / `filter_node_async`. Deleting
/// the per-row check because "the scope is already restricted" is the way this
/// becomes a read bypass.
pub fn readable_workspaces(
    auth: Option<&AuthContext>,
    candidates: &[String],
    branch: &str,
) -> ReadableWorkspaces {
    // No identity at all: the system/internal caller convention every scan
    // executor and GRAPH_TABLE already use. Unfiltered.
    let Some(auth) = auth else {
        return ReadableWorkspaces::All;
    };

    if auth.is_system {
        return ReadableWorkspaces::All;
    }

    // Deny, exactly as `filter_node` does above. NOT "no restrictions".
    let Some(permissions) = auth.permissions() else {
        return ReadableWorkspaces::Only(Vec::new());
    };

    if permissions.is_system_admin {
        return ReadableWorkspaces::All;
    }

    // A permission's workspace is a GLOB, so this tests each candidate against
    // each grant rather than enumerating the grants — `content-*` names no
    // workspace but matches many. Pure CPU, O(candidates x permissions), and the
    // matchers are compiled once and cached on the Permission.
    let readable: Vec<String> = candidates
        .iter()
        .filter(|ws| {
            let scope = PermissionScope::new(ws.as_str(), branch);
            permissions.permissions.iter().any(|p| {
                p.operations.contains(&Operation::Read) && p.scope_matcher().matches(&scope)
            })
        })
        .cloned()
        .collect();

    ReadableWorkspaces::Only(readable)
}

/// Result of [`readable_workspaces`].
///
/// `All` and an empty `Only` are DIFFERENT and must stay different. Modelling
/// this as `Option<Vec<String>>` and collapsing the empty vector to `None` turns
/// "may read nothing" into "search everything" — a complete read-path bypass one
/// keystroke away, in the same area that just closed one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadableWorkspaces {
    /// Every candidate is readable; push no workspace filter.
    All,
    /// Exactly these are readable. An EMPTY vector means nothing is.
    Only(Vec<String>),
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

/// Async counterpart to [`filter_node`] that can evaluate `RELATES … VIA`
/// graph-relationship conditions via the supplied `resolver`.
///
/// When `resolver` is `None`, behaves exactly like [`filter_node`] (RELATES
/// conditions fail closed).
pub async fn filter_node_async(
    node: Node,
    auth: &AuthContext,
    scope: &PermissionScope,
    resolver: Option<&dyn RelationResolver>,
) -> Option<Node> {
    if auth.is_system {
        return Some(node);
    }

    let permissions = auth.permissions()?;

    if permissions.is_system_admin {
        return Some(node);
    }

    for permission in matching_permissions(&node, &permissions.permissions, scope, Operation::Read)
    {
        if let Some(condition) = &permission.condition {
            if !evaluate_rel_condition_async(condition, &node, auth, resolver).await {
                continue;
            }
        }
        return Some(apply_field_filter(node, permission));
    }
    None
}

/// Async counterpart to [`filter_nodes`]. Evaluates each node sequentially so a
/// single graph resolver reference can be shared across the batch.
pub async fn filter_nodes_async(
    nodes: Vec<Node>,
    auth: &AuthContext,
    scope: &PermissionScope,
    resolver: Option<&dyn RelationResolver>,
) -> Vec<Node> {
    if auth.is_system {
        return nodes;
    }

    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        if let Some(filtered) = filter_node_async(node, auth, scope, resolver).await {
            out.push(filtered);
        }
    }
    out
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

    for permission in matching_permissions(node, &permissions.permissions, scope, operation) {
        match &permission.condition {
            Some(condition) if !evaluate_rel_condition(condition, node, auth) => continue,
            _ => return true,
        }
    }
    false
}

/// Async counterpart to [`can_perform`] that can evaluate `RELATES … VIA`
/// graph-relationship conditions via the supplied `resolver`.
///
/// When `resolver` is `None`, behaves exactly like [`can_perform`] (RELATES
/// conditions fail closed).
pub async fn can_perform_async(
    node: &Node,
    operation: Operation,
    auth: &AuthContext,
    scope: &PermissionScope,
    resolver: Option<&dyn RelationResolver>,
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

    for permission in matching_permissions(node, &permissions.permissions, scope, operation) {
        if let Some(condition) = &permission.condition {
            if !evaluate_rel_condition_async(condition, node, auth, resolver).await {
                continue;
            }
        }
        return true;
    }
    false
}

/// Check if user can create a node at a path with a given type.
///
/// Permission `condition`s are evaluated against a candidate node built from
/// `path`/`node_type` (no properties). Conditions that only reference
/// `node.path` / `node.node_type` (e.g. `node.path.startsWith(auth.home)`)
/// work for creates; conditions on fields that don't exist yet fail closed.
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

    // Candidate node for condition evaluation (lazily built once needed).
    let mut candidate: Option<Node> = None;

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

        if let Some(condition) = &permission.condition {
            let node = candidate.get_or_insert_with(|| candidate_node(path, node_type));
            if !evaluate_rel_condition(condition, node, auth) {
                tracing::debug!(
                    path = path,
                    condition = %condition,
                    "RLS: create condition not satisfied - trying next permission"
                );
                continue;
            }
        }

        return true;
    }

    false
}

/// Async counterpart to [`can_create_at_path`] that can evaluate `RELATES … VIA`
/// graph-relationship conditions via the supplied `resolver`.
///
/// When `resolver` is `None`, behaves exactly like [`can_create_at_path`].
pub async fn can_create_at_path_async(
    path: &str,
    node_type: &str,
    auth: &AuthContext,
    scope: &PermissionScope,
    resolver: Option<&dyn RelationResolver>,
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

    // Candidate node for condition evaluation (lazily built once needed).
    let mut candidate: Option<Node> = None;

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

        if let Some(condition) = &permission.condition {
            let node = candidate.get_or_insert_with(|| candidate_node(path, node_type));
            if !evaluate_rel_condition_async(condition, node, auth, resolver).await {
                continue;
            }
        }

        return true;
    }

    false
}

/// Build a minimal candidate node so REL conditions can be evaluated for
/// creates. Only `path`, `name` and `node_type` are meaningful; conditions on
/// fields that don't exist yet (owner, created_by, properties) fail closed.
fn candidate_node(path: &str, node_type: &str) -> Node {
    Node {
        id: String::new(),
        name: path.rsplit('/').next().unwrap_or_default().to_string(),
        path: path.to_string(),
        parent: None,
        node_type: node_type.to_string(),
        children: Vec::new(),
        order_key: String::new(),
        has_children: None,
        properties: Default::default(),
        archetype: None,
        created_at: None,
        updated_at: None,
        created_by: None,
        updated_by: None,
        published_at: None,
        published_by: None,
        version: 0,
        translations: None,
        tenant_id: None,
        workspace: None,
        owner_id: None,
        relations: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
