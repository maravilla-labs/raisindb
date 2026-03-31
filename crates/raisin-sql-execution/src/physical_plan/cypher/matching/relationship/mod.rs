//! Relationship pattern matching
//!
//! Handles matching of relationship patterns between nodes, including global
//! index scans, directional traversal, and bidirectional pattern expansion.
//!
//! # Module Structure
//!
//! - `direction` - Direction-based fetching and type filtering

mod direction;

use std::collections::HashMap;
use std::sync::Arc;

use futures::{StreamExt, TryStreamExt};
use raisin_cypher_parser::{Direction, Expr, NodePattern, RelPattern};
use raisin_storage::{NodeRepository, RelationRepository, Storage, StorageScope};

use crate::physical_plan::cypher::evaluation::{execute_where, FunctionContext};
use crate::physical_plan::cypher::types::{CypherContext, NodeInfo, RelationInfo, VariableBinding};
use crate::physical_plan::executor::ExecutionError;

pub use direction::{filter_relations_by_type, get_relations_by_direction};

type Result<T> = std::result::Result<T, ExecutionError>;

/// Match a simple relationship pattern using global index: (a)-[:TYPE]->(b)
///
/// Scans all relationships of the specified type across all workspaces, then
/// fetches full node data for WHERE clause evaluation.
pub async fn match_simple_relationship_pattern<S: Storage>(
    source_pattern: &NodePattern,
    rel_pattern: &RelPattern,
    target_pattern: &NodePattern,
    bindings: Vec<VariableBinding>,
    where_clause: Option<&Expr>,
    storage: &Arc<S>,
    context: &CypherContext,
) -> Result<Vec<VariableBinding>> {
    tracing::info!("Scanning relationships in global index...");

    let relation_types: Vec<&str> = rel_pattern.types.iter().map(|s| s.as_str()).collect();
    let relation_type_filter = relation_types.first().copied();
    tracing::debug!("   Relation type filter: {:?}", relation_types);

    let relationships = storage
        .relations()
        .scan_relations_global(
            raisin_storage::BranchScope::new(&context.tenant_id, &context.repo_id, &context.branch),
            relation_type_filter,
            context.revision.as_ref(),
        )
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

    tracing::info!(
        "   Found {} relationships in global index",
        relationships.len()
    );

    if relationships.is_empty() {
        tracing::warn!("   No relationships found - returning empty result");
        return Ok(vec![]);
    }

    // Filter by multiple relation types if specified
    let relationships: Vec<_> = if relation_types.len() > 1 {
        relationships
            .into_iter()
            .filter(|(_, _, _, _, rel)| relation_types.contains(&rel.relation_type.as_str()))
            .collect()
    } else {
        relationships
    };

    // Collect and fetch unique nodes
    let mut unique_nodes: HashMap<(String, String), Option<raisin_models::nodes::Node>> =
        HashMap::new();
    for (source_workspace, source_id, target_workspace, target_id, _) in &relationships {
        unique_nodes.insert((source_workspace.clone(), source_id.clone()), None);
        unique_nodes.insert((target_workspace.clone(), target_id.clone()), None);
    }

    tracing::debug!("   Fetching {} unique nodes...", unique_nodes.len());

    for (workspace, node_id) in unique_nodes.keys().cloned().collect::<Vec<_>>() {
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
        unique_nodes.insert((workspace, node_id), node_opt);
    }

    tracing::debug!("   Nodes fetched");

    let is_bidirectional = matches!(rel_pattern.direction, Direction::Both | Direction::None);
    let mut result = Vec::with_capacity(relationships.len());

    for (source_workspace, source_id, target_workspace, target_id, relation_ref) in
        relationships.iter()
    {
        let source_node = unique_nodes
            .get(&(source_workspace.clone(), source_id.clone()))
            .and_then(|n| n.as_ref());
        let target_node = unique_nodes
            .get(&(target_workspace.clone(), target_id.clone()))
            .and_then(|n| n.as_ref());

        let (Some(source_node), Some(target_node)) = (source_node, target_node) else {
            continue;
        };

        for binding in &bindings {
            let new_binding = create_forward_binding(
                binding,
                source_pattern,
                target_pattern,
                rel_pattern,
                source_node,
                target_node,
                source_workspace,
                target_workspace,
                source_id,
                target_id,
                relation_ref,
            );
            result.push(new_binding);

            if is_bidirectional {
                let reverse_binding = create_reverse_binding(
                    binding,
                    source_pattern,
                    target_pattern,
                    rel_pattern,
                    source_node,
                    target_node,
                    source_workspace,
                    target_workspace,
                    source_id,
                    target_id,
                    relation_ref,
                );
                result.push(reverse_binding);
            }
        }
    }

    tracing::info!("   Created {} bindings", result.len());

    // Apply WHERE clause filter if present
    if let Some(where_expr) = where_clause {
        tracing::info!("   Applying WHERE clause filter: {:?}", where_expr);

        let func_context = FunctionContext {
            storage: storage.as_ref(),
            tenant_id: &context.tenant_id,
            repo_id: &context.repo_id,
            branch: &context.branch,
            workspace_id: &context.workspace_id,
            revision: context.revision.as_ref(),
            parameters: &context.parameters,
        };
        let filtered = execute_where(where_expr, result, &func_context).await?;
        tracing::info!("   After WHERE filter: {} bindings", filtered.len());
        Ok(filtered)
    } else {
        Ok(result)
    }
}

