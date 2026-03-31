//! Centrality Algorithms
//!
//! Implements various centrality measures for graph analysis:
//! - degree() - Total number of relationships
//! - inDegree() - Number of incoming relationships
//! - outDegree() - Number of outgoing relationships
//! - closeness() - Closeness centrality (average distance to all other nodes)
//!
//! These are the simplest and most commonly used centrality measures.

use std::collections::{HashMap, HashSet, VecDeque};

use super::types::{GraphAdjacency, GraphNodeId};

/// Calculate the total degree (in + out) of a node
///
/// # Arguments
/// * `adjacency` - Graph adjacency list: (workspace, id) -> [(target_workspace, target_id, rel_type)]
/// * `node` - The node to calculate degree for (workspace, id)
///
/// # Returns
/// * Total number of relationships (incoming + outgoing)
pub fn degree(adjacency: &GraphAdjacency, node: &GraphNodeId) -> usize {
    out_degree(adjacency, node) + in_degree(adjacency, node)
}

/// Calculate the out-degree (outgoing relationships) of a node
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `node` - The node to calculate out-degree for
///
/// # Returns
/// * Number of outgoing relationships
pub fn out_degree(adjacency: &GraphAdjacency, node: &GraphNodeId) -> usize {
    adjacency
        .get(node)
        .map(|neighbors| neighbors.len())
        .unwrap_or(0)
}

/// Calculate the in-degree (incoming relationships) of a node
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `node` - The node to calculate in-degree for
///
/// # Returns
/// * Number of incoming relationships
pub fn in_degree(adjacency: &GraphAdjacency, node: &GraphNodeId) -> usize {
    let mut count = 0;
    for ((_src_workspace, _src_id), neighbors) in adjacency.iter() {
        for (tgt_workspace, tgt_id, _rel_type) in neighbors {
            if (tgt_workspace, tgt_id) == (&node.0, &node.1) {
                count += 1;
            }
        }
    }
    count
}

/// Calculate degree centrality for all nodes in the graph
///
/// Returns a sorted list of (node, degree) pairs
pub fn all_degrees(adjacency: &GraphAdjacency) -> Vec<(GraphNodeId, usize)> {
    // Collect all unique nodes
    let mut nodes = std::collections::HashSet::new();
    for (source, neighbors) in adjacency.iter() {
        nodes.insert(source.clone());
        for (tgt_workspace, tgt_id, _) in neighbors {
            nodes.insert((tgt_workspace.clone(), tgt_id.clone()));
        }
    }

    // Calculate degree for each node
    let mut degrees: Vec<_> = nodes
        .iter()
        .map(|node| (node.clone(), degree(adjacency, node)))
        .collect();

    // Sort by degree descending
    degrees.sort_by(|a, b| b.1.cmp(&a.1));

    degrees
}

/// Calculate closeness centrality for a node
///
/// Closeness centrality measures how close a node is to all other nodes.
/// It's the inverse of the average distance to all other reachable nodes.
///
/// Formula: C(v) = (N-1) / Σ d(v, u)
/// Where:
/// - N = number of reachable nodes (including v)
/// - d(v, u) = shortest distance from v to u
///
/// For disconnected graphs, only reachable nodes are considered.
/// Returns 0.0 for isolated nodes (no reachable nodes).
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `node` - The node to calculate closeness for
///
/// # Returns
/// * Normalized closeness centrality score (0.0 to 1.0)
pub fn closeness_centrality(adjacency: &GraphAdjacency, node: &GraphNodeId) -> f64 {
    // Use BFS to calculate distances to all reachable nodes
    let mut queue = VecDeque::new();
    let mut distances: HashMap<(String, String), usize> = HashMap::new();

    queue.push_back((node.clone(), 0));
    distances.insert(node.clone(), 0);

    while let Some((current, dist)) = queue.pop_front() {
        if let Some(neighbors) = adjacency.get(&current) {
            for (next_workspace, next_id, _rel_type) in neighbors {
                let next = (next_workspace.clone(), next_id.clone());
                if !distances.contains_key(&next) {
                    distances.insert(next.clone(), dist + 1);
                    queue.push_back((next, dist + 1));
                }
            }
        }
    }

    // Calculate closeness
    let num_reachable = distances.len();

    if num_reachable <= 1 {
        // Isolated node or only self-reachable
        return 0.0;
    }

    let total_distance: usize = distances.values().sum();

    if total_distance == 0 {
        return 0.0;
    }

    // Normalized closeness: (N-1) / sum of distances
    (num_reachable - 1) as f64 / total_distance as f64
}

