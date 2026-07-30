//! Weighted path search: A*, Dijkstra, and `ANY CHEAPEST`.
//!
//! Moved here from `physical_plan/cypher/algorithms/shortest_path.rs`.
//!
//! One core relaxation loop backs three entry points:
//! - [`astar_shortest_path`] — the historical Cypher `astar()` surface, with
//!   an infallible cost function.
//! - [`cheapest_path`] — `ANY CHEAPEST`. Same loop with a zero heuristic (A*
//!   degenerates to Dijkstra, which is correct and admissible with no domain
//!   knowledge) and a **fallible** cost function, so a missing or unusable
//!   edge weight fails loudly instead of defaulting to `1.0`.
//! - [`k_shortest_paths`] (in `yen.rs`) reuses the same adjacency shape.

use super::cost::{CostError, CostSpec};
use super::path::GraphPath;
use super::types::{GraphAdjacency, GraphEdge, GraphNodeId};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Min-heap entry for the priority queue.
#[derive(Copy, Clone, PartialEq)]
struct State {
    cost: f64,
    node_idx: usize,
}

impl Eq for State {}

impl Ord for State {
    fn cmp(&self, other: &Self) -> Ordering {
        // Costs are flipped so `BinaryHeap` (a max-heap) behaves as a min-heap.
        // Ties break on index to keep `PartialEq` and `Ord` consistent.
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
            .then_with(|| self.node_idx.cmp(&other.node_idx))
    }
}

impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A* / Dijkstra over `adjacency`, with a fallible per-edge cost function.
///
/// `cost_fn` receives the source node and the edge being relaxed, and may
/// reject the edge outright — that rejection aborts the search rather than
/// being swallowed, which is what makes `ANY CHEAPEST` honest about missing
/// weights.
fn astar_core<C, H>(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    end: &GraphNodeId,
    cost_fn: C,
    heuristic_fn: H,
) -> Result<Option<GraphPath>, CostError>
where
    C: Fn(&GraphNodeId, &GraphEdge) -> Result<f64, CostError>,
    H: Fn(&GraphNodeId) -> f64,
{
    if start == end {
        return Ok(Some(GraphPath::from_key(start)));
    }

    // Map nodes to integers so `dist` can be a flat Vec.
    let mut node_to_idx: HashMap<GraphNodeId, usize> = HashMap::new();
    let mut idx_to_node: Vec<GraphNodeId> = Vec::new();

    let mut all_nodes = HashSet::new();
    all_nodes.insert(start.clone());
    all_nodes.insert(end.clone());
    for (src, targets) in adjacency {
        all_nodes.insert(src.clone());
        for edge in targets {
            all_nodes.insert(edge.target());
        }
    }

    for (i, node) in all_nodes.into_iter().enumerate() {
        node_to_idx.insert(node.clone(), i);
        idx_to_node.push(node);
    }

    let (Some(&start_idx), Some(&end_idx)) = (node_to_idx.get(start), node_to_idx.get(end)) else {
        return Ok(None);
    };

    let mut dist: Vec<f64> = vec![f64::INFINITY; idx_to_node.len()];
    dist[start_idx] = 0.0;

    // parent[node_idx] = (parent_idx, edge taken to reach it)
    let mut parent: HashMap<usize, (usize, GraphEdge)> = HashMap::new();

    let mut heap = BinaryHeap::new();
    heap.push(State {
        cost: 0.0,
        node_idx: start_idx,
    });

    while let Some(State { cost, node_idx }) = heap.pop() {
        if node_idx == end_idx {
            return Ok(Some(rebuild(&idx_to_node, &parent, start_idx, end_idx)));
        }

        // Stale heap entry from an already-improved relaxation.
        if cost > dist[node_idx] + heuristic_fn(&idx_to_node[node_idx]) {
            continue;
        }

        let current_node = idx_to_node[node_idx].clone();
        let Some(neighbors) = adjacency.get(&current_node) else {
            continue;
        };

        for edge in neighbors {
            let next_node = edge.target();
            let Some(&next_idx) = node_to_idx.get(&next_node) else {
                continue;
            };

            let edge_cost = cost_fn(&current_node, edge)?;
            let next_dist = dist[node_idx] + edge_cost;

            if next_dist < dist[next_idx] {
                dist[next_idx] = next_dist;
                parent.insert(next_idx, (node_idx, edge.clone()));
                heap.push(State {
                    cost: next_dist + heuristic_fn(&next_node),
                    node_idx: next_idx,
                });
            }
        }
    }

    Ok(None)
}

/// Walk the parent pointers back from `end_idx` and build the ordered path.
fn rebuild(
    idx_to_node: &[GraphNodeId],
    parent: &HashMap<usize, (usize, GraphEdge)>,
    start_idx: usize,
    end_idx: usize,
) -> GraphPath {
    let mut hops: Vec<GraphEdge> = Vec::new();
    let mut curr = end_idx;

    while curr != start_idx {
        match parent.get(&curr) {
            Some((prev, edge)) => {
                hops.push(edge.clone());
                curr = *prev;
            }
            None => break,
        }
    }

    hops.reverse();

    let mut path = GraphPath::from_key(&idx_to_node[start_idx]);
    for edge in &hops {
        path.push(edge);
    }
    path
}

/// A* shortest path with a caller-supplied cost and heuristic.
///
/// Retained with the historical `(source, target, relation_type) -> f64`
/// signature so the Cypher `astar()` surface is untouched by the move.
pub fn astar_shortest_path<C, H>(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    end: &GraphNodeId,
    cost_fn: C,
    heuristic_fn: H,
) -> Option<GraphPath>
where
    C: Fn(&GraphNodeId, &GraphNodeId, &str) -> f64,
    H: Fn(&GraphNodeId) -> f64,
{
    astar_core(
        adjacency,
        start,
        end,
        |source, edge| Ok(cost_fn(source, &edge.target(), &edge.relation_type)),
        heuristic_fn,
    )
    // The closure above is infallible, so the `Err` arm is unreachable.
    .unwrap_or(None)
}

/// `ANY CHEAPEST` — the minimum-total-cost path under `spec`.
///
/// A zero heuristic degenerates A* to Dijkstra, which is admissible without
/// any domain knowledge. A geospatial heuristic backed by the geo index is an
/// explicit non-goal.
///
/// # Errors
/// Returns [`CostError`] if any **traversed** edge has no weight, or a weight
/// that is not positive and finite. It never falls back to a hop count: a
/// weighted query that silently answers with an unweighted result is the exact
/// failure class this signature exists to prevent.
///
/// "Traversed" means *relaxed*, not *on the answer*: Dijkstra prices every edge
/// it examines before the target is settled, so one unweighted edge reachable
/// from the start fails the query even if the cheapest path avoids it. That is
/// deliberate — the alternative is deciding per edge whether a missing weight
/// mattered, and being wrong about it silently.
pub fn cheapest_path(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    end: &GraphNodeId,
    spec: &CostSpec,
) -> Result<Option<GraphPath>, CostError> {
    astar_core(
        adjacency,
        start,
        end,
        |source, edge| spec.edge_cost(source, edge),
        |_| 0.0,
    )
}
