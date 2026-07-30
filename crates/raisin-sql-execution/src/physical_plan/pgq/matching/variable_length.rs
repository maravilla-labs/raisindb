//! Variable-Length Path Matching for PGQ/GRAPH_TABLE
//!
//! Matches patterns like `(a)-[:TYPE]->{1,3}(b)` (and the deprecated Cypher-style
//! `(a)-[:TYPE*1..3]->(b)`) by depth-first enumeration, then applies the path
//! selector.
//!
//! # What changed in this pass
//!
//! The ordered path used to be computed and thrown away: only the first and last
//! nodes were bound, and the hop count was smuggled out by rewriting
//! `relation_type` to `"TYPE[3]"` so `CARDINALITY(r)` could parse it back out.
//! The full [`GraphPath`] is now bound, `relation_type` is left verbatim, and
//! `CARDINALITY` reads the bound path.

use std::sync::Arc;

use raisin_sql::ast::{Direction, NodePattern, PathQuantifier, RelationshipPattern};
use raisin_storage::{RelationRepository, Storage};

use super::matches_label;
use super::selectors::{apply_selector, selector_over_bound_endpoints, PathSemantics};
use super::traversal::{
    build_adjacency, enumerate_paths, ScannedRelation, TraversalParams, MAX_PATHS,
};
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::graph_algo::{GraphAdjacency, GraphNodeId, GraphPath};
use crate::physical_plan::pgq::context::PgqContext;
use crate::physical_plan::pgq::types::{NodeInfo, RelationInfo, VariableBinding};

type Result<T> = std::result::Result<T, ExecutionError>;

/// Execute a variable-length relationship pattern.
///
/// # Arguments
/// * `source_pattern` - Source node pattern
/// * `rel_pattern` - Relationship pattern with quantifier specification
/// * `target_pattern` - Target node pattern
/// * `semantics` - Path variable, selector and restrictor for the enclosing path
/// * `bindings` - Existing variable bindings (for chained patterns)
/// * `storage` - Storage backend
/// * `context` - PGQ execution context
pub async fn execute_variable_length_pattern<S: Storage>(
    source_pattern: &NodePattern,
    rel_pattern: &RelationshipPattern,
    target_pattern: &NodePattern,
    semantics: &PathSemantics,
    bindings: Vec<VariableBinding>,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<Vec<VariableBinding>> {
    let quantifier = rel_pattern.quantifier.as_ref().ok_or_else(|| {
        ExecutionError::Validation("Variable-length pattern requires a quantifier".to_string())
    })?;

    let params = TraversalParams {
        min_depth: quantifier.min,
        max_depth: quantifier.max.unwrap_or(PathQuantifier::DEFAULT_MAX),
        restrictor: semantics.restrictor,
        target_labels: target_pattern.labels.clone(),
    };

    tracing::info!(
        "PGQ: variable-length pattern min={} max={} direction={:?} restrictor={} selector={:?}",
        params.min_depth,
        params.max_depth,
        rel_pattern.direction,
        params.restrictor.as_str(),
        semantics.selector,
    );

    if params.max_depth > 5 {
        tracing::warn!("PGQ: Variable-length pattern with depth > 5 may be expensive");
    }

    let relations = scan_relations(rel_pattern, storage, context).await?;
    if relations.is_empty() {
        return Ok(vec![]);
    }

    let (forward, reverse) = build_adjacency(&relations, &rel_pattern.types);
    let adjacency = match rel_pattern.direction {
        Direction::Right => &forward,
        Direction::Left => &reverse,
        Direction::Any => {
            // Bidirectional variable-length traversal would need the union of
            // both adjacencies; today the forward one is used. Reported, not
            // silent.
            tracing::warn!("PGQ: Bidirectional variable-length paths use forward direction only");
            &forward
        }
    };

    let source_var = source_pattern.variable.as_ref();
    if source_var.is_none() && target_pattern.variable.is_none() {
        return Err(ExecutionError::Validation(
            "Variable-length patterns require at least one node variable".to_string(),
        ));
    }
    let Some(source_var) = source_var else {
        return Ok(vec![]);
    };

    let mut budget = MAX_PATHS;
    let mut result_bindings = Vec::new();

    for binding in bindings {
        // Fast path: both endpoints bound and the default restrictor in force,
        // so the shared BFS / Dijkstra answer the selector exactly and nothing
        // has to be enumerated.
        let direct = match (
            semantics.selector,
            binding.get_node(source_var),
            target_pattern
                .variable
                .as_ref()
                .and_then(|v| binding.get_node(v)),
        ) {
            (Some(selector), Some(source), Some(target)) => selector_over_bound_endpoints(
                selector,
                semantics.restrictor,
                params.min_depth,
                params.max_depth,
                adjacency,
                &(source.workspace.clone(), source.id.clone()),
                &source.node_type,
                &(target.workspace.clone(), target.id.clone()),
                budget,
            )?,
            _ => None,
        };

        let selected = match direct {
            Some(paths) => {
                // The fast path did not enumerate, but it still consumed part of
                // the pattern's path budget; charge it so the cap means the same
                // thing whichever route a query took.
                budget = budget.saturating_sub(paths.len());
                paths
            }
            None => {
                let paths = match binding.get_node(source_var) {
                    // Source already bound by an earlier pattern element.
                    Some(source_node) => enumerate_paths(
                        &(source_node.workspace.clone(), source_node.id.clone()),
                        &source_node.node_type,
                        adjacency,
                        &params,
                        &mut budget,
                    )?,
                    // Source unbound: every node matching the source labels starts a path.
                    None => enumerate_from_all_sources(
                        source_pattern,
                        rel_pattern,
                        adjacency,
                        &relations,
                        &params,
                        &mut budget,
                    )?,
                };
                apply_selector(semantics.selector, paths)?
            }
        };

        for path in selected {
            result_bindings.push(bind_path(
                &binding,
                &path,
                source_var,
                rel_pattern,
                target_pattern,
                semantics,
            ));
        }
    }

    tracing::info!(
        "PGQ: variable-length pattern produced {} bindings",
        result_bindings.len()
    );

    Ok(result_bindings)
}

/// Scan the relations the pattern can traverse.
///
/// `scan_relations_global` takes a single `Option<&str>`, so only a
/// single-typed pattern can push its filter down to storage. A multi-type
/// alternation such as `-[:knows|follows]->{1,3}` must scan unfiltered and be
/// filtered while the adjacency is built; taking `types.first()` — as this did —
/// silently dropped every `follows` edge and returned wrong results for a
/// documented feature.
async fn scan_relations<S: Storage>(
    rel_pattern: &RelationshipPattern,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<Vec<ScannedRelation>> {
    let pushdown = match rel_pattern.types.as_slice() {
        [single] => Some(single.as_str()),
        _ => None,
    };

    if rel_pattern.types.len() > 1 {
        tracing::debug!(
            "PGQ: {} relation types in alternation; scanning unfiltered and filtering in memory",
            rel_pattern.types.len()
        );
    }

    storage
        .relations()
        .scan_relations_global(
            raisin_storage::BranchScope::new(&context.tenant_id, &context.repo_id, &context.branch),
            pushdown,
            context.revision.as_ref(),
        )
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))
}