/// Calculate closeness centrality for all nodes in the graph
///
/// Returns a sorted list of (node, closeness) pairs, sorted by closeness descending.
pub fn all_closeness_centrality(adjacency: &GraphAdjacency) -> Vec<(GraphNodeId, f64)> {
    // Collect all unique nodes
    let mut nodes = HashSet::new();
    for (source, neighbors) in adjacency.iter() {
        nodes.insert(source.clone());
        for (tgt_workspace, tgt_id, _) in neighbors {
            nodes.insert((tgt_workspace.clone(), tgt_id.clone()));
        }
    }

    // Calculate closeness for each node
    let mut closeness_scores: Vec<_> = nodes
        .iter()
        .map(|node| (node.clone(), closeness_centrality(adjacency, node)))
        .collect();

    // Sort by closeness descending
    closeness_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    closeness_scores
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_graph() -> HashMap<(String, String), Vec<(String, String, String)>> {
        let mut graph = HashMap::new();

        // A has 2 outgoing: A -> B, A -> C
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
                ("ws".to_string(), "C".to_string(), "LINK".to_string()),
            ],
        );

        // B has 1 outgoing: B -> C
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![("ws".to_string(), "C".to_string(), "LINK".to_string())],
        );

        // C has 1 outgoing: C -> D
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![("ws".to_string(), "D".to_string(), "LINK".to_string())],
        );

        graph
    }

    #[test]
    fn test_out_degree() {
        let graph = create_test_graph();

        assert_eq!(out_degree(&graph, &("ws".to_string(), "A".to_string())), 2);
        assert_eq!(out_degree(&graph, &("ws".to_string(), "B".to_string())), 1);
        assert_eq!(out_degree(&graph, &("ws".to_string(), "C".to_string())), 1);
        assert_eq!(out_degree(&graph, &("ws".to_string(), "D".to_string())), 0);
    }

    #[test]
    fn test_in_degree() {
        let graph = create_test_graph();

        assert_eq!(in_degree(&graph, &("ws".to_string(), "A".to_string())), 0);
        assert_eq!(in_degree(&graph, &("ws".to_string(), "B".to_string())), 1);
        assert_eq!(in_degree(&graph, &("ws".to_string(), "C".to_string())), 2); // From A and B
        assert_eq!(in_degree(&graph, &("ws".to_string(), "D".to_string())), 1);
    }

    #[test]
    fn test_total_degree() {
        let graph = create_test_graph();

        assert_eq!(degree(&graph, &("ws".to_string(), "A".to_string())), 2); // 2 out, 0 in
        assert_eq!(degree(&graph, &("ws".to_string(), "B".to_string())), 2); // 1 out, 1 in
        assert_eq!(degree(&graph, &("ws".to_string(), "C".to_string())), 3); // 1 out, 2 in
        assert_eq!(degree(&graph, &("ws".to_string(), "D".to_string())), 1); // 0 out, 1 in
    }

    #[test]
    fn test_all_degrees() {
        let graph = create_test_graph();
        let degrees = all_degrees(&graph);

        assert_eq!(degrees.len(), 4);
        // C should have highest degree (3)
        assert_eq!(degrees[0].0, ("ws".to_string(), "C".to_string()));
        assert_eq!(degrees[0].1, 3);
    }

    #[test]
    fn test_closeness_linear_graph() {
        // Test graph: A -> B, A -> C, B -> C, C -> D
        let graph = create_test_graph();

        // A can reach A(0), B(1), C(1), D(2) = 4 total distance, N=4
        let closeness_a = closeness_centrality(&graph, &("ws".to_string(), "A".to_string()));
        assert!((closeness_a - 0.75).abs() < 0.01); // (4-1)/4 = 0.75

        // B can reach B(0), C(1), D(2) = 3 total distance, N=3
        let closeness_b = closeness_centrality(&graph, &("ws".to_string(), "B".to_string()));
        assert!((closeness_b - 0.666).abs() < 0.01); // (3-1)/3 = 0.666

        // D has no outgoing edges, only self-reachable
        let closeness_d = closeness_centrality(&graph, &("ws".to_string(), "D".to_string()));
        assert_eq!(closeness_d, 0.0);
    }

    #[test]
    fn test_closeness_star_graph() {
        let mut graph = HashMap::new();

        // Star: A -> B, A -> C, A -> D (A is hub)
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
                ("ws".to_string(), "C".to_string(), "LINK".to_string()),
                ("ws".to_string(), "D".to_string(), "LINK".to_string()),
            ],
        );

        // A can reach all nodes in 1 hop: (4-1)/(0+1+1+1) = 3/3 = 1.0
        let closeness_a = closeness_centrality(&graph, &("ws".to_string(), "A".to_string()));
        assert!((closeness_a - 1.0).abs() < 0.01);

        // B, C, D are isolated (no outgoing edges)
        let closeness_b = closeness_centrality(&graph, &("ws".to_string(), "B".to_string()));
        assert_eq!(closeness_b, 0.0);
    }

    #[test]
    fn test_all_closeness_centrality() {
        let graph = create_test_graph();
        let closeness = all_closeness_centrality(&graph);

        assert_eq!(closeness.len(), 4);
        // C should have highest closeness: reaches C(0), D(1) = (2-1)/1 = 1.0
        // Even though A reaches more nodes, C has lower average distance
        assert_eq!(closeness[0].0, ("ws".to_string(), "C".to_string()));
        assert!((closeness[0].1 - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_closeness_isolated_node() {
        let graph = HashMap::new();
        let closeness = closeness_centrality(&graph, &("ws".to_string(), "Isolated".to_string()));
        assert_eq!(closeness, 0.0);
    }
}
