//! Node pattern matching
//!
//! Handles matching of single node patterns in Cypher queries.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_cypher_parser::{Expr, Literal, NodePattern};
use raisin_storage::{NodeRepository, RelationRepository, Storage, StorageScope};

use crate::physical_plan::cypher::types::{CypherContext, NodeInfo, VariableBinding};
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Match a node pattern
///
/// **GRAPH-ONLY SEMANTICS**: This function returns only nodes that participate in
/// relationships (either as source or target). This is different from SQL which
/// queries all workspace nodes. For complete workspace queries, use SQL.
///
/// # Arguments
/// * `pattern` - The node pattern to match (e.g., `(n:Label)`)
/// * `bindings` - Existing variable bindings to extend
/// * `storage` - Storage backend
/// * `context` - Cypher execution context
///
/// # Returns
/// New bindings with matched nodes added
pub async fn match_node_pattern<S: Storage>(
    pattern: &NodePattern,
    bindings: Vec<VariableBinding>,
    storage: &Arc<S>,
    context: &CypherContext,
) -> Result<Vec<VariableBinding>> {
    // Pre-allocate result vector with estimated capacity
    let mut result = Vec::with_capacity(bindings.len() * 4);

    // OPTIMIZATION: Path-First Traversal
    // Check if pattern has "path" property constraint (e.g. {path: '/foo'})
    // If so, use path_index to find the node directly (O(1)) instead of scanning
    if let Some(props) = &pattern.properties {
        for (key, expr) in props {
            if key == "path" {
                if let Expr::Literal(Literal::String(path)) = expr {
                    tracing::debug!(
                        "🚀 Path-First Optimization: Looking up node by path '{}'",
                        path
                    );

                    // Use path_index to find node ID
                    let node_id_opt = storage
                        .nodes()
                        .get_node_id_by_path(
                            StorageScope::new(
                                &context.tenant_id,
                                &context.repo_id,
                                &context.branch,
                                &context.workspace_id,
                            ),
                            path,
                            context.revision.as_ref(),
                        )
                        .await
                        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

                    if let Some(node_id) = node_id_opt {
                        // Fetch the node
                        let node_opt = storage
                            .nodes()
                            .get(
                                StorageScope::new(
                                    &context.tenant_id,
                                    &context.repo_id,
                                    &context.branch,
                                    &context.workspace_id,
                                ),
                                &node_id,
                                context.revision.as_ref(),
                            )
                            .await
                            .map_err(|e| ExecutionError::Backend(e.to_string()))?;

                        if let Some(node) = node_opt {
                            // Check if it matches other constraints (label, other props)
                            if node_matches_pattern(&node, pattern) {
                                // Create binding
                                for binding in bindings {
                                    if let Some(var) = &pattern.variable {
                                        let node_info = NodeInfo {
                                            id: node.id.clone(),
                                            path: node.path.clone(),
                                            node_type: node.node_type.clone(),
                                            properties: node.properties.clone(),
                                            workspace: context.workspace_id.clone(),
                                        };
                                        let mut new_binding = binding.clone();
                                        new_binding.bind_node(var.clone(), node_info);
                                        result.push(new_binding);
                                    } else {
                                        result.push(binding.clone());
                                    }
                                }
                                return Ok(result);
                            }
                        }
                    }
                    // If path not found or node doesn't match, return empty
                    return Ok(vec![]);
                }
            }
        }
    }

    // Get label filter (e.g., :RaisinPage filters by node_type label)
    let node_type_filter = pattern.labels.first().map(|s| s.as_str());

    // GRAPH-ONLY SEMANTICS: Scan the global relationship index to find all
    // nodes that participate in the graph (have at least one relationship)
    if tracing::enabled!(tracing::Level::DEBUG) {
        tracing::debug!(
            "🔵 Cypher: Scanning global relationship graph (tenant={}, repo={}, branch={})",
            context.tenant_id,
            context.repo_id,
            context.branch
        );
    }

    let relationships = storage
        .relations()
        .scan_relations_global(
            raisin_storage::BranchScope::new(&context.tenant_id, &context.repo_id, &context.branch),
            None, // No relationship type filter
            context.revision.as_ref(),
        )
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

    tracing::debug!(
        "   Found {} relationships in global index",
        relationships.len()
    );

    // Collect unique nodes from relationships with their types
    // HashMap: (workspace, node_id) -> Cypher label (e.g., "RaisinPage")
    let mut unique_nodes: HashMap<(String, String), String> = HashMap::new();

    for (_src_ws, _src_id, _tgt_ws, _tgt_id, full_rel) in &relationships {
        // Add source node if matches filter (or no filter)
        if node_type_filter.is_none() || full_rel.source_node_type == node_type_filter.unwrap() {
            unique_nodes.insert(
                (
                    full_rel.source_workspace.clone(),
                    full_rel.source_id.clone(),
                ),
                full_rel.source_node_type.clone(),
            );
        }

        // Add target node if matches filter (or no filter)
        if node_type_filter.is_none() || full_rel.target_node_type == node_type_filter.unwrap() {
            unique_nodes.insert(
                (
                    full_rel.target_workspace.clone(),
                    full_rel.target_id.clone(),
                ),
                full_rel.target_node_type.clone(),
            );
        }
    }

    tracing::debug!("   Found {} unique nodes in graph", unique_nodes.len());
    if let Some(filter) = node_type_filter {
        tracing::debug!("   Filtered by node type label: {}", filter);
    }

    // Fetch actual nodes for their properties and paths
    let mut filtered_nodes: Vec<(String, raisin_models::nodes::Node)> = Vec::new();
    for ((workspace, node_id), _node_type_label) in unique_nodes {
        let node_opt = storage
            .nodes()
            .get(
                StorageScope::new(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    &workspace,
                ),
                &node_id,
                context.revision.as_ref(),
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?;

        if let Some(node) = node_opt {
            filtered_nodes.push((workspace, node));
        }
        // If node doesn't exist, skip (dangling relationship)
    }

    tracing::debug!("   After node fetch: {} nodes", filtered_nodes.len());

    // For each binding, create new bindings with matched nodes
    for binding in bindings {
        for (workspace, node) in &filtered_nodes {
            if let Some(var) = &pattern.variable {
                // Create node info with actual workspace
                let node_info = NodeInfo {
                    id: node.id.clone(),
                    path: node.path.clone(),
                    node_type: node.node_type.clone(),
                    properties: node.properties.clone(),
                    workspace: workspace.clone(), // Use actual workspace from relationships
                };

                // Create new binding with this node
                let mut new_binding = binding.clone();
                new_binding.bind_node(var.clone(), node_info);
                result.push(new_binding);
            } else {
                // No variable, just pass through
                result.push(binding.clone());
            }
        }
    }

    Ok(result)
}

/// Check if a node matches a pattern's constraints
///
/// Validates node against pattern requirements like labels and properties.
///
/// # Arguments
/// * `node` - The node to validate
/// * `pattern` - The pattern to match against
///
/// # Returns
/// `true` if the node matches the pattern, `false` otherwise
#[inline]
pub fn node_matches_pattern(node: &raisin_models::nodes::Node, pattern: &NodePattern) -> bool {
    // Check label (node type)
    if let Some(expected_type) = pattern.labels.first() {
        if node.node_type != *expected_type {
            return false;
        }
    }

    // TODO: Check property constraints in pattern.properties
    // This would require evaluating expressions against node.properties

    // TODO: Check WHERE clause in pattern.where_clause

    true
}