/// Match a relationship pattern connecting two nodes (vectorized expansion)
pub async fn match_relationship_pattern<S: Storage>(
    rel_pattern: &RelPattern,
    source_node_var: &str,
    target_node_pattern: &NodePattern,
    bindings: Vec<VariableBinding>,
    storage: &Arc<S>,
    context: &CypherContext,
) -> Result<Vec<VariableBinding>> {
    let concurrency = 50;

    let results: Vec<Vec<VariableBinding>> = futures::stream::iter(bindings)
        .map(|binding| {
            let storage = storage.clone();
            let context = context.clone();
            let rel_pattern = rel_pattern.clone();
            let source_node_var = source_node_var.to_string();
            let target_node_pattern = target_node_pattern.clone();

            async move {
                let source_node = binding.get_node(&source_node_var).ok_or_else(|| {
                    ExecutionError::Validation(format!(
                        "Source node variable '{}' not found in binding",
                        source_node_var
                    ))
                })?;

                let relations = get_relations_by_direction(
                    &source_node.id,
                    &source_node.workspace,
                    &rel_pattern,
                    &storage,
                    &context,
                )
                .await?;

                let filtered_relations = filter_relations_by_type(relations, &rel_pattern.types);

                let mut new_bindings = Vec::new();

                for relation in filtered_relations {
                    let target_node_result = storage
                        .nodes()
                        .get(
                            StorageScope::new(
                                &context.tenant_id,
                                &context.repo_id,
                                &context.branch,
                                &relation.workspace,
                            ),
                            &relation.target,
                            context.revision.as_ref(),
                        )
                        .await;

                    let target_node = match target_node_result {
                        Ok(Some(node)) => node,
                        Ok(None) => continue,
                        Err(e) => return Err(ExecutionError::Backend(e.to_string())),
                    };

                    if !super::node::node_matches_pattern(&target_node, &target_node_pattern) {
                        continue;
                    }

                    let mut new_binding = binding.clone();

                    if let Some(target_var) = &target_node_pattern.variable {
                        let target_info = NodeInfo {
                            id: target_node.id.clone(),
                            path: target_node.path.clone(),
                            node_type: target_node.node_type.clone(),
                            properties: target_node.properties.clone(),
                            workspace: relation.workspace.clone(),
                        };
                        new_binding.bind_node(target_var.clone(), target_info);
                    }

                    if let Some(rel_var) = &rel_pattern.variable {
                        let mut properties = std::collections::HashMap::new();
                        if let Some(weight) = relation.weight {
                            properties.insert(
                                "weight".to_string(),
                                raisin_models::nodes::properties::PropertyValue::Float(
                                    weight as f64,
                                ),
                            );
                        }

                        let rel_info = RelationInfo {
                            source_var: source_node_var.clone(),
                            target_var: target_node_pattern
                                .variable
                                .clone()
                                .unwrap_or_else(|| format!("_anon_{}", target_node.id)),
                            relation_type: relation.relation_type.clone(),
                            properties,
                        };
                        new_binding.bind_relation(rel_var.clone(), rel_info);
                    }

                    new_bindings.push(new_binding);
                }
                Ok(new_bindings)
            }
        })
        .buffer_unordered(concurrency)
        .try_collect()
        .await?;

    Ok(results.into_iter().flatten().collect())
}

