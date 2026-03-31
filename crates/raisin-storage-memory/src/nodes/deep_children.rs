use std::collections::HashMap;

use raisin_error::Result;
use raisin_models as models;

use super::InMemoryNodeRepo;
use crate::NodeKey;

/// Retrieves all descendants of a node up to a maximum depth as a nested structure.
///
/// The returned HashMap maps node paths to DeepNode instances, which recursively
/// contain their own children.
pub(super) async fn deep_children_nested(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_path: &str,
    max_depth: u32,
) -> Result<HashMap<String, models::nodes::DeepNode>> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
    let max_depth = max_depth.min(5);

    let map = repo.nodes.read().await;
    if parent_path.is_empty() {
        return Ok(HashMap::new());
    }

    // collect all nodes under parent up to depth
    let mut all: Vec<models::nodes::Node> = Vec::new();
    let mut frontier = vec![parent_path.to_string()];
    let mut depth = 0;
    while depth < max_depth && !frontier.is_empty() {
        let mut next = Vec::new();
        for p in frontier {
            for (_, n) in map.iter().filter(|(k, n)| {
                k.starts_with(&workspace_prefix) && n.parent_path().as_deref() == Some(p.as_str())
            }) {
                all.push(n.clone());
                next.push(n.path.clone());
            }
        }
        depth += 1;
        frontier = next;
    }

    // group by parent path (derive PATH from node.path, not use node.parent NAME)
    let mut by_parent: HashMap<String, Vec<models::nodes::Node>> = HashMap::new();
    for n in all {
        by_parent
            .entry(n.parent_path().unwrap_or_default())
            .or_default()
            .push(n);
    }

    fn build(
        by_parent: &HashMap<String, Vec<models::nodes::Node>>,
        parent: &str,
    ) -> HashMap<String, models::nodes::DeepNode> {
        let mut out = HashMap::new();
        if let Some(children) = by_parent.get(parent) {
            for child in children {
                let mut dn = models::nodes::DeepNode::new(child.clone());
                dn.children = build(by_parent, &child.path);
                out.insert(child.path.clone(), dn);
            }
        }
        out
    }

    Ok(build(&by_parent, parent_path))
}

/// Retrieves all descendants of a node up to a maximum depth as a flat Vec.
///
/// Unlike deep_children_nested, this returns a simple Vec of nodes
/// without nested structure.
pub(super) async fn deep_children_flat(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_path: &str,
    max_depth: u32,
) -> Result<Vec<models::nodes::Node>> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
    let max_depth = max_depth.min(5);

    let map = repo.nodes.read().await;
    if parent_path.is_empty() {
        return Ok(Vec::new());
    }

    let mut result: Vec<models::nodes::Node> = Vec::new();
    let mut frontier = vec![parent_path.to_string()];
    let mut depth = 0;
    while depth < max_depth && !frontier.is_empty() {
        let mut next = Vec::new();
        for p in frontier {
            for (_, n) in map.iter().filter(|(k, n)| {
                k.starts_with(&workspace_prefix) && n.parent_path().as_deref() == Some(p.as_str())
            }) {
                result.push(n.clone());
                next.push(n.path.clone());
            }
        }
        depth += 1;
        frontier = next;
    }
    Ok(result)
}

/// Retrieves all descendants of a node up to a maximum depth as a DX-friendly array.
///
/// Returns a Vec<NodeWithChildren> where each node has its children expanded as full nodes,
/// not just string names. This is much more frontend-friendly than HashMap<String, DeepNode>.
pub(super) async fn deep_children_array(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_path: &str,
    max_depth: u32,
) -> Result<Vec<models::nodes::NodeWithChildren>> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
    let max_depth = max_depth.min(5);

    let map = repo.nodes.read().await;
    if parent_path.is_empty() {
        return Ok(Vec::new());
    }

    // First, collect all nodes under parent up to depth (same as deep_children_nested)
    let mut all = Vec::new();
    let mut frontier = vec![parent_path.to_string()];
    let mut depth = 0;

    while depth < max_depth && !frontier.is_empty() {
        let mut next = Vec::new();
        for p in frontier {
            for (_, n) in map.iter().filter(|(k, n)| {
                k.starts_with(&workspace_prefix) && n.parent_path().as_deref() == Some(p.as_str())
            }) {
                all.push(n.clone());
                next.push(n.path.clone());
            }
        }
        depth += 1;
        frontier = next;
    }

    // Group nodes by their parent path (derive PATH from node.path, not use node.parent NAME)
    let mut by_parent: HashMap<String, Vec<models::nodes::Node>> = HashMap::new();
    for n in all {
        by_parent
            .entry(n.parent_path().unwrap_or_default())
            .or_default()
            .push(n);
    }

    // Also need a map of all nodes to get parent's children order
    let all_nodes: HashMap<String, models::nodes::Node> = map
        .iter()
        .filter(|(k, _)| k.starts_with(&workspace_prefix))
        .map(|(_, n)| (n.path.clone(), n.clone()))
        .collect();

    // Sort each group of children according to their parent's order
    for (parent_path, children) in by_parent.iter_mut() {
        if let Some(parent_node) = all_nodes.get(parent_path) {
            let order = &parent_node.children;
            children.sort_by(|a, b| {
                let ia = order.iter().position(|n| n == &a.name);
                let ib = order.iter().position(|n| n == &b.name);
                match (ia, ib) {
                    (Some(ia), Some(ib)) => ia.cmp(&ib),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.name.cmp(&b.name),
                }
            });
        } else {
            // No parent found (root level), sort alphabetically
            children.sort_by(|a, b| a.name.cmp(&b.name));
        }
    }

    // Recursive function to build NodeWithChildren tree with depth tracking
    fn build_array(
        by_parent: &HashMap<String, Vec<models::nodes::Node>>,
        parent: &str,
        current_depth: usize,
        max_depth: usize,
    ) -> Vec<models::nodes::NodeWithChildren> {
        let mut result = Vec::new();
        if let Some(children) = by_parent.get(parent) {
            for child in children {
                // Create node with string children initially (from the Node's children field)
                let mut node_with_children = models::nodes::NodeWithChildren::new(child.clone());

                // Only expand children if we haven't reached max depth
                if current_depth < max_depth - 1 {
                    let child_array =
                        build_array(by_parent, &child.path, current_depth + 1, max_depth);
                    if !child_array.is_empty() {
                        node_with_children = node_with_children.with_children(child_array);
                    }
                }
                // Otherwise, leave children as strings (which is already set from new())

                result.push(node_with_children);
            }
        }
        result
    }

    // Build the array starting from the parent path (depth 0 is the parent itself)
    Ok(build_array(&by_parent, parent_path, 0, max_depth as usize))
}
