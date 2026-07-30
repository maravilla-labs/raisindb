//! Path selector and restrictor semantics for SQL/PGQ variable-length matching.
//!
//! The two are **not** the same kind of thing and are applied at different times:
//!
//! * A **restrictor** (`WALK` / `TRAIL` / `ACYCLIC`) constrains which walks are
//!   paths at all. It is enforced **during** matching, by the DFS, because it
//!   prunes the search — see [`RestrictorExt::allows_extension`].
//! * A **selector** (`ANY`, `ANY SHORTEST`, `ALL SHORTEST`, `ANY CHEAPEST`)
//!   chooses among the paths that survive. It is applied **after** matching —
//!   see [`apply_selector`].
//!
//! Getting that order wrong changes results: `ALL SHORTEST TRAIL` means "the
//! shortest of the edge-distinct paths", not "the edge-distinct ones among the
//! shortest paths".
//!
//! The enums live in `raisin_sql::ast` and are re-exported here rather than
//! mirrored, so the grammar and the engine cannot drift apart.

use std::collections::HashMap;

use raisin_sql::ast::PathPattern;
pub use raisin_sql::ast::{PathRestrictor, PathSelector};

use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::graph_algo::{
    all_shortest_paths, cheapest_path as algo_cheapest_path, shortest_path, CostSpec,
    GraphAdjacency, GraphEdge, GraphNodeId, GraphPath, PathEdge,
};

type Result<T> = std::result::Result<T, ExecutionError>;

/// Execution-side behaviour of a path restrictor.
pub trait RestrictorExt {
    /// Whether `path` may be extended along `edge`.
    fn allows_extension(&self, path: &GraphPath, edge: &GraphEdge) -> bool;
}

impl RestrictorExt for PathRestrictor {
    fn allows_extension(&self, path: &GraphPath, edge: &GraphEdge) -> bool {
        match self {
            // No distinctness requirement; bounded only by the depth cap.
            PathRestrictor::Walk => true,
            // Edge-distinct.
            PathRestrictor::Trail => !path.would_repeat_edge(edge),
            // Node-distinct. This is the default, and is exactly what the DFS
            // has always done, so an unannotated query behaves as before.
            PathRestrictor::Acyclic => !path.contains_node(&edge.target_id, &edge.target_workspace),
        }
    }
}

/// Everything a top-level path pattern says about *how* to match it.
#[derive(Debug, Clone)]
pub struct PathSemantics {
    /// The path variable, if the pattern was written `MATCH p = …`.
    pub variable: Option<String>,
    /// Selector, if any. `None` means "every path".
    pub selector: Option<PathSelector>,
    /// Restrictor in force, defaulted by the AST when none was written.
    pub restrictor: PathRestrictor,
}

impl Default for PathSemantics {
    fn default() -> Self {
        Self {
            variable: None,
            selector: None,
            restrictor: PathRestrictor::DEFAULT,
        }
    }
}

impl PathSemantics {
    /// Semantics of a pattern written with no selector and no restrictor —
    /// exactly the behaviour PGQ had before path variables existed.
    pub fn legacy() -> Self {
        Self::default()
    }

    /// True when a path value must be produced for this pattern.
    pub fn binds_path(&self) -> bool {
        self.variable.is_some()
    }
}

/// Derive the matching semantics of a top-level path pattern.
pub fn path_semantics_for(pattern: &PathPattern) -> PathSemantics {
    PathSemantics {
        variable: pattern.variable.clone(),
        selector: pattern.selector,
        restrictor: pattern.effective_restrictor(),
    }
}

/// Answer a selector directly from the shared graph algorithms.
///
/// Applicable only when both endpoints are bound to concrete nodes and the
/// default `ACYCLIC` restrictor is in force. Under those conditions the
/// algorithms are exact: a minimum-hop walk never repeats a node, and neither
/// does a minimum-cost walk when every weight is positive, so BFS and Dijkstra
/// already produce acyclic paths.
///
/// `min_hops > 1` is not applicable either — the algorithms answer "the
/// shortest", which a lower bound may exclude, and there is no way to ask them
/// for "the shortest of at least length n".
///
/// Returns `Ok(None)` when the fast path does not apply; the caller then
/// enumerates and calls [`apply_selector`].
#[allow(clippy::too_many_arguments)]
pub fn selector_over_bound_endpoints(
    selector: PathSelector,
    restrictor: PathRestrictor,
    min_hops: u32,
    max_hops: u32,
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    start_node_type: &str,
    end: &GraphNodeId,
    max_paths: usize,
) -> Result<Option<Vec<GraphPath>>> {
    if restrictor != PathRestrictor::Acyclic || min_hops > 1 {
        return Ok(None);
    }

    let paths = match selector {
        // ANY promises no minimality, so answering it with a shortest path
        // would be a stronger promise than the query asked for - harmless, but
        // it would make ANY and ANY SHORTEST indistinguishable in practice and
        // hide the difference from anyone reading results. Enumerate instead.
        PathSelector::Any => return Ok(None),
        PathSelector::AnyShortest => shortest_path(adjacency, start, end, max_hops)
            .into_iter()
            .collect(),
        PathSelector::AllShortest => all_shortest_paths(adjacency, start, end, max_hops, max_paths),
        PathSelector::AnyCheapest => {
            algo_cheapest_path(adjacency, start, end, &CostSpec::EdgeWeight)
                .map_err(|e| ExecutionError::Validation(e.to_string()))?
                .into_iter()
                .filter(|p| p.length() as u32 <= max_hops)
                .collect()
        }
    };

    Ok(Some(
        paths
            .into_iter()
            .filter(|p| p.length() as u32 >= min_hops)
            .map(|p| p.with_start_node_type(start_node_type))
            .collect(),
    ))
}

