//! Betweenness Centrality Algorithm
//!
//! Implements betweenness centrality using Brandes' algorithm.
//! Betweenness centrality measures how often a node appears on shortest paths
//! between other nodes, identifying "bridge" nodes that connect different parts
//! of the graph.
//!
//! Time Complexity: O(V * E) for unweighted graphs
//! Space Complexity: O(V + E)

use std::collections::{HashMap, HashSet, VecDeque};

use super::types::{GraphAdjacency, GraphNodeId};

/// Calculate betweenness centrality for a single node
///
/// Betweenness centrality measures how often a node lies on the shortest path
/// between two other nodes. Nodes with high betweenness are "bridges" that
/// connect different parts of the graph.
///
/// Formula: CB(v) = Σ(σst(v) / σst)
/// Where:
/// - σst = number of shortest paths from s to t
/// - σst(v) = number of those paths that pass through v
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `node` - The node to calculate betweenness for
///
/// # Returns
/// * Normalized betweenness centrality score (0.0 to 1.0)
pub fn betweenness_centrality(adjacency: &GraphAdjacency, node: &GraphNodeId) -> f64 {
    let all_scores = all_betweenness_centrality(adjacency);
    all_scores.get(node).copied().unwrap_or(0.0)
}

/// Calculate betweenness centrality for all nodes using Brandes' algorithm
///
/// This is more efficient than calculating betweenness for each node individually
/// as it performs a single BFS from each source node.
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
///
/// # Returns
/// * HashMap of (node -> betweenness_score) for all nodes
pub fn all_betweenness_centrality(adjacency: &GraphAdjacency) -> HashMap<GraphNodeId, f64> {
    // Collect all unique nodes
    let mut nodes = HashSet::new();
    for (source, neighbors) in adjacency.iter() {
        nodes.insert(source.clone());
        for (tgt_workspace, tgt_id, _) in neighbors {
            nodes.insert((tgt_workspace.clone(), tgt_id.clone()));
        }
    }

    let node_count = nodes.len();
    if node_count <= 2 {
        // Betweenness only meaningful for graphs with 3+ nodes
        return nodes.iter().map(|n| (n.clone(), 0.0)).collect();
    }

    // Initialize betweenness scores
    let mut betweenness: HashMap<(String, String), f64> =
        nodes.iter().map(|n| (n.clone(), 0.0)).collect();

    // Brandes' algorithm: BFS from each node
    for source in &nodes {
        // Shortest path data structures
        let mut stack = Vec::new(); // Nodes in order of discovery
        let mut paths: HashMap<(String, String), Vec<(String, String)>> = HashMap::new(); // Predecessors on shortest paths
        let mut sigma: HashMap<(String, String), f64> = HashMap::new(); // Number of shortest paths
        let mut dist: HashMap<(String, String), i32> = HashMap::new(); // Distance from source

        // Initialize
        for node in &nodes {
            paths.insert(node.clone(), Vec::new());
            sigma.insert(node.clone(), 0.0);
            dist.insert(node.clone(), -1);
        }
        sigma.insert(source.clone(), 1.0);
        dist.insert(source.clone(), 0);

        // BFS
        let mut queue = VecDeque::new();
        queue.push_back(source.clone());

        while let Some(current) = queue.pop_front() {
            stack.push(current.clone());
            let current_dist = dist[&current];

            if let Some(neighbors) = adjacency.get(&current) {
                for (next_workspace, next_id, _) in neighbors {
                    let next = (next_workspace.clone(), next_id.clone());

                    // First time seeing this node?
                    if dist[&next] < 0 {
                        queue.push_back(next.clone());
                        dist.insert(next.clone(), current_dist + 1);
                    }

                    // Shortest path to next via current?
                    if dist[&next] == current_dist + 1 {
                        sigma.insert(next.clone(), sigma[&next] + sigma[&current]);
                        paths.get_mut(&next).unwrap().push(current.clone());
                    }
                }
            }
        }

        // Accumulation: back-propagate betweenness scores
        let mut delta: HashMap<(String, String), f64> =
            nodes.iter().map(|n| (n.clone(), 0.0)).collect();

        while let Some(w) = stack.pop() {
            if let Some(predecessors) = paths.get(&w) {
                for v in predecessors {
                    let coefficient = (sigma[v] / sigma[&w]) * (1.0 + delta[&w]);
                    delta.insert(v.clone(), delta[v] + coefficient);
                }
            }
            if &w != source {
                betweenness.insert(w.clone(), betweenness[&w] + delta[&w]);
            }
        }
    }

    // Normalize: divide by (n-1)(n-2) for directed graphs
    let normalization = ((node_count - 1) * (node_count - 2)) as f64;
    if normalization > 0.0 {
        for score in betweenness.values_mut() {
            *score /= normalization;
        }
    }

    betweenness
}

