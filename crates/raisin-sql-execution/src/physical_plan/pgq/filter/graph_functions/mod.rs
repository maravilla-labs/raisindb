//! Graph algorithm function evaluation for GRAPH_TABLE queries
//!
//! Evaluates graph algorithm functions (pageRank, bfs, sssp, cdlp, lcc, wcc, etc.)
//! within GRAPH_TABLE COLUMNS expressions.

mod centrality;
mod community;
mod counting;
mod pathfinding;

use std::collections::HashMap;
use std::sync::Arc;

use raisin_sql::ast::{Expr, Literal};
use raisin_storage::{BranchScope, RelationRepository, Storage};

use super::Result;
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::pgq::context::PgqContext;
use crate::physical_plan::pgq::types::{SqlValue, VariableBinding};

use crate::physical_plan::cypher::algorithms::{
    self,
    types::{GraphAdjacency, GraphNodeId},
};

/// Check if a function name is a graph algorithm function
pub fn is_graph_function(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "pagerank"
            | "page_rank"
            | "bfs"
            | "breadth_first_search"
            | "sssp"
            | "shortest_path_distance"
            | "cdlp"
            | "community_detection"
            | "lcc"
            | "local_clustering_coefficient"
            | "clustering_coefficient"
            | "wcc"
            | "connected_component"
            | "component_id"
            | "componentid"
            | "louvain"
            | "triangle_count"
            | "trianglecount"
            | "betweenness"
            | "betweenness_centrality"
            | "closeness"
            | "closeness_centrality"
            | "degree"
            | "in_degree"
            | "out_degree"
            | "community_id"
            | "communityid"
            | "community_count"
            | "communitycount"
            | "component_count"
            | "componentcount"
    )
}

/// Evaluate a graph algorithm function
///
/// Called from `evaluate_expr` when a function call matches a known graph
/// algorithm name. Builds the adjacency graph from storage and delegates
/// to the appropriate algorithm implementation.
pub async fn evaluate_graph_function<S: Storage>(
    name: &str,
    args: &[Expr],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let name_lower = name.to_lowercase();

    match name_lower.as_str() {
        "bfs" | "breadth_first_search" => {
            pathfinding::evaluate_bfs(args, binding, storage, context).await
        }
        "sssp" | "shortest_path_distance" => {
            pathfinding::evaluate_sssp(args, binding, storage, context).await
        }
        "cdlp" | "community_detection" => {
            community::evaluate_cdlp(args, binding, storage, context).await
        }
        "lcc" | "local_clustering_coefficient" | "clustering_coefficient" => {
            counting::evaluate_lcc(args, binding, storage, context).await
        }
        "pagerank" | "page_rank" => {
            centrality::evaluate_pagerank(args, binding, storage, context).await
        }
        "wcc" | "connected_component" | "component_id" | "componentid" => {
            community::evaluate_wcc(args, binding, storage, context).await
        }
        "louvain" => community::evaluate_louvain(args, binding, storage, context).await,
        "triangle_count" | "trianglecount" => {
            counting::evaluate_triangle_count(args, binding, storage, context).await
        }
        "community_id" | "communityid" => {
            community::evaluate_community_id(args, binding, storage, context).await
        }
        "betweenness" | "betweenness_centrality" => {
            centrality::evaluate_betweenness(args, binding, storage, context).await
        }
        "closeness" | "closeness_centrality" => {
            centrality::evaluate_closeness(args, binding, storage, context).await
        }
        "degree" => centrality::evaluate_degree(args, binding, storage, context).await,
        "in_degree" => centrality::evaluate_in_degree(args, binding, storage, context).await,
        "out_degree" => centrality::evaluate_out_degree(args, binding, storage, context).await,
        "community_count" | "communitycount" => {
            community::evaluate_community_count(args, binding, storage, context).await
        }
        "component_count" | "componentcount" => {
            community::evaluate_component_count(args, binding, storage, context).await
        }
        _ => Err(ExecutionError::Validation(format!(
            "Unknown graph function: {}",
            name
        ))),
    }
}

// ---------------------------------------------------------------------------
// Helpers (shared across submodules)
// ---------------------------------------------------------------------------

/// Edge weight map: (source_workspace, source_id, target_workspace, target_id) -> weight
pub(crate) type EdgeWeightMap = HashMap<(String, String, String, String), f64>;

/// Build graph adjacency and weight map from storage for the current context
pub(crate) async fn build_adjacency_with_weights<S: Storage>(
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<(GraphAdjacency, EdgeWeightMap)> {
    let scope = BranchScope::new(&context.tenant_id, &context.repo_id, &context.branch);
    let relations = storage
        .relations()
        .scan_relations_global(scope, None, context.revision.as_ref())
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

    let mut adjacency: GraphAdjacency = HashMap::new();
    let mut weights: EdgeWeightMap = HashMap::new();
    for (src_workspace, src_id, tgt_workspace, tgt_id, rel) in relations {
        let weight = rel.weight.map(|w| w as f64).unwrap_or(1.0);
        weights.insert(
            (
                src_workspace.clone(),
                src_id.clone(),
                tgt_workspace.clone(),
                tgt_id.clone(),
            ),
            weight,
        );
        let source = (src_workspace, src_id);
        let target_entry = (tgt_workspace, tgt_id, rel.relation_type);
        adjacency.entry(source).or_default().push(target_entry);
    }

    Ok((adjacency, weights))
}

/// Build graph adjacency from storage (without weights, for algorithms that don't need them)
pub(crate) async fn build_adjacency<S: Storage>(
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<GraphAdjacency> {
    let (adjacency, _) = build_adjacency_with_weights(storage, context).await?;
    Ok(adjacency)
}

/// Extract a node identifier from the first argument expression
pub(crate) fn get_node_from_args(args: &[Expr], binding: &VariableBinding) -> Result<GraphNodeId> {
    if args.is_empty() {
        return Err(ExecutionError::Validation(
            "Graph function requires at least one argument (node variable)".into(),
        ));
    }
    match &args[0] {
        Expr::PropertyAccess {
            variable,
            properties,
            ..
        } if properties.is_empty() => {
            if let Some(node) = binding.get_node(variable) {
                Ok((node.workspace.clone(), node.id.clone()))
            } else {
                Err(ExecutionError::Validation(format!(
                    "Variable '{}' is not bound to a node",
                    variable
                )))
            }
        }
        _ => Err(ExecutionError::Validation(
            "First argument must be a node variable".into(),
        )),
    }
}

/// Extract a string literal from args at the given index
pub(crate) fn get_string_arg(args: &[Expr], index: usize) -> Option<String> {
    args.get(index).and_then(|expr| {
        if let Expr::Literal(Literal::String(s)) = expr {
            Some(s.clone())
        } else {
            None
        }
    })
}