/// Enumerate paths from every node that can start this pattern.
fn enumerate_from_all_sources(
    source_pattern: &NodePattern,
    rel_pattern: &RelationshipPattern,
    adjacency: &GraphAdjacency,
    relations: &[ScannedRelation],
    params: &TraversalParams,
    budget: &mut usize,
) -> Result<Vec<GraphPath>> {
    let mut paths = Vec::new();

    for start in adjacency.keys() {
        let node_type = source_node_type(start, rel_pattern.direction, relations);
        if !matches_label(&source_pattern.labels, &node_type) {
            continue;
        }
        paths.extend(enumerate_paths(
            start, &node_type, adjacency, params, budget,
        )?);
    }

    Ok(paths)
}

/// Resolve the node type of a traversal start node.
///
/// For a LEFT (backward) traversal the start node appears as a relation
/// *target*, so its type is `target_node_type`; for RIGHT it appears as a
/// *source*.
fn source_node_type(
    start: &GraphNodeId,
    direction: Direction,
    relations: &[ScannedRelation],
) -> String {
    let (workspace, id) = start;
    if matches!(direction, Direction::Left) {
        relations
            .iter()
            .find(|(_, _, tw, tid, _)| tw == workspace && tid == id)
            .map(|(_, _, _, _, r)| r.target_node_type.clone())
            .unwrap_or_default()
    } else {
        relations
            .iter()
            .find(|(sw, sid, _, _, _)| sw == workspace && sid == id)
            .map(|(_, _, _, _, r)| r.source_node_type.clone())
            .unwrap_or_default()
    }
}

/// Turn one matched path into a binding.
///
/// The path is bound under the path variable when the pattern declared one, and
/// **also** under the relationship variable. Binding it under the relationship
/// variable is what lets `CARDINALITY(r)` keep working without the old
/// `"TYPE[3]"` string encoding: the accessor reads a real path instead of
/// parsing a mangled relation type.
fn bind_path(
    base: &VariableBinding,
    path: &GraphPath,
    source_var: &str,
    rel_pattern: &RelationshipPattern,
    target_pattern: &NodePattern,
    semantics: &PathSemantics,
) -> VariableBinding {
    let mut binding = base.clone();

    if let Some(first) = path.first() {
        binding.bind_node(
            source_var.to_string(),
            NodeInfo::new(
                first.id.clone(),
                first.workspace.clone(),
                first.node_type.clone(),
            ),
        );
    }

    if let (Some(target_var), Some(last)) = (target_pattern.variable.as_ref(), path.last()) {
        binding.bind_node(
            target_var.clone(),
            NodeInfo::new(
                last.id.clone(),
                last.workspace.clone(),
                last.node_type.clone(),
            ),
        );
    }

    if let Some(rel_var) = &rel_pattern.variable {
        binding.bind_path(rel_var.clone(), path.clone());
        if let Some(first_edge) = path.edges.first() {
            // relation_type verbatim - no length encoding.
            binding.bind_relation(
                rel_var.clone(),
                RelationInfo::new(
                    first_edge.relation_type.clone(),
                    first_edge.weight,
                    first_edge.source_id.clone(),
                    first_edge.target_id.clone(),
                ),
            );
        }
    }

    if let Some(path_var) = &semantics.variable {
        binding.bind_path(path_var.clone(), path.clone());
    }

    binding
}

#[cfg(test)]
#[path = "variable_length_tests.rs"]
mod tests;