/// Group key for selector application: the endpoints of a path.
type Endpoints = (String, String, String, String);

fn endpoints(path: &GraphPath) -> Endpoints {
    let first = path.first();
    let last = path.last();
    (
        first.map(|n| n.workspace.clone()).unwrap_or_default(),
        first.map(|n| n.id.clone()).unwrap_or_default(),
        last.map(|n| n.workspace.clone()).unwrap_or_default(),
        last.map(|n| n.id.clone()).unwrap_or_default(),
    )
}

/// Apply a selector to a set of enumerated paths.
///
/// Paths are grouped by endpoint pair first — a selector is defined per pair of
/// endpoints, not over the whole result. Group order and the order within each
/// group are preserved from the input, so results are deterministic for a given
/// traversal order.
///
/// `None` returns the input unchanged (no selector = every path).
///
/// # Why this filters instead of always calling a shortest-path algorithm
///
/// Selecting from the enumerated set is the only formulation that composes with
/// the restrictor and with a free (unbound) target. `ANY SHORTEST TRAIL` asks
/// for the shortest *edge-distinct* path; a plain BFS answers the shortest walk
/// and could return a path the restrictor forbids. When both endpoints are
/// bound and the restrictor is the default, the caller takes
/// [`selector_over_bound_endpoints`] instead and never enumerates.
pub fn apply_selector(
    selector: Option<PathSelector>,
    paths: Vec<GraphPath>,
) -> Result<Vec<GraphPath>> {
    let Some(selector) = selector else {
        return Ok(paths);
    };

    // Preserve first-seen group order.
    let mut order: Vec<Endpoints> = Vec::new();
    let mut groups: HashMap<Endpoints, Vec<GraphPath>> = HashMap::new();
    for path in paths {
        let key = endpoints(&path);
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push(path);
    }

    let mut selected = Vec::with_capacity(order.len());
    for key in order {
        let group = groups.remove(&key).unwrap_or_default();
        match selector {
            PathSelector::Any => selected.extend(group.into_iter().next()),
            PathSelector::AnyShortest => {
                selected.extend(group.into_iter().min_by_key(GraphPath::length))
            }
            PathSelector::AllShortest => {
                let Some(min) = group.iter().map(GraphPath::length).min() else {
                    continue;
                };
                selected.extend(group.into_iter().filter(|p| p.length() == min));
            }
            PathSelector::AnyCheapest => {
                if let Some(cheapest) = cheapest_of(group)? {
                    selected.push(cheapest);
                }
            }
        }
    }

    Ok(selected)
}

/// Pick the minimum-total-cost path from an enumerated group.
///
/// Prices every edge through [`CostSpec::EdgeWeight`], the same resolver the
/// Dijkstra path uses, so a missing or unusable weight produces one error text
/// whichever route the query took. It deliberately never defaults a missing
/// weight to 1.0: a routing query that silently answers "shortest" while
/// claiming "cheapest" is the silent-wrong-results class this codebase has been
/// eliminating.
fn cheapest_of(group: Vec<GraphPath>) -> Result<Option<GraphPath>> {
    let mut best: Option<(f64, GraphPath)> = None;

    for path in group {
        let mut cost = 0.0_f64;
        for edge in &path.edges {
            cost += CostSpec::EdgeWeight
                .edge_cost(&edge_source(edge), &as_graph_edge(edge))
                .map_err(|e| ExecutionError::Validation(e.to_string()))?;
        }

        match &best {
            Some((best_cost, _)) if *best_cost <= cost => {}
            _ => best = Some((cost, path)),
        }
    }

    Ok(best.map(|(_, path)| path))
}

fn edge_source(edge: &PathEdge) -> GraphNodeId {
    (edge.source_workspace.clone(), edge.source_id.clone())
}

fn as_graph_edge(edge: &PathEdge) -> GraphEdge {
    GraphEdge::new(
        &edge.target_workspace,
        &edge.target_id,
        String::new(),
        &edge.relation_type,
        edge.weight,
    )
}

#[cfg(test)]
#[path = "selectors_tests.rs"]
mod tests;
