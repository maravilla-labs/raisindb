//! Variable-Length Path Matching for PGQ/GRAPH_TABLE
//!
//! Handles matching of variable-length relationship patterns using DFS traversal.
//! Patterns like: (a)-[:TYPE*]->(b) or (a)-[:TYPE*1..3]->(b)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use raisin_sql::ast::{Direction, NodePattern, PathQuantifier, RelationshipPattern};
use raisin_storage::{RelationRepository, Storage};

use super::matches_label;
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::pgq::context::PgqContext;
use crate::physical_plan::pgq::types::{NodeInfo, RelationInfo, VariableBinding};

/// A graph node identifier: (workspace, node_id)
type GraphNodeId = (String, String);

/// A PGQ graph edge: (target_workspace, target_id, target_node_type, relation_type, weight)
type PgqGraphEdge = (String, String, String, String, Option<f32>);

/// PGQ adjacency list mapping nodes to their outgoing edges.
type PgqGraphAdjacency = HashMap<GraphNodeId, Vec<PgqGraphEdge>>;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Maximum paths to return (prevents runaway queries)
const MAX_PATHS: usize = 10000;

/// Information about a path found during DFS
#[derive(Debug, Clone)]
struct PathInfo {
    /// Nodes in the path: (id, workspace, node_type)
    nodes: Vec<(String, String, String)>,
    /// Relationships in the path
    relationships: Vec<RelationInfo>,
    /// Path length (number of hops)
    length: usize,
}

impl PathInfo {
    /// Create a new path starting at a node
    fn new(id: String, workspace: String, node_type: String) -> Self {
        Self {
            nodes: vec![(id, workspace, node_type)],
            relationships: vec![],
            length: 0,
        }
    }

    /// Extend the path with a new hop
    fn extend(&self, rel: RelationInfo, id: String, workspace: String, node_type: String) -> Self {
        let mut nodes = self.nodes.clone();
        nodes.push((id, workspace, node_type));
        let mut relationships = self.relationships.clone();
        relationships.push(rel);
        Self {
            nodes,
            relationships,
            length: self.length + 1,
        }
    }
}