/// Get nodes with highest betweenness centrality
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `limit` - Maximum number of nodes to return
///
/// # Returns
/// * Sorted list of (node, betweenness_score) pairs, descending by score
pub fn top_betweenness_nodes(adjacency: &GraphAdjacency, limit: usize) -> Vec<(GraphNodeId, f64)> {
    let mut scores: Vec<_> = all_betweenness_centrality(adjacency).into_iter().collect();
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores.into_iter().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_bridge_graph() -> HashMap<(String, String), Vec<(String, String, String)>> {
        let mut graph = HashMap::new();

        // Left cluster: A <-> B <-> C
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![
                ("ws".to_string(), "A".to_string(), "LINK".to_string()),
                ("ws".to_string(), "C".to_string(), "LINK".to_string()),
                ("ws".to_string(), "D".to_string(), "LINK".to_string()), // Bridge to right cluster
            ],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );

        // Right cluster: D <-> E <-> F
        graph.insert(
            ("ws".to_string(), "D".to_string()),
            vec![
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
                ("ws".to_string(), "E".to_string(), "LINK".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "E".to_string()),
            vec![
                ("ws".to_string(), "D".to_string(), "LINK".to_string()),
                ("ws".to_string(), "F".to_string(), "LINK".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "F".to_string()),
            vec![("ws".to_string(), "E".to_string(), "LINK".to_string())],
        );

        graph
    }

    #[test]
    fn test_betweenness_bridge_node() {
        let graph = create_bridge_graph();

        // B and D are bridge nodes connecting two clusters
        let betweenness_b = betweenness_centrality(&graph, &("ws".to_string(), "B".to_string()));
        let betweenness_d = betweenness_centrality(&graph, &("ws".to_string(), "D".to_string()));

        // Bridge nodes should have high betweenness
        assert!(betweenness_b > 0.3);
        assert!(betweenness_d > 0.3);

        // Peripheral nodes should have low betweenness
        let betweenness_a = betweenness_centrality(&graph, &("ws".to_string(), "A".to_string()));
        let betweenness_f = betweenness_centrality(&graph, &("ws".to_string(), "F".to_string()));
        assert!(betweenness_a < 0.1);
        assert!(betweenness_f < 0.1);
    }

    #[test]
    fn test_betweenness_star_graph() {
        let mut graph = HashMap::new();

        // Bidirectional star: A <-> B, A <-> C, A <-> D (A is center)
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
                ("ws".to_string(), "C".to_string(), "LINK".to_string()),
                ("ws".to_string(), "D".to_string(), "LINK".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![("ws".to_string(), "A".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![("ws".to_string(), "A".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "D".to_string()),
            vec![("ws".to_string(), "A".to_string(), "LINK".to_string())],
        );

        // In a bidirectional star graph, center node has all the betweenness
        let betweenness_a = betweenness_centrality(&graph, &("ws".to_string(), "A".to_string()));
        let betweenness_b = betweenness_centrality(&graph, &("ws".to_string(), "B".to_string()));

        // Center should have maximum betweenness (all paths go through it)
        assert!(betweenness_a > 0.9);
        // Leaf nodes have zero betweenness (no paths go through them)
        assert_eq!(betweenness_b, 0.0);
    }

    #[test]
    fn test_all_betweenness_centrality() {
        let graph = create_bridge_graph();
        let scores = all_betweenness_centrality(&graph);

        assert_eq!(scores.len(), 6);

        // All scores should be between 0 and 1
        for score in scores.values() {
            assert!(*score >= 0.0 && *score <= 1.0);
        }
    }

    #[test]
    fn test_top_betweenness_nodes() {
        let graph = create_bridge_graph();
        let top = top_betweenness_nodes(&graph, 2);

        assert_eq!(top.len(), 2);
        // Top nodes should be B and D (the bridges)
        let top_ids: HashSet<_> = top.iter().map(|(node, _)| &node.1).collect();
        assert!(top_ids.contains(&"B".to_string()) || top_ids.contains(&"D".to_string()));
    }

    #[test]
    fn test_betweenness_small_graph() {
        let mut graph = HashMap::new();
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );

        // Graph with only 2 nodes has zero betweenness
        let scores = all_betweenness_centrality(&graph);
        assert_eq!(scores[&("ws".to_string(), "A".to_string())], 0.0);
        assert_eq!(scores[&("ws".to_string(), "B".to_string())], 0.0);
    }

    #[test]
    fn test_betweenness_linear_graph() {
        let mut graph = HashMap::new();

        // Linear: A -> B -> C -> D
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![("ws".to_string(), "C".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![("ws".to_string(), "D".to_string(), "LINK".to_string())],
        );

        // In a linear graph, middle nodes have higher betweenness
        let betweenness_b = betweenness_centrality(&graph, &("ws".to_string(), "B".to_string()));
        let betweenness_c = betweenness_centrality(&graph, &("ws".to_string(), "C".to_string()));
        let betweenness_a = betweenness_centrality(&graph, &("ws".to_string(), "A".to_string()));

        // B and C should have higher betweenness than A or D
        assert!(betweenness_b > betweenness_a);
        assert!(betweenness_c > betweenness_a);
    }
}
