//! Triangle Count Algorithm
//!
//! Counts the number of triangles (cycles of length 3) each node participates in.
//! This is a key metric for clustering coefficient and community detection.
//!
//! Time Complexity: O(V * d^2) where d is average degree, or O(E^(1.5))
//! Space Complexity: O(V + E)

use std::collections::{HashMap, HashSet};

use super::types::{GraphAdjacency, GraphNodeId};

/// Calculate Triangle Count for all nodes in the graph
///
/// Returns a map of (workspace, id) -> count
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
///
/// # Returns
/// * HashMap of (node -> triangle_count)
pub fn triangle_count(adjacency: &GraphAdjacency) -> HashMap<GraphNodeId, usize> {
    // 1. Build undirected adjacency list (set of neighbors for fast lookup)
    let mut neighbors: HashMap<(String, String), HashSet<(String, String)>> = HashMap::new();

    for (source, targets) in adjacency.iter() {
        for (tgt_w, tgt_id, _) in targets {
            let target = (tgt_w.clone(), tgt_id.clone());
            if source != &target {
                neighbors
                    .entry(source.clone())
                    .or_default()
                    .insert(target.clone());
                neighbors.entry(target).or_default().insert(source.clone());
            }
        }
    }

    let mut counts = HashMap::new();

    // 2. For each node, count triangles
    // A triangle exists at u if two neighbors v and w are connected.
    for (u, u_neighbors) in &neighbors {
        let mut count = 0;

        // Convert to vector for indexed access to pairs
        let u_neighbors_vec: Vec<&(String, String)> = u_neighbors.iter().collect();

        // Check all pairs of neighbors
        for i in 0..u_neighbors_vec.len() {
            for j in (i + 1)..u_neighbors_vec.len() {
                let v = u_neighbors_vec[i];
                let w = u_neighbors_vec[j];

                // Check if v and w are connected
                if let Some(v_neighbors) = neighbors.get(v) {
                    if v_neighbors.contains(w) {
                        count += 1;
                    }
                }
            }
        }

        counts.insert(u.clone(), count);
    }

    // Ensure all nodes in adjacency are in result (even if count is 0)
    for node in adjacency.keys() {
        counts.entry(node.clone()).or_insert(0);
    }

    counts
}

/// Get triangle count for a specific node
pub fn node_triangle_count(adjacency: &GraphAdjacency, node: &GraphNodeId) -> usize {
    // This is inefficient if we only want one node, but reuses the logic.
    // Optimization: Only check neighbors of `node`.

    // 1. Build undirected neighbors for `node` and its neighbors
    // We need 2-hop neighborhood to check connections between neighbors.

    // Let's just use the global function for now or implement a local version.
    // Local version:

    let mut neighbors_set = HashSet::new();
    // Outgoing
    if let Some(targets) = adjacency.get(node) {
        for (w, id, _) in targets {
            neighbors_set.insert((w.clone(), id.clone()));
        }
    }
    // Incoming (scan all adjacency? expensive. Better to assume undirected graph is built or passed)
    // Since we only have directed adjacency map, we have to scan it to find incoming edges to `node`.
    for (src, targets) in adjacency {
        for (w, id, _) in targets {
            if w == &node.0 && id == &node.1 {
                neighbors_set.insert(src.clone());
            }
        }
    }

    let neighbors_vec: Vec<_> = neighbors_set.into_iter().collect();
    let mut count = 0;

    for i in 0..neighbors_vec.len() {
        for j in (i + 1)..neighbors_vec.len() {
            let v = &neighbors_vec[i];
            let w = &neighbors_vec[j];

            // Check if v and w are connected (in either direction)
            let mut connected = false;

            // Check v -> w
            if let Some(targets) = adjacency.get(v) {
                if targets.iter().any(|(tw, tid, _)| tw == &w.0 && tid == &w.1) {
                    connected = true;
                }
            }

            // Check w -> v
            if !connected {
                if let Some(targets) = adjacency.get(w) {
                    if targets.iter().any(|(tw, tid, _)| tw == &v.0 && tid == &v.1) {
                        connected = true;
                    }
                }
            }

            if connected {
                count += 1;
            }
        }
    }

    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_triangle_graph() -> HashMap<(String, String), Vec<(String, String, String)>> {
        let mut graph = HashMap::new();
        // A-B-C-A triangle
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
                ("ws".to_string(), "C".to_string(), "LINK".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![
                ("ws".to_string(), "C".to_string(), "LINK".to_string()),
                ("ws".to_string(), "A".to_string(), "LINK".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![
                ("ws".to_string(), "A".to_string(), "LINK".to_string()),
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
            ],
        );
        graph
    }

    #[test]
    fn test_triangle_count() {
        let graph = create_triangle_graph();
        let counts = triangle_count(&graph);

        assert_eq!(counts.get(&("ws".to_string(), "A".to_string())), Some(&1));
        assert_eq!(counts.get(&("ws".to_string(), "B".to_string())), Some(&1));
        assert_eq!(counts.get(&("ws".to_string(), "C".to_string())), Some(&1));
    }
}
