//! Single-Source Shortest Path (SSSP) with Weighted Edges
//!
//! Implements Dijkstra's algorithm on the directed `GraphAdjacency` HashMap
//! with a user-supplied weight function that maps relation types to edge costs.
//!
//! Time Complexity: O((V + E) log V)
//! Space Complexity: O(V)

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use super::types::{GraphAdjacency, GraphNodeId};

/// Priority-queue state for Dijkstra (min-heap via reversed ordering).
#[derive(Clone, PartialEq)]
struct State {
    cost: f64,
    node: GraphNodeId,
}

impl Eq for State {}

impl Ord for State {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
            .then_with(|| self.node.cmp(&other.node))
    }
}

impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Compute shortest weighted distances from `source` to all reachable nodes.
///
/// Uses Dijkstra's algorithm. The `weight_fn` maps a relation type string
/// to a non-negative edge weight (e.g., `|_| 1.0` for unit weights).
///
/// # Arguments
/// * `adjacency` - Directed graph adjacency list
/// * `source` - Starting node
/// * `weight_fn` - Function mapping relation_type (&str) to edge weight (f64 >= 0)
///
/// # Returns
/// * HashMap of (node -> distance) for all reachable nodes (including source with 0.0)
pub fn sssp_distances(
    adjacency: &GraphAdjacency,
    source: &GraphNodeId,
    weight_fn: impl Fn(&str) -> f64,
) -> HashMap<GraphNodeId, f64> {
    let mut dist: HashMap<GraphNodeId, f64> = HashMap::new();
    let mut heap = BinaryHeap::new();

    dist.insert(source.clone(), 0.0);
    heap.push(State {
        cost: 0.0,
        node: source.clone(),
    });

    while let Some(State { cost, node }) = heap.pop() {
        // Skip if we already found a shorter path
        if let Some(&best) = dist.get(&node) {
            if cost > best {
                continue;
            }
        }

        if let Some(neighbors) = adjacency.get(&node) {
            for (tgt_w, tgt_id, rel_type) in neighbors {
                let next = (tgt_w.clone(), tgt_id.clone());
                let edge_weight = weight_fn(rel_type);
                let next_cost = cost + edge_weight;

                let is_shorter = match dist.get(&next) {
                    Some(&current_best) => next_cost < current_best,
                    None => true,
                };

                if is_shorter {
                    dist.insert(next.clone(), next_cost);
                    heap.push(State {
                        cost: next_cost,
                        node: next,
                    });
                }
            }
        }
    }

    dist
}

/// Compute the shortest weighted distance between two specific nodes.
///
/// Returns `None` if `target` is not reachable from `source`.
///
/// # Arguments
/// * `adjacency` - Directed graph adjacency list
/// * `source` - Starting node
/// * `target` - Destination node
/// * `weight_fn` - Function mapping relation_type (&str) to edge weight (f64 >= 0)
///
/// # Returns
/// * `Some(distance)` if target is reachable, `None` otherwise
pub fn node_sssp_distance(
    adjacency: &GraphAdjacency,
    source: &GraphNodeId,
    target: &GraphNodeId,
    weight_fn: impl Fn(&str) -> f64,
) -> Option<f64> {
    if source == target {
        return Some(0.0);
    }

    let distances = sssp_distances(adjacency, source, weight_fn);
    distances.get(target).copied()
}

/// Compute shortest weighted distances using per-edge weight map.
///
/// The weight map maps `(src_workspace, src_id, tgt_workspace, tgt_id)` to a weight.
/// Edges not in the map default to 1.0.
pub fn sssp_distances_weighted(
    adjacency: &GraphAdjacency,
    source: &GraphNodeId,
    edge_weights: &HashMap<(String, String, String, String), f64>,
) -> HashMap<GraphNodeId, f64> {
    let mut dist: HashMap<GraphNodeId, f64> = HashMap::new();
    let mut heap = BinaryHeap::new();

    dist.insert(source.clone(), 0.0);
    heap.push(State {
        cost: 0.0,
        node: source.clone(),
    });

    while let Some(State { cost, node }) = heap.pop() {
        if let Some(&best) = dist.get(&node) {
            if cost > best {
                continue;
            }
        }

        if let Some(neighbors) = adjacency.get(&node) {
            for (tgt_w, tgt_id, _rel_type) in neighbors {
                let next = (tgt_w.clone(), tgt_id.clone());
                let edge_weight = edge_weights
                    .get(&(
                        node.0.clone(),
                        node.1.clone(),
                        tgt_w.clone(),
                        tgt_id.clone(),
                    ))
                    .copied()
                    .unwrap_or(1.0);
                let next_cost = cost + edge_weight;

                let is_shorter = match dist.get(&next) {
                    Some(&current_best) => next_cost < current_best,
                    None => true,
                };

                if is_shorter {
                    dist.insert(next.clone(), next_cost);
                    heap.push(State {
                        cost: next_cost,
                        node: next,
                    });
                }
            }
        }
    }

    dist
}