/// Create a forward binding (source -> target as stored)
fn create_forward_binding(
    binding: &VariableBinding,
    source_pattern: &NodePattern,
    target_pattern: &NodePattern,
    rel_pattern: &RelPattern,
    source_node: &raisin_models::nodes::Node,
    target_node: &raisin_models::nodes::Node,
    source_workspace: &str,
    target_workspace: &str,
    source_id: &str,
    target_id: &str,
    relation_ref: &raisin_models::nodes::FullRelation,
) -> VariableBinding {
    let mut new_binding = binding.clone();

    if let Some(source_var) = &source_pattern.variable {
        let source_info = NodeInfo {
            id: source_node.id.clone(),
            workspace: source_workspace.to_string(),
            path: source_node.path.clone(),
            node_type: source_node.node_type.clone(),
            properties: source_node.properties.clone(),
        };
        new_binding.bind_node(source_var.clone(), source_info);
    }

    if let Some(target_var) = &target_pattern.variable {
        let target_info = NodeInfo {
            id: target_node.id.clone(),
            workspace: target_workspace.to_string(),
            path: target_node.path.clone(),
            node_type: target_node.node_type.clone(),
            properties: target_node.properties.clone(),
        };
        new_binding.bind_node(target_var.clone(), target_info);
    }

    if let Some(rel_var) = &rel_pattern.variable {
        let mut properties = HashMap::new();
        if let Some(weight) = relation_ref.weight {
            properties.insert(
                "weight".to_string(),
                raisin_models::nodes::properties::PropertyValue::Float(weight as f64),
            );
        }

        let rel_info = RelationInfo {
            source_var: source_pattern
                .variable
                .clone()
                .unwrap_or_else(|| format!("_anon_src_{}", source_id)),
            target_var: target_pattern
                .variable
                .clone()
                .unwrap_or_else(|| format!("_anon_tgt_{}", target_id)),
            relation_type: relation_ref.relation_type.clone(),
            properties,
        };
        new_binding.bind_relation(rel_var.clone(), rel_info);
    }

    new_binding
}

/// Create a reverse binding (target -> source) for bidirectional patterns
fn create_reverse_binding(
    binding: &VariableBinding,
    source_pattern: &NodePattern,
    target_pattern: &NodePattern,
    rel_pattern: &RelPattern,
    source_node: &raisin_models::nodes::Node,
    target_node: &raisin_models::nodes::Node,
    source_workspace: &str,
    target_workspace: &str,
    source_id: &str,
    target_id: &str,
    relation_ref: &raisin_models::nodes::FullRelation,
) -> VariableBinding {
    let mut reverse_binding = binding.clone();

    if let Some(source_var) = &source_pattern.variable {
        let target_as_source = NodeInfo {
            id: target_node.id.clone(),
            workspace: target_workspace.to_string(),
            path: target_node.path.clone(),
            node_type: target_node.node_type.clone(),
            properties: target_node.properties.clone(),
        };
        reverse_binding.bind_node(source_var.clone(), target_as_source);
    }

    if let Some(target_var) = &target_pattern.variable {
        let source_as_target = NodeInfo {
            id: source_node.id.clone(),
            workspace: source_workspace.to_string(),
            path: source_node.path.clone(),
            node_type: source_node.node_type.clone(),
            properties: source_node.properties.clone(),
        };
        reverse_binding.bind_node(target_var.clone(), source_as_target);
    }

    if let Some(rel_var) = &rel_pattern.variable {
        let mut properties = HashMap::new();
        if let Some(weight) = relation_ref.weight {
            properties.insert(
                "weight".to_string(),
                raisin_models::nodes::properties::PropertyValue::Float(weight as f64),
            );
        }

        let rel_info = RelationInfo {
            source_var: source_pattern
                .variable
                .clone()
                .unwrap_or_else(|| format!("_anon_src_{}", target_id)),
            target_var: target_pattern
                .variable
                .clone()
                .unwrap_or_else(|| format!("_anon_tgt_{}", source_id)),
            relation_type: relation_ref.relation_type.clone(),
            properties,
        };
        reverse_binding.bind_relation(rel_var.clone(), rel_info);
    }

    reverse_binding
}