/// Execute variable-length relationship pattern: (a)-[:TYPE*min..max]->(b)
///
/// Uses DFS (Depth-First Search) to find all paths within the specified length range.
/// Implements cycle detection to prevent infinite loops.
///
/// # Arguments
/// * `source_pattern` - Source node pattern
/// * `rel_pattern` - Relationship pattern with quantifier specification
/// * `target_pattern` - Target node pattern
/// * `bindings` - Existing variable bindings (for chained patterns)
/// * `storage` - Storage backend
/// * `context` - PGQ execution context
///
/// # Returns
/// New bindings with matched variable-length paths
pub async fn execute_variable_length_pattern<S: Storage>(
    source_pattern: &NodePattern,
    rel_pattern: &RelationshipPattern,
    target_pattern: &NodePattern,
    bindings: Vec<VariableBinding>,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<Vec<VariableBinding>> {
    let quantifier = rel_pattern.quantifier.as_ref().ok_or_else(|| {
        ExecutionError::Validation("Variable-length pattern requires a quantifier".to_string())
    })?;

    let min_depth = quantifier.min;
    let max_depth = quantifier.max.unwrap_or(PathQuantifier::DEFAULT_MAX);

    tracing::info!(
        "PGQ: Executing variable-length pattern: min={}, max={}, direction={:?}",
        min_depth,
        max_depth,
        rel_pattern.direction
    );

    // Warn on expensive queries
    if max_depth > 5 {
        tracing::warn!("PGQ: Variable-length pattern with depth > 5 may be expensive");
    }

    // Extract relation type filter
    let relation_type_filter = rel_pattern.types.first().map(|s| s.as_str());

    // Get all relationships once (cached for the duration of this query)
    tracing::debug!(
        "PGQ: Fetching all relationships of type {:?}...",
        relation_type_filter
    );

    let all_relationships = storage
        .relations()
        .scan_relations_global(
            raisin_storage::BranchScope::new(&context.tenant_id, &context.repo_id, &context.branch),
            relation_type_filter,
            context.revision.as_ref(),
        )
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

    tracing::info!(
        "PGQ: Fetched {} relationships for variable-length traversal",
        all_relationships.len()
    );

    if all_relationships.is_empty() {
        return Ok(vec![]);
    }

    // Build adjacency list for efficient traversal
    // Key: (workspace, node_id)
    // Value: Vec<(target_workspace, target_id, target_node_type, relation_type, weight)>
    let mut adjacency: PgqGraphAdjacency = HashMap::new();

    // Also build reverse adjacency for LEFT direction
    let mut reverse_adjacency: PgqGraphAdjacency = HashMap::new();

    for (src_workspace, src_id, tgt_workspace, tgt_id, rel_ref) in &all_relationships {
        // Forward adjacency (for RIGHT direction)
        let forward_key = (src_workspace.clone(), src_id.clone());
        let forward_value = (
            tgt_workspace.clone(),
            tgt_id.clone(),
            rel_ref.target_node_type.clone(),
            rel_ref.relation_type.clone(),
            rel_ref.weight,
        );
        adjacency
            .entry(forward_key)
            .or_default()
            .push(forward_value);

        // Reverse adjacency (for LEFT direction)
        let reverse_key = (tgt_workspace.clone(), tgt_id.clone());
        let reverse_value = (
            src_workspace.clone(),
            src_id.clone(),
            rel_ref.source_node_type.clone(),
            rel_ref.relation_type.clone(),
            rel_ref.weight,
        );
        reverse_adjacency
            .entry(reverse_key)
            .or_default()
            .push(reverse_value);
    }

    tracing::debug!(
        "PGQ: Built adjacency list with {} forward nodes, {} reverse nodes",
        adjacency.len(),
        reverse_adjacency.len()
    );

    // Choose which adjacency to use based on direction
    let adj = match rel_pattern.direction {
        Direction::Right => &adjacency,
        Direction::Left => &reverse_adjacency,
        Direction::Any => {
            // For bidirectional, we'd need to merge both - for now use forward
            tracing::warn!("PGQ: Bidirectional variable-length paths use forward direction only");
            &adjacency
        }
    };

    let mut result_bindings = Vec::new();
    let mut total_paths = 0;

    // For each existing binding, find paths
    for binding in bindings {
        // Check if source pattern has a variable
        if source_pattern.variable.is_none() && target_pattern.variable.is_none() {
            return Err(ExecutionError::Validation(
                "Variable-length patterns require at least one node variable".to_string(),
            ));
        }

        // Check if source node is already bound
        if let Some(source_var) = &source_pattern.variable {
            if let Some(source_node) = binding.get_node(source_var) {
                // Source is already bound - find paths from this node
                let start_key = (source_node.workspace.clone(), source_node.id.clone());

                tracing::debug!(
                    "PGQ: Finding paths from bound source: {}:{}",
                    source_node.workspace,
                    source_node.id
                );

                // Perform DFS from this source
                let mut visited = HashSet::new();
                let initial_path = PathInfo::new(
                    source_node.id.clone(),
                    source_node.workspace.clone(),
                    source_node.node_type.clone(),
                );

                let paths = dfs_find_paths(
                    &start_key,
                    adj,
                    &mut visited,
                    initial_path,
                    min_depth,
                    max_depth,
                    0,
                    MAX_PATHS - total_paths,
                    &target_pattern.labels,
                )?;

                tracing::debug!("PGQ: Found {} paths from this source", paths.len());
                total_paths += paths.len();

                // Convert paths to bindings
                for path in paths {
                    let mut new_binding = binding.clone();

                    // Bind target node (last node in path)
                    if let Some(target_var) = &target_pattern.variable {
                        if let Some((target_id, target_workspace, target_type)) = path.nodes.last()
                        {
                            new_binding.bind_node(
                                target_var.clone(),
                                NodeInfo::new(
                                    target_id.clone(),
                                    target_workspace.clone(),
                                    target_type.clone(),
                                ),
                            );
                        }
                    }

                    // Bind relationship variable (we store first relationship info)
                    if let Some(rel_var) = &rel_pattern.variable {
                        if let Some(first_rel) = path.relationships.first() {
                            // For variable-length paths, we bind the first relationship
                            // and add path length info
                            let mut rel_info = first_rel.clone();
                            // Mark as multi-hop path
                            rel_info.relation_type =
                                format!("{}[{}]", rel_info.relation_type, path.length);
                            new_binding.bind_relation(rel_var.clone(), rel_info);
                        }
                    }

                    result_bindings.push(new_binding);
                }

                if total_paths >= MAX_PATHS {
                    tracing::warn!("PGQ: Path limit reached: {}", MAX_PATHS);
                    break;
                }
            } else {
                // Source variable not bound yet - enumerate all possible sources
                tracing::debug!("PGQ: Enumerating all possible source nodes...");

                for start_key in adj.keys() {
                    let (start_workspace, start_id) = start_key;

                    // Get node type for this source
                    // For LEFT direction (backward traversal), start_key is a TARGET in the original relationship
                    // For RIGHT direction (forward traversal), start_key is a SOURCE in the original relationship
                    let source_type = if matches!(rel_pattern.direction, Direction::Left) {
                        // Find where this node is a TARGET and get target_node_type
                        all_relationships
                            .iter()
                            .find(|(_, _, tw, tid, _)| tw == start_workspace && tid == start_id)
                            .map(|(_, _, _, _, r)| r.target_node_type.clone())
                            .unwrap_or_default()
                    } else {
                        // Find where this node is a SOURCE and get source_node_type
                        all_relationships
                            .iter()
                            .find(|(ws, id, _, _, _)| ws == start_workspace && id == start_id)
                            .map(|(_, _, _, _, r)| r.source_node_type.clone())
                            .unwrap_or_default()
                    };

                    // Apply source label filter
                    if !matches_label(&source_pattern.labels, &source_type) {
                        continue;
                    }

                    // Perform DFS from this source
                    let mut visited = HashSet::new();
                    let initial_path = PathInfo::new(
                        start_id.clone(),
                        start_workspace.clone(),
                        source_type.clone(),
                    );

                    let paths = dfs_find_paths(
                        start_key,
                        adj,
                        &mut visited,
                        initial_path,
                        min_depth,
                        max_depth,
                        0,
                        MAX_PATHS - total_paths,
                        &target_pattern.labels,
                    )?;

                    total_paths += paths.len();

                    // Convert paths to bindings
                    for path in paths {
                        let mut new_binding = binding.clone();

                        // Bind source node (first node in path)
                        if let Some((source_id, source_workspace, source_node_type)) =
                            path.nodes.first()
                        {
                            new_binding.bind_node(
                                source_var.clone(),
                                NodeInfo::new(
                                    source_id.clone(),
                                    source_workspace.clone(),
                                    source_node_type.clone(),
                                ),
                            );
                        }

                        // Bind target node (last node in path)
                        if let Some(target_var) = &target_pattern.variable {
                            if let Some((target_id, target_workspace, target_type)) =
                                path.nodes.last()
                            {
                                new_binding.bind_node(
                                    target_var.clone(),
                                    NodeInfo::new(
                                        target_id.clone(),
                                        target_workspace.clone(),
                                        target_type.clone(),
                                    ),
                                );
                            }
                        }

                        // Bind relationship variable
                        if let Some(rel_var) = &rel_pattern.variable {
                            if let Some(first_rel) = path.relationships.first() {
                                let mut rel_info = first_rel.clone();
                                rel_info.relation_type =
                                    format!("{}[{}]", rel_info.relation_type, path.length);
                                new_binding.bind_relation(rel_var.clone(), rel_info);
                            }
                        }

                        result_bindings.push(new_binding);
                    }

                    if total_paths >= MAX_PATHS {
                        tracing::warn!("PGQ: Path limit reached: {}", MAX_PATHS);
                        break;
                    }
                }
            }
        }
    }

    tracing::info!(
        "PGQ: Variable-length pattern found {} paths, created {} bindings",
        total_paths,
        result_bindings.len()
    );

    Ok(result_bindings)
}

/// DFS helper to find all paths within depth range
///
/// Returns paths that satisfy: min_depth <= path.length <= max_depth
///
/// # Arguments
/// * `current` - Current node (workspace, id)
/// * `adjacency` - Adjacency list for graph traversal
/// * `visited` - Set of visited nodes for cycle detection
/// * `current_path` - Current path being built
/// * `min_depth` - Minimum path length
/// * `max_depth` - Maximum path length
/// * `current_depth` - Current traversal depth
/// * `max_results` - Maximum number of results to return
/// * `target_labels` - Target node labels to filter (empty = any)
///
/// # Returns
/// Vector of valid paths found
fn dfs_find_paths(
    current: &(String, String),
    adjacency: &PgqGraphAdjacency,
    visited: &mut HashSet<(String, String)>,
    current_path: PathInfo,
    min_depth: u32,
    max_depth: u32,
    current_depth: u32,
    max_results: usize,
    target_labels: &[String],
) -> Result<Vec<PathInfo>> {
    let mut results = Vec::new();

    // Check if we've reached max depth or max results
    if current_depth >= max_depth || results.len() >= max_results {
        // If we're within the valid range and target label matches, add this path
        if current_depth >= min_depth {
            // Check target label filter
            if let Some((_, _, node_type)) = current_path.nodes.last() {
                if matches_label(target_labels, node_type) {
                    return Ok(vec![current_path]);
                }
            }
        }
        return Ok(results);
    }

    // Mark current node as visited
    visited.insert(current.clone());

    // If we're at or above min depth and target matches, this path is valid
    if current_depth >= min_depth {
        if let Some((_, _, node_type)) = current_path.nodes.last() {
            if matches_label(target_labels, node_type) {
                results.push(current_path.clone());
            }
        }
    }

    // Explore neighbors
    if let Some(neighbors) = adjacency.get(current) {
        for (next_workspace, next_id, next_node_type, rel_type, weight) in neighbors {
            let next_key = (next_workspace.clone(), next_id.clone());

            // Skip if already visited (cycle detection)
            if visited.contains(&next_key) {
                continue;
            }

            // Create extended path
            let rel_info = RelationInfo::new(
                rel_type.clone(),
                *weight,
                current.1.clone(), // source ID
                next_id.clone(),   // target ID
            );

            let extended_path = current_path.extend(
                rel_info,
                next_id.clone(),
                next_workspace.clone(),
                next_node_type.clone(),
            );

            // Recurse
            let sub_paths = dfs_find_paths(
                &next_key,
                adjacency,
                visited,
                extended_path,
                min_depth,
                max_depth,
                current_depth + 1,
                max_results - results.len(),
                target_labels,
            )?;

            results.extend(sub_paths);

            if results.len() >= max_results {
                break;
            }
        }
    }

    // Unmark current node (backtrack)
    visited.remove(current);

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_info_new() {
        let path = PathInfo::new("node1".into(), "ws".into(), "User".into());
        assert_eq!(path.nodes.len(), 1);
        assert_eq!(path.length, 0);
    }

    #[test]
    fn test_path_info_extend() {
        let path = PathInfo::new("node1".into(), "ws".into(), "User".into());
        let rel = RelationInfo::new("FOLLOWS".into(), Some(0.9), "node1".into(), "node2".into());
        let extended = path.extend(rel, "node2".into(), "ws".into(), "User".into());

        assert_eq!(extended.nodes.len(), 2);
        assert_eq!(extended.relationships.len(), 1);
        assert_eq!(extended.length, 1);
    }
}
