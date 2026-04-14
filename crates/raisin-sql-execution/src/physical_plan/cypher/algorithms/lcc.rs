//! Local Clustering Coefficient (LCC)
//!
//! Computes the local clustering coefficient for each node in the graph.
//! The LCC of a node measures the fraction of pairs of its neighbors that
//! are themselves connected, i.e., how close its neighborhood is to a clique.
//!
//! Time Complexity: O(V * d^2) where d is the average degree
//! Space Complexity: O(V + E)

use std::collections::{HashMap, HashSet};

use super::types::{GraphAdjacency, GraphNodeId};

/// Compute the local clustering coefficient for every node.
///
/// The graph is treated as undirected. For a node with degree `d`:
/// - If `d < 2`, the coefficient is 0.0 (no triangle possible).
/// - Otherwise, LCC = 2 * triangles / (d * (d - 1)).
///
/// # Arguments
/// * `adjacency` - Directed graph adjacency list (treated as undirected)
///
/// # Returns
/// * HashMap of (node -> clustering_coefficient) for all nodes
pub fn lcc(adjacency: &GraphAdjacency) -> HashMap<GraphNodeId, f64> {
    let undirected = build_undirected_neighbor_sets(adjacency);

    let mut result: HashMap<GraphNodeId, f64> = HashMap::new();

    for (node, neighbors) in &undirected {
        let deg = neighbors.len();
        if deg < 2 {
            result.insert(node.clone(), 0.0);
            continue;
        }

        // Count edges among neighbors
        let neighbors_vec: Vec<&GraphNodeId> = neighbors.iter().collect();
        let mut triangle_edges = 0usize;

        for i in 0..neighbors_vec.len() {
            for j in (i + 1)..neighbors_vec.len() {
                if let Some(ni_neighbors) = undirected.get(neighbors_vec[i]) {
                    if ni_neighbors.contains(neighbors_vec[j]) {
                        triangle_edges += 1;
                    }
                }
            }
        }

        let max_edges = deg * (deg - 1) / 2;
        let coefficient = triangle_edges as f64 / max_edges as f64;
        result.insert(node.clone(), coefficient);
    }

    // Ensure all adjacency keys are in the result even if they have no edges
    for node in adjacency.keys() {
        result.entry(node.clone()).or_insert(0.0);
    }

    result
}

/// Compute the local clustering coefficient for a single node.
///
/// Returns `None` if the node does not appear in the graph.
///
/// # Arguments
/// * `adjacency` - Directed graph adjacency list (treated as undirected)
/// * `node` - The node to compute the coefficient for
///
/// # Returns
/// * `Some(coefficient)` if the node exists, `None` otherwise
pub fn node_lcc(adjacency: &GraphAdjacency, node: &GraphNodeId) -> Option<f64> {
    let coefficients = lcc(adjacency);
    coefficients.get(node).copied()
}

/// Build undirected neighbor sets from directed adjacency.
fn build_undirected_neighbor_sets(
    adjacency: &GraphAdjacency,
) -> HashMap<GraphNodeId, HashSet<GraphNodeId>> {
    let mut undirected: HashMap<GraphNodeId, HashSet<GraphNodeId>> = HashMap::new();

    for (source, neighbors) in adjacency.iter() {
        for (tgt_w, tgt_id, _rel_type) in neighbors {
            let target = (tgt_w.clone(), tgt_id.clone());
            if source != &target {
                undirected
                    .entry(source.clone())
                    .or_default()
                    .insert(target.clone());
                undirected.entry(target).or_default().insert(source.clone());
            }
        }
    }

    undirected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_triangle_full_clustering() {
        // Triangle: A-B-C-A, every neighbor pair is connected => LCC = 1.0
        let mut graph: GraphAdjacency = HashMap::new();
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
                ("ws".to_string(), "A".to_string(), "LINK".to_string()),
                ("ws".to_string(), "C".to_string(), "LINK".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![
                ("ws".to_string(), "A".to_string(), "LINK".to_string()),
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
            ],
        );

        let coefficients = lcc(&graph);
        for (_, &c) in &coefficients {
            assert!(
                (c - 1.0).abs() < 1e-9,
                "triangle nodes should have LCC = 1.0"
            );
        }
    }

    #[test]
    fn test_star_zero_clustering() {
        // Star: center -> A, B, C but A, B, C not connected to each other
        let mut graph: GraphAdjacency = HashMap::new();
        graph.insert(
            ("ws".to_string(), "center".to_string()),
            vec![
                ("ws".to_string(), "A".to_string(), "LINK".to_string()),
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
                ("ws".to_string(), "C".to_string(), "LINK".to_string()),
            ],
        );

        let coefficients = lcc(&graph);
        let center_lcc = coefficients[&("ws".to_string(), "center".to_string())];
        assert!(
            center_lcc.abs() < 1e-9,
            "star center should have LCC = 0.0, got {}",
            center_lcc
        );

        // Leaves have degree 1 => LCC = 0.0
        let a_lcc = coefficients[&("ws".to_string(), "A".to_string())];
        assert!(a_lcc.abs() < 1e-9, "leaf should have LCC = 0.0");
    }

    #[test]
    fn test_path_zero_clustering() {
        // Path: A -> B -> C  (no triangles)
        let mut graph: GraphAdjacency = HashMap::new();
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![("ws".to_string(), "C".to_string(), "LINK".to_string())],
        );

        let coefficients = lcc(&graph);
        for (_, &c) in &coefficients {
            assert!(c.abs() < 1e-9, "path nodes should have LCC = 0.0");
        }
    }

    #[test]
    fn test_k4_full_clustering() {
        // K4: complete graph of 4 nodes, every node has LCC = 1.0
        let mut graph: GraphAdjacency = HashMap::new();
        for i in 0..4 {
            let mut edges = vec![];
            for j in 0..4 {
                if i != j {
                    edges.push(("ws".to_string(), format!("n{}", j), "LINK".to_string()));
                }
            }
            graph.insert(("ws".to_string(), format!("n{}", i)), edges);
        }

        let coefficients = lcc(&graph);
        for (_, &c) in &coefficients {
            assert!(
                (c - 1.0).abs() < 1e-9,
                "K4 nodes should have LCC = 1.0, got {}",
                c
            );
        }
    }

    #[test]
    fn test_node_lcc_specific() {
        let mut graph: GraphAdjacency = HashMap::new();
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );

        let result = node_lcc(&graph, &("ws".to_string(), "A".to_string()));
        assert!(result.is_some());
        assert!(result.unwrap().abs() < 1e-9);

        let missing = node_lcc(&graph, &("ws".to_string(), "Z".to_string()));
        assert!(missing.is_none());
    }
}