/// Compute shortest weighted distance between two nodes using per-edge weight map.
pub fn node_sssp_distance_weighted(
    adjacency: &GraphAdjacency,
    source: &GraphNodeId,
    target: &GraphNodeId,
    edge_weights: &HashMap<(String, String, String, String), f64>,
) -> Option<f64> {
    if source == target {
        return Some(0.0);
    }
    let distances = sssp_distances_weighted(adjacency, source, edge_weights);
    distances.get(target).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_weighted_graph() -> GraphAdjacency {
        let mut graph = HashMap::new();

        // A --FAST--> B --FAST--> D  (cost 1+1 = 2)
        // A --SLOW--> C --FAST--> D  (cost 5+1 = 6)
        // A --FAST--> C             (cost 1, but A->C->D = 1+1 = 2, same as A->B->D)
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![
                ("ws".to_string(), "B".to_string(), "FAST".to_string()),
                ("ws".to_string(), "C".to_string(), "SLOW".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![("ws".to_string(), "D".to_string(), "FAST".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![("ws".to_string(), "D".to_string(), "FAST".to_string())],
        );

        graph
    }

    fn weight_fn(rel_type: &str) -> f64 {
        match rel_type {
            "FAST" => 1.0,
            "SLOW" => 5.0,
            _ => 1.0,
        }
    }

    #[test]
    fn test_sssp_weighted() {
        let graph = create_weighted_graph();
        let source = ("ws".to_string(), "A".to_string());

        let dists = sssp_distances(&graph, &source, weight_fn);

        assert_eq!(dists[&("ws".to_string(), "A".to_string())], 0.0);
        assert_eq!(dists[&("ws".to_string(), "B".to_string())], 1.0);
        assert_eq!(dists[&("ws".to_string(), "C".to_string())], 5.0);
        assert_eq!(dists[&("ws".to_string(), "D".to_string())], 2.0); // A->B->D
    }

    #[test]
    fn test_sssp_unit_weights_matches_bfs() {
        let graph = create_weighted_graph();
        let source = ("ws".to_string(), "A".to_string());

        let dists = sssp_distances(&graph, &source, |_| 1.0);

        assert_eq!(dists[&("ws".to_string(), "A".to_string())], 0.0);
        assert_eq!(dists[&("ws".to_string(), "B".to_string())], 1.0);
        assert_eq!(dists[&("ws".to_string(), "C".to_string())], 1.0);
        assert_eq!(dists[&("ws".to_string(), "D".to_string())], 2.0);
    }

    #[test]
    fn test_node_sssp_distance() {
        let graph = create_weighted_graph();
        let source = ("ws".to_string(), "A".to_string());
        let target = ("ws".to_string(), "D".to_string());

        assert_eq!(
            node_sssp_distance(&graph, &source, &target, weight_fn),
            Some(2.0)
        );
    }

    #[test]
    fn test_sssp_unreachable() {
        let graph = create_weighted_graph();
        let source = ("ws".to_string(), "D".to_string());
        let target = ("ws".to_string(), "A".to_string());

        assert_eq!(
            node_sssp_distance(&graph, &source, &target, weight_fn),
            None
        );
    }

    #[test]
    fn test_sssp_same_node() {
        let graph = create_weighted_graph();
        let node = ("ws".to_string(), "A".to_string());

        assert_eq!(
            node_sssp_distance(&graph, &node, &node, weight_fn),
            Some(0.0)
        );
    }
}
