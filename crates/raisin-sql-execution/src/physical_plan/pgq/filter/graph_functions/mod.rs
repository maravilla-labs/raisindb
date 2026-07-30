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
use crate::physical_plan::graph_algo::{GraphAdjacency, GraphEdge, GraphNodeId};
use crate::physical_plan::pgq::context::{PgqContext, ScopedAdjacency};
use crate::physical_plan::pgq::types::{SqlValue, VariableBinding};

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

pub(crate) use crate::physical_plan::pgq::context::EdgeWeightMap;

/// Build (or reuse) the adjacency list and weight map for the current context.
///
/// Two things happen here that did not before:
///
/// 1. **Relation-type pushdown.** This used to pass `None` to
///    `scan_relations_global`, loading EVERY relation in the branch across all
///    workspaces on EVERY algorithm invocation. The scope now comes from the
///    query's own patterns (see [`PgqContext::relation_type_scope`]). A single
///    type is pushed into storage; several are pushed as `None` plus an
///    in-memory filter, because the storage API takes one type — which is also
///    why `-[:a|b]->` must never be narrowed to just `a`.
/// 2. **A per-query memo.** Every scalar graph function used to rebuild the
///    whole adjacency, so `COLUMNS (pagerank(a), wcc(a), bfs(a,b))` scanned the
///    branch three times PER ROW. The memo is keyed by the relation-type set
///    and lives exactly as long as the query.
///
/// # This changes what the scalar graph functions compute
///
/// `COLUMNS (pagerank(a))` under `MATCH (a:User)-[:follows]->(b:User)` now runs
/// over the **follows graph**, not over every relation in the branch. That is
/// the projected-graph reading of `GRAPH_TABLE` and is the intended semantics,
/// but it is a numeric change for any query whose pattern names its types.
/// A pattern that leaves any hop untyped still gets the whole branch.
///
/// Relation types are compared **exactly**, matching both the storage prefix
/// scan and the single-hop matcher (`t == &rel.relation_type`). A pattern whose
/// spelling differs in case from the stored type already matched nothing before
/// this change.
pub(crate) async fn build_adjacency_with_weights<S: Storage>(
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<ScopedAdjacency> {
    build_adjacency_scoped(storage, context, context.relation_type_scope()).await
}

/// As [`build_adjacency_with_weights`], for a caller that knows the exact
/// relation types its pattern traverses.
pub(crate) async fn build_adjacency_scoped<S: Storage>(
    storage: &Arc<S>,
    context: &PgqContext,
    relation_types: &[String],
) -> Result<ScopedAdjacency> {
    let key = PgqContext::adjacency_key(relation_types);
    if let Some(hit) = context.cached_adjacency(&key) {
        return Ok(hit);
    }

    // One type can be pushed into the scan; several cannot, so they are
    // filtered in memory after an unfiltered scan.
    let pushed_down = match relation_types {
        [single] => Some(single.as_str()),
        _ => None,
    };
    let needs_memory_filter = relation_types.len() > 1;

    let scope = BranchScope::new(&context.tenant_id, &context.repo_id, &context.branch);
    let relations = storage
        .relations()
        .scan_relations_global(scope, pushed_down, context.revision.as_ref())
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

    let mut adjacency: GraphAdjacency = HashMap::new();
    let mut weights: EdgeWeightMap = HashMap::new();

    for (src_workspace, src_id, tgt_workspace, tgt_id, rel) in relations {
        if needs_memory_filter && !relation_types.contains(&rel.relation_type) {
            continue;
        }

        // Legacy weight map: keyed by node pair only, so it cannot hold two
        // relation types between the same pair, and it defaults a missing
        // weight to 1.0. Both flaws are why `ANY CHEAPEST` reads
        // `GraphEdge::weight` instead — see `graph_algo::cost`.
        weights.insert(
            (
                src_workspace.clone(),
                src_id.clone(),
                tgt_workspace.clone(),
                tgt_id.clone(),
            ),
            rel.weight.map(|w| w as f64).unwrap_or(1.0),
        );

        adjacency
            .entry((src_workspace, src_id))
            .or_default()
            .push(GraphEdge::new(
                tgt_workspace,
                tgt_id,
                rel.target_node_type,
                rel.relation_type,
                rel.weight,
            ));
    }

    let built: ScopedAdjacency = Arc::new((adjacency, weights));
    context.store_adjacency(&key, Arc::clone(&built));
    Ok(built)
}

/// Build graph adjacency from storage (without weights, for algorithms that don't need them)
pub(crate) async fn build_adjacency<S: Storage>(
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<ScopedAdjacency> {
    build_adjacency_with_weights(storage, context).await
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
