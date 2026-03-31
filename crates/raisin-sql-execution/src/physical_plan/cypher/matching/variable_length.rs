//! Variable-length path matching
//!
//! Handles matching of variable-length relationship patterns using DFS traversal.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use raisin_cypher_parser::{NodePattern, RelPattern};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{RelationRepository, Storage};

use crate::physical_plan::cypher::algorithms::GraphAdjacency;
use crate::physical_plan::cypher::types::{
    CypherContext, NodeInfo, PathInfo, RelationInfo, VariableBinding,
};
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Execute variable-length relationship pattern: (a)-[:TYPE*min..max]->(b)
///
/// Uses DFS (Depth-First Search) to find all paths within the specified length range.
/// Implements cycle detection to prevent infinite loops.
///
/// # Arguments
/// * `source_pattern` - Source node pattern
/// * `rel_pattern` - Relationship pattern with range specification
/// * `target_pattern` - Target node pattern
/// * `bindings` - Existing variable bindings
/// * `storage` - Storage backend
/// * `context` - Cypher execution context
///
/// # Returns
/// New bindings with matched variable-length paths
pub async fn execute_variable_length_pattern<S: Storage>(
    source_pattern: &NodePattern,
    rel_pattern: &RelPattern,
    target_pattern: &NodePattern,
    bindings: Vec<VariableBinding>,
    storage: &Arc<S>,
    context: &CypherContext,
) -> Result<Vec<VariableBinding>> {
    let range = rel_pattern.range.as_ref().ok_or_else(|| {
        ExecutionError::Validation(
            "Variable-length pattern requires a range specification".to_string(),
        )
    })?;

    // Default bounds for unbounded queries
    const DEFAULT_MAX_DEPTH: u32 = 10;
    const MAX_PATHS: usize = 10000;

    let min_depth = range.min.unwrap_or(1); // Default minimum is 1
    let max_depth = range.max.unwrap_or(DEFAULT_MAX_DEPTH); // Cap unbounded queries

    tracing::info!(
        "   Executing variable-length pattern: min={}, max={}",
        min_depth,
        max_depth
    );

    // Warn on expensive queries
    if max_depth > 5 {
        tracing::warn!("   ⚠️  Variable-length pattern with depth > 5 may be expensive");
    }

    // Extract relation type filter
    let relation_type_filter = rel_pattern.types.first().map(|s| s.as_str());

    // Get all relationships once (cached for the duration of this query)
    tracing::debug!(
        "   Fetching all relationships of type {:?}...",
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

    tracing::info!("   ✓ Fetched {} relationships", all_relationships.len());

    // Build adjacency list for efficient traversal
    let mut adjacency: GraphAdjacency = HashMap::new();

    for (src_workspace, src_id, tgt_workspace, tgt_id, rel_ref) in &all_relationships {
        let key = (src_workspace.clone(), src_id.clone());
        let value = (
            tgt_workspace.clone(),
            tgt_id.clone(),
            rel_ref.relation_type.clone(),
        );
        adjacency.entry(key).or_default().push(value);
    }

    tracing::debug!(
        "   Built adjacency list with {} source nodes",
        adjacency.len()
    );

    let mut result_bindings = Vec::new();
    let mut total_paths = 0;

    // For each existing binding, find paths
    for binding in bindings {
        // If source pattern has no variable, we need to scan all nodes as starting points
        // For now, we'll require a source variable
        if source_pattern.variable.is_none() {
            return Err(ExecutionError::Validation(
                "Variable-length patterns require a source node variable".to_string(),
            ));
        }

        // Check if source node is already bound
        if let Some(source_var) = &source_pattern.variable {
            if let Some(source_node) = binding.get_node(source_var) {
                // Source is already bound - find paths from this node
                let start_key = (source_node.workspace.clone(), source_node.id.clone());

                tracing::debug!(
                    "   Finding paths from bound source: {}:{}",
                    source_node.workspace,
                    source_node.id
                );

                // Perform DFS from this source
                let mut visited = HashSet::new();
                let initial_path =
                    PathInfo::new(source_node.id.clone(), source_node.workspace.clone());

                let paths = dfs_find_paths(
                    &start_key,
                    &adjacency,
                    &mut visited,
                    initial_path,
                    min_depth,
                    max_depth,
                    0,
                    MAX_PATHS - total_paths,
                )?;

                tracing::debug!("   Found {} paths from this source", paths.len());
                total_paths += paths.len();

                // Convert paths to bindings
                for path in paths {
                    let mut new_binding = binding.clone();

                    // Bind target node (last node in path)
                    if let Some(target_var) = &target_pattern.variable {
                        if let Some((target_id, target_workspace)) = path.nodes.last() {
                            let target_info = NodeInfo {
                                id: target_id.clone(),
                                workspace: target_workspace.clone(),
                                path: String::new(),
                                node_type: String::new(),
                                properties: HashMap::new(),
                            };
                            new_binding.bind_node(target_var.clone(), target_info);
                        }
                    }

                    // Bind relationship variable as array of relationships
                    if let Some(rel_var) = &rel_pattern.variable {
                        // For now, bind as a JSON array representation
                        // TODO: Support proper array binding
                        let rel_array: Vec<PropertyValue> = path
                            .relationships
                            .iter()
                            .map(|rel| {
                                let mut rel_map = HashMap::new();
                                rel_map.insert(
                                    "type".to_string(),
                                    PropertyValue::String(rel.relation_type.clone()),
                                );
                                PropertyValue::Object(rel_map)
                            })
                            .collect();

                        // Store path length as a property we can access
                        let mut path_info = HashMap::new();
                        path_info.insert(
                            "length".to_string(),
                            PropertyValue::Integer(path.length as i64),
                        );
                        path_info
                            .insert("relationships".to_string(), PropertyValue::Array(rel_array));

                        // Create a special RelationInfo for the path
                        let path_rel_info = RelationInfo {
                            source_var: source_var.clone(),
                            target_var: target_pattern
                                .variable
                                .clone()
                                .unwrap_or_else(|| "_target".to_string()),
                            relation_type: format!("PATH[{}]", path.length),
                            properties: path_info,
                        };

                        new_binding.bind_relation(rel_var.clone(), path_rel_info);
                    }

                    result_bindings.push(new_binding);
                }

                if total_paths >= MAX_PATHS {
                    tracing::warn!("   ⚠️  Path limit reached: {}", MAX_PATHS);
                    break;
                }
            } else {
                // Source variable not bound yet - need to enumerate all possible sources
                // This is the case for: MATCH (a)-[:TYPE*1..3]->(b) with no prior bindings
                tracing::debug!("   Enumerating all possible source nodes...");

                for start_key in adjacency.keys() {
                    let (start_workspace, start_id) = start_key;

                    // Perform DFS from this source
                    let mut visited = HashSet::new();
                    let initial_path = PathInfo::new(start_id.clone(), start_workspace.clone());

                    let paths = dfs_find_paths(
                        start_key,
                        &adjacency,
                        &mut visited,
                        initial_path,
                        min_depth,
                        max_depth,
                        0,
                        MAX_PATHS - total_paths,
                    )?;

                    total_paths += paths.len();

                    // Convert paths to bindings
                    for path in paths {
                        let mut new_binding = binding.clone();

                        // Bind source node (first node in path)
                        if let Some((source_id, source_workspace)) = path.nodes.first() {
                            let source_info = NodeInfo {
                                id: source_id.clone(),
                                workspace: source_workspace.clone(),
                                path: String::new(),
                                node_type: String::new(),
                                properties: HashMap::new(),
                            };
                            new_binding.bind_node(source_var.clone(), source_info);
                        }

                        // Bind target node (last node in path)
                        if let Some(target_var) = &target_pattern.variable {
                            if let Some((target_id, target_workspace)) = path.nodes.last() {
                                let target_info = NodeInfo {
                                    id: target_id.clone(),
                                    workspace: target_workspace.clone(),
                                    path: String::new(),
                                    node_type: String::new(),
                                    properties: HashMap::new(),
                                };
                                new_binding.bind_node(target_var.clone(), target_info);
                            }
                        }

                        // Bind relationship variable as path info
                        if let Some(rel_var) = &rel_pattern.variable {
                            let rel_array: Vec<PropertyValue> = path
                                .relationships
                                .iter()
                                .map(|rel| {
                                    let mut rel_map = HashMap::new();
                                    rel_map.insert(
                                        "type".to_string(),
                                        PropertyValue::String(rel.relation_type.clone()),
                                    );
                                    PropertyValue::Object(rel_map)
                                })
                                .collect();

                            let mut path_info = HashMap::new();
                            path_info.insert(
                                "length".to_string(),
                                PropertyValue::Integer(path.length as i64),
                            );
                            path_info.insert(
                                "relationships".to_string(),
                                PropertyValue::Array(rel_array),
                            );

                            let path_rel_info = RelationInfo {
                                source_var: source_var.clone(),
                                target_var: target_pattern
                                    .variable
                                    .clone()
                                    .unwrap_or_else(|| "_target".to_string()),
                                relation_type: format!("PATH[{}]", path.length),
                                properties: path_info,
                            };

                            new_binding.bind_relation(rel_var.clone(), path_rel_info);
                        }

                        result_bindings.push(new_binding);
                    }

                    if total_paths >= MAX_PATHS {
                        tracing::warn!("   ⚠️  Path limit reached: {}", MAX_PATHS);
                        break;
                    }
                }
            }
        }
    }

    tracing::info!(
        "   ✓ Variable-length pattern found {} paths, created {} bindings",
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
///
/// # Returns
/// Vector of valid paths found
pub fn dfs_find_paths(
    current: &(String, String),
    adjacency: &GraphAdjacency,
    visited: &mut HashSet<(String, String)>,
    current_path: PathInfo,
    min_depth: u32,
    max_depth: u32,
    current_depth: u32,
    max_results: usize,
) -> Result<Vec<PathInfo>> {
    let mut results = Vec::new();

    // Check if we've reached max depth or max results
    if current_depth >= max_depth || results.len() >= max_results {
        // If we're within the valid range, add this path
        if current_depth >= min_depth {
            return Ok(vec![current_path]);
        }
        return Ok(results);
    }

    // Mark current node as visited
    visited.insert(current.clone());

    // If we're at or above min depth, this path is valid
    if current_depth >= min_depth {
        results.push(current_path.clone());
    }

    // Explore neighbors
    if let Some(neighbors) = adjacency.get(current) {
        for (next_workspace, next_id, rel_type) in neighbors {
            let next_key = (next_workspace.clone(), next_id.clone());

            // Skip if already visited (cycle detection)
            if visited.contains(&next_key) {
                continue;
            }

            // Create extended path
            let rel_info = RelationInfo {
                source_var: current.1.clone(), // source ID
                target_var: next_id.clone(),
                relation_type: rel_type.clone(),
                properties: HashMap::new(),
            };

            let extended_path =
                current_path.extend(rel_info, next_id.clone(), next_workspace.clone());

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
