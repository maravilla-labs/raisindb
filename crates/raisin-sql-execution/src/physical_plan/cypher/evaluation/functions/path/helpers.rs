//! Shared helpers for path-finding functions
//!
//! Node ID/workspace extraction, adjacency graph construction, and path
//! serialization to PropertyValue.

use std::collections::HashMap;

use raisin_cypher_parser::Expr;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{RelationRepository, Storage};

use super::super::super::expr::evaluate_expr_async_impl;
use super::super::traits::FunctionContext;
use crate::physical_plan::cypher::algorithms::GraphAdjacency;
use crate::physical_plan::cypher::types::{PathInfo, VariableBinding};

/// Extract node ID and workspace from an expression
pub(super) async fn extract_node_id_workspace<S: Storage>(
    expr: &Expr,
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<(String, String), Error> {
    let node_value = evaluate_expr_async_impl(expr, binding, context).await?;

    match node_value {
        PropertyValue::Object(ref map) => {
            let id = map
                .get("id")
                .and_then(|v| match v {
                    PropertyValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| Error::Validation("Node must have an 'id' field".to_string()))?;

            let workspace = map
                .get("workspace")
                .and_then(|v| match v {
                    PropertyValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    Error::Validation("Node must have a 'workspace' field".to_string())
                })?;

            Ok((id, workspace))
        }
        _ => Err(Error::Validation(
            "Path functions require node objects as arguments".to_string(),
        )),
    }
}

/// Build adjacency graph from current query context
pub(super) async fn build_adjacency_graph<S: Storage>(
    context: &FunctionContext<'_, S>,
) -> Result<GraphAdjacency, Error> {
    tracing::debug!("   Building adjacency graph for path finding...");

    let all_relationships = context
        .storage
        .relations()
        .scan_relations_global(
            raisin_storage::BranchScope::new(context.tenant_id, context.repo_id, context.branch),
            None,
            context.revision,
        )
        .await
        .map_err(|e| Error::Backend(e.to_string()))?;

    tracing::debug!("   Scanned {} relationships", all_relationships.len());

    let mut adjacency: GraphAdjacency = HashMap::new();

    for (src_workspace, src_id, tgt_workspace, tgt_id, rel_ref) in all_relationships {
        let key = (src_workspace, src_id);
        let value = (tgt_workspace, tgt_id, rel_ref.relation_type);
        adjacency.entry(key).or_default().push(value);
    }

    tracing::debug!("   Built adjacency graph with {} nodes", adjacency.len());

    Ok(adjacency)
}

/// Convert PathInfo to PropertyValue
pub(super) fn path_info_to_property_value(path_info: &PathInfo) -> PropertyValue {
    let mut path_map = HashMap::new();

    let nodes_array: Vec<PropertyValue> = path_info
        .nodes
        .iter()
        .map(|(id, workspace)| {
            let mut node_map = HashMap::new();
            node_map.insert("id".to_string(), PropertyValue::String(id.clone()));
            node_map.insert(
                "workspace".to_string(),
                PropertyValue::String(workspace.clone()),
            );
            PropertyValue::Object(node_map)
        })
        .collect();

    path_map.insert("nodes".to_string(), PropertyValue::Array(nodes_array));

    let rels_array: Vec<PropertyValue> = path_info
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

    path_map.insert(
        "relationships".to_string(),
        PropertyValue::Array(rels_array),
    );

    path_map.insert(
        "length".to_string(),
        PropertyValue::Integer(path_info.length as i64),
    );

    PropertyValue::Object(path_map)
}
