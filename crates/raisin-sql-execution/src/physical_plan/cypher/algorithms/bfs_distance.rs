//! BFS Distance Algorithm
//!
//! Computes shortest-hop distances from a source node to all reachable nodes
//! using Breadth-First Search on the directed `GraphAdjacency` HashMap.
//!
//! Time Complexity: O(V + E)
//! Space Complexity: O(V)

use std::collections::{HashMap, VecDeque};

use super::types::{GraphAdjacency, GraphNodeId};

/// Compute BFS distances from `source` to all nodes in the graph.
///
/// Returns a map of node -> hop distance. The source itself
/// is included with distance 0. Unreachable nodes have distance `usize::MAX`.
///
/// # Arguments
/// * `adjacency` - Directed graph adjacency list
/// * `source` - Starting node
///
/// # Returns
/// * HashMap of (node -> distance) for all nodes (unreachable ones get `usize::MAX`)
pub fn bfs_distances(
    adjacency: &GraphAdjacency,
    source: &GraphNodeId,
) -> HashMap<GraphNodeId, usize> {
    // Initialize all nodes to usize::MAX (unreachable)
    let mut distances: HashMap<GraphNodeId, usize> = HashMap::new();
    for key in adjacency.keys() {
        distances.insert(key.clone(), usize::MAX);
        if let Some(neighbors) = adjacency.get(key) {
            for (tgt_w, tgt_id, _rel_type) in neighbors {
                let next = (tgt_w.clone(), tgt_id.clone());
                distances.entry(next).or_insert(usize::MAX);
            }
        }
    }

    let mut queue = VecDeque::new();

    distances.insert(source.clone(), 0);
    queue.push_back(source.clone());

    while let Some(current) = queue.pop_front() {
        let current_dist = distances[&current];

        if let Some(neighbors) = adjacency.get(&current) {
            for (tgt_w, tgt_id, _rel_type) in neighbors {
                let next = (tgt_w.clone(), tgt_id.clone());
                if distances[&next] == usize::MAX {
                    distances.insert(next.clone(), current_dist + 1);
                    queue.push_back(next);
                }
            }
        }
    }

    distances
}

/// Compute the BFS (shortest-hop) distance between two specific nodes.
///
/// Returns `None` if `target` is not reachable from `source`.
///
/// # Arguments
/// * `adjacency` - Directed graph adjacency list
/// * `source` - Starting node
/// * `target` - Destination node
///
/// # Returns
/// * `Some(distance)` if target is reachable, `None` otherwise
pub fn node_bfs_distance(
    adjacency: &GraphAdjacency,
    source: &GraphNodeId,
    target: &GraphNodeId,
) -> Option<usize> {
    if source == target {
        return Some(0);
    }

    let mut visited: HashMap<GraphNodeId, usize> = HashMap::new();
    let mut queue = VecDeque::new();

    visited.insert(source.clone(), 0);
    queue.push_back(source.clone());

    while let Some(current) = queue.pop_front() {
        let current_dist = visited[&current];

        if let Some(neighbors) = adjacency.get(&current) {
            for (tgt_w, tgt_id, _rel_type) in neighbors {
                let next = (tgt_w.clone(), tgt_id.clone());
                if !visited.contains_key(&next) {
                    let next_dist = current_dist + 1;
                    if &next == target {
                        return Some(next_dist);
                    }
                    visited.insert(next.clone(), next_dist);
                    queue.push_back(next);
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_graph() -> GraphAdjacency {
        let mut graph = HashMap::new();

        // A -> B -> C -> D
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
                ("ws".to_string(), "C".to_string(), "SHORT".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![("ws".to_string(), "C".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![("ws".to_string(), "D".to_string(), "LINK".to_string())],
        );

        graph
    }

    #[test]
    fn test_bfs_distances_basic() {
        let graph = create_test_graph();
        let source = ("ws".to_string(), "A".to_string());

        let dists = bfs_distances(&graph, &source);

        assert_eq!(dists[&("ws".to_string(), "A".to_string())], 0);
        assert_eq!(dists[&("ws".to_string(), "B".to_string())], 1);
        assert_eq!(dists[&("ws".to_string(), "C".to_string())], 1); // A->C shortcut
        assert_eq!(dists[&("ws".to_string(), "D".to_string())], 2); // A->C->D
    }

    #[test]
    fn test_bfs_distances_all_reachable() {
        let graph = create_test_graph();
        let source = ("ws".to_string(), "A".to_string());

        let dists = bfs_distances(&graph, &source);
        assert_eq!(dists.len(), 4); // A, B, C, D all reachable
    }

    #[test]
    fn test_node_bfs_distance() {
        let graph = create_test_graph();
        let source = ("ws".to_string(), "A".to_string());
        let target = ("ws".to_string(), "D".to_string());

        assert_eq!(node_bfs_distance(&graph, &source, &target), Some(2));
    }

    #[test]
    fn test_node_bfs_distance_same_node() {
        let graph = create_test_graph();
        let node = ("ws".to_string(), "A".to_string());
        assert_eq!(node_bfs_distance(&graph, &node, &node), Some(0));
    }

    #[test]
    fn test_unreachable_node() {
        let graph = create_test_graph();
        let source = ("ws".to_string(), "D".to_string());
        let target = ("ws".to_string(), "A".to_string());

        // No reverse edges, so D cannot reach A
        assert_eq!(node_bfs_distance(&graph, &source, &target), None);
    }

    #[test]
    fn test_bfs_distances_from_leaf() {
        let graph = create_test_graph();
        let source = ("ws".to_string(), "D".to_string());

        let dists = bfs_distances(&graph, &source);
        // D has no outgoing edges, so only D itself is reachable; others get usize::MAX
        assert_eq!(dists.len(), 4);
        assert_eq!(dists[&("ws".to_string(), "D".to_string())], 0);
        assert_eq!(dists[&("ws".to_string(), "A".to_string())], usize::MAX);
        assert_eq!(dists[&("ws".to_string(), "B".to_string())], usize::MAX);
        assert_eq!(dists[&("ws".to_string(), "C".to_string())], usize::MAX);
    }
}
