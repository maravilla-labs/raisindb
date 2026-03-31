//! Single-Node Pattern Matching
//!
//! Matches patterns of the form: (n) or (n:Label)
//!
//! Extracts unique nodes from the relationship index by scanning all relations
//! and collecting both source and target nodes.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_sql::ast::NodePattern;
use raisin_storage::{RelationRepository, Storage};

use super::matches_label;
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::pgq::context::PgqContext;
use crate::physical_plan::pgq::types::{NodeInfo, VariableBinding};

type Result<T> = std::result::Result<T, ExecutionError>;

/// Match a single-node pattern: (n) or (n:Label)
///
/// This function scans the global relation index and collects unique nodes
/// from both source and target positions of all relationships.
///
/// # Arguments
///
/// * `node_pattern` - Pattern for the node (with optional variable and labels)
/// * `storage` - Storage backend
/// * `context` - PGQ execution context
///
/// # Returns
///
/// Vector of variable bindings, one for each unique matched node.
pub async fn match_single_node<S: Storage>(
    node_pattern: &NodePattern,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<Vec<VariableBinding>> {
    tracing::debug!(
        "PGQ: Matching single-node pattern ({:?}:{:?})",
        node_pattern.variable,
        node_pattern.labels
    );

    // Scan all relations (no type filter)
    // Returns: Vec<(src_workspace, src_id, tgt_workspace, tgt_id, FullRelation)>
    let relations = storage
        .relations()
        .scan_relations_global(
            raisin_storage::BranchScope::new(&context.tenant_id, &context.repo_id, &context.branch),
            None, // No relation type filter - get all
            context.revision.as_ref(),
        )
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

    tracing::debug!(
        "PGQ: Found {} relations, extracting unique nodes",
        relations.len()
    );

    // Collect unique nodes from both source and target positions
    // Key: "workspace:id" to ensure uniqueness
    let mut unique_nodes: HashMap<String, NodeInfo> = HashMap::new();

    for (src_ws, src_id, tgt_ws, tgt_id, full_rel) in relations {
        // Check source node
        if matches_label(&node_pattern.labels, &full_rel.source_node_type) {
            let key = format!("{}:{}", src_ws, src_id);
            unique_nodes.entry(key).or_insert_with(|| {
                NodeInfo::new(src_id, src_ws, full_rel.source_node_type.clone())
            });
        }

        // Check target node
        if matches_label(&node_pattern.labels, &full_rel.target_node_type) {
            let key = format!("{}:{}", tgt_ws, tgt_id);
            unique_nodes.entry(key).or_insert_with(|| {
                NodeInfo::new(tgt_id, tgt_ws, full_rel.target_node_type.clone())
            });
        }
    }

    tracing::debug!("PGQ: Found {} unique nodes", unique_nodes.len());

    // Convert to bindings
    let mut bindings = Vec::with_capacity(unique_nodes.len());

    for (_, node_info) in unique_nodes {
        let mut binding = VariableBinding::new();

        // Bind node if variable specified
        if let Some(var) = &node_pattern.variable {
            binding.bind_node(var.clone(), node_info);
        }

        bindings.push(binding);
    }

    tracing::debug!("PGQ: Created {} bindings", bindings.len());

    Ok(bindings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_info_creation() {
        let node = NodeInfo::new("123".into(), "default".into(), "Article".into());
        assert_eq!(node.id, "123");
        assert_eq!(node.workspace, "default");
        assert_eq!(node.node_type, "Article");
    }
}
