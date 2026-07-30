//! Adjacency construction and depth-first path enumeration for SQL/PGQ.
//!
//! Split out of `variable_length.rs` so that file stays about *binding* and this
//! one stays about *graph walking*. The adjacency and path types are the shared
//! ones from [`crate::physical_plan::graph_algo`], so an enumerated path and a
//! path returned by `shortest_path` are the same value.

use std::collections::HashMap;

use raisin_models::nodes::FullRelation;

use super::matches_label;
use super::selectors::RestrictorExt;
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::graph_algo::{GraphAdjacency, GraphEdge, GraphNodeId, GraphPath};
use raisin_sql::ast::PathRestrictor;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Maximum paths a single variable-length pattern may enumerate.
///
/// Exceeding it is a hard error, never a truncated answer — see
/// [`enumerate_paths`].
pub const MAX_PATHS: usize = 10_000;

/// A scanned relation tuple as produced by `scan_relations_global`.
pub type ScannedRelation = (String, String, String, String, FullRelation);

/// Build forward and reverse adjacency from scanned relations.
///
/// `accepted_types` is the pattern's relation-type list. When it holds more than
/// one type the storage scan cannot filter (`scan_relations_global` takes a
/// single `Option<&str>`), so the filter is applied here instead. An empty slice
/// accepts every type.
///
/// This is the second half of the multi-type alternation fix: the caller pushes
/// a single type down to storage and otherwise scans unfiltered, and this
/// function keeps only the types the pattern actually named.
pub fn build_adjacency(
    relations: &[ScannedRelation],
    accepted_types: &[String],
) -> (GraphAdjacency, GraphAdjacency) {
    let mut forward: GraphAdjacency = HashMap::new();
    let mut reverse: GraphAdjacency = HashMap::new();

    for (src_workspace, src_id, tgt_workspace, tgt_id, rel) in relations {
        if !accepted_types.is_empty()
            && !accepted_types
                .iter()
                .any(|t| t.eq_ignore_ascii_case(&rel.relation_type))
        {
            continue;
        }

        forward
            .entry((src_workspace.clone(), src_id.clone()))
            .or_default()
            .push(GraphEdge::new(
                tgt_workspace,
                tgt_id,
                &rel.target_node_type,
                &rel.relation_type,
                rel.weight,
            ));

        reverse
            .entry((tgt_workspace.clone(), tgt_id.clone()))
            .or_default()
            .push(GraphEdge::new(
                src_workspace,
                src_id,
                &rel.source_node_type,
                &rel.relation_type,
                rel.weight,
            ));
    }

    (forward, reverse)
}

/// Everything the DFS needs beyond the graph itself.
#[derive(Debug, Clone)]
pub struct TraversalParams {
    /// Minimum hop count for a path to be emitted.
    pub min_depth: u32,
    /// Maximum hop count to traverse.
    pub max_depth: u32,
    /// Distinctness rule enforced while walking.
    pub restrictor: PathRestrictor,
    /// Labels the final node must match (empty = any).
    pub target_labels: Vec<String>,
}

/// Enumerate every path from `start` satisfying `params`.
///
/// # Truncation is an error, not a shorter answer
///
/// `budget` is the number of paths still allowed across the whole pattern. When
/// it runs out this returns `Err` naming the cap and the remedies, rather than
/// returning what it has. A partial path set is indistinguishable from a
/// complete one once it reaches the client, and "looks complete but is not" is
/// precisely the failure mode this codebase keeps removing.
pub fn enumerate_paths(
    start: &GraphNodeId,
    start_node_type: &str,
    adjacency: &GraphAdjacency,
    params: &TraversalParams,
    budget: &mut usize,
) -> Result<Vec<GraphPath>> {
    let mut out = Vec::new();
    let initial = GraphPath::from_key(start).with_start_node_type(start_node_type);
    walk(start, adjacency, initial, params, 0, budget, &mut out)?;
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    current: &GraphNodeId,
    adjacency: &GraphAdjacency,
    path: GraphPath,
    params: &TraversalParams,
    depth: u32,
    budget: &mut usize,
    out: &mut Vec<GraphPath>,
) -> Result<()> {
    // Emit the current path if it is long enough and ends on a matching label.
    if depth >= params.min_depth {
        let ends_on_match = path
            .last()
            .is_some_and(|n| matches_label(&params.target_labels, &n.node_type));
        if ends_on_match {
            if *budget == 0 {
                return Err(path_cap_exceeded());
            }
            *budget -= 1;
            out.push(path.clone());
        }
    }

    if depth >= params.max_depth {
        return Ok(());
    }

    let Some(neighbours) = adjacency.get(current) else {
        return Ok(());
    };

    for edge in neighbours {
        if !params.restrictor.allows_extension(&path, edge) {
            continue;
        }
        let next_key = edge.target();
        walk(
            &next_key,
            adjacency,
            path.extended(edge),
            params,
            depth + 1,
            budget,
            out,
        )?;
    }

    Ok(())
}

fn path_cap_exceeded() -> ExecutionError {
    ExecutionError::Validation(format!(
        "variable-length path match exceeded the {MAX_PATHS} path cap. Partial results are \
         not returned because they are indistinguishable from a complete answer. Narrow the \
         quantifier (for example ->{{1,2}}), add a selector such as ANY SHORTEST, or filter \
         the endpoints with a label or WHERE clause."
    ))
}

#[cfg(test)]
#[path = "traversal_tests.rs"]
mod tests;
