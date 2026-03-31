//! Label Propagation Community Detection Algorithm
//!
//! Implements the Label Propagation Algorithm (LPA) for community detection.
//! This algorithm detects communities by propagating labels through the network,
//! where nodes iteratively adopt the most frequent label among their neighbors.
//!
//! Time Complexity: O(k * E) where k = iterations
//! Space Complexity: O(V)
//!
//! Reference: Raghavan, Albert, and Kumara (2007)

use rand::seq::SliceRandom;
use rand::thread_rng;
use std::collections::{HashMap, HashSet};

use super::types::{GraphAdjacency, GraphNodeId};

/// Configuration for Label Propagation algorithm
pub struct LabelPropagationConfig {
    /// Maximum number of iterations
    pub max_iterations: usize,
    /// Whether to use random tie-breaking
    pub randomize_ties: bool,
}

impl Default for LabelPropagationConfig {
    fn default() -> Self {
        Self {
            max_iterations: 100,
            randomize_ties: true,
        }
    }
}

/// Detect communities using Label Propagation Algorithm
///
/// Returns a mapping of node -> community_id where nodes in the same
/// community share the same label.
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `config` - Algorithm configuration
///
/// # Returns
/// * HashMap of (node -> community_id) for all nodes
pub fn label_propagation(
    adjacency: &GraphAdjacency,
    config: &LabelPropagationConfig,
) -> HashMap<GraphNodeId, usize> {
    // Build undirected graph (treat edges as bidirectional for communities)
    let undirected = build_undirected_neighbors(adjacency);

    // Collect all unique nodes
    let mut all_nodes: Vec<_> = undirected.keys().cloned().collect();
    if all_nodes.is_empty() {
        return HashMap::new();
    }

    // Initialize: each node gets unique label
    let mut labels: HashMap<(String, String), usize> = all_nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.clone(), i))
        .collect();

    let mut rng = thread_rng();

    // Iterate until convergence or max iterations
    for iteration in 0..config.max_iterations {
        let mut changed = false;

        // Randomize node order for asynchronous updates
        if config.randomize_ties {
            all_nodes.shuffle(&mut rng);
        }

        for node in &all_nodes {
            // Count neighbor labels
            let mut label_counts: HashMap<usize, usize> = HashMap::new();

            if let Some(neighbors) = undirected.get(node) {
                for neighbor in neighbors {
                    if let Some(&neighbor_label) = labels.get(neighbor) {
                        *label_counts.entry(neighbor_label).or_insert(0) += 1;
                    }
                }
            }

            if label_counts.is_empty() {
                continue; // Isolated node keeps its label
            }

            // Find most frequent label(s)
            let max_count = *label_counts
                .values()
                .max()
                .expect("non-empty after is_empty guard");
            let mut max_labels: Vec<_> = label_counts
                .iter()
                .filter(|(_, &count)| count == max_count)
                .map(|(&label, _)| label)
                .collect();

            // Handle ties with randomization or deterministic choice
            let new_label = if config.randomize_ties && max_labels.len() > 1 {
                *max_labels
                    .choose(&mut rng)
                    .expect("non-empty after max_count filter")
            } else {
                max_labels.sort();
                max_labels[0]
            };

            // Update label if changed
            let old_label = labels[node];
            if new_label != old_label {
                labels.insert(node.clone(), new_label);
                changed = true;
            }
        }

        // Check convergence
        if !changed {
            tracing::debug!("Label propagation converged at iteration {}", iteration);
            break;
        }
    }

    // Normalize labels to 0..N-1 range
    normalize_community_ids(labels)
}

/// Build undirected neighbor list (ignoring edge types)
fn build_undirected_neighbors(
    adjacency: &GraphAdjacency,
) -> HashMap<GraphNodeId, Vec<GraphNodeId>> {
    let mut undirected: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();

    for (source, neighbors) in adjacency.iter() {
        for (tgt_workspace, tgt_id, _rel_type) in neighbors {
            let target = (tgt_workspace.clone(), tgt_id.clone());

            // Add forward edge
            undirected
                .entry(source.clone())
                .or_default()
                .push(target.clone());

            // Add reverse edge (for undirected)
            undirected
                .entry(target.clone())
                .or_default()
                .push(source.clone());
        }
    }

    undirected
}

/// Normalize community IDs to be consecutive integers starting from 0
fn normalize_community_ids(labels: HashMap<GraphNodeId, usize>) -> HashMap<GraphNodeId, usize> {
    // Map old label -> new label
    let mut unique_labels: Vec<_> = labels.values().copied().collect();
    unique_labels.sort();
    unique_labels.dedup();

    let label_map: HashMap<usize, usize> = unique_labels
        .into_iter()
        .enumerate()
        .map(|(i, old_label)| (old_label, i))
        .collect();

    // Apply mapping
    labels
        .into_iter()
        .map(|(node, old_label)| (node, label_map[&old_label]))
        .collect()
}

/// Get the community ID for a specific node
pub fn node_community_id(adjacency: &GraphAdjacency, node: &GraphNodeId) -> Option<usize> {
    let config = LabelPropagationConfig::default();
    let communities = label_propagation(adjacency, &config);
    communities.get(node).copied()
}

/// Get the number of communities detected
pub fn community_count(adjacency: &GraphAdjacency) -> usize {
    let config = LabelPropagationConfig::default();
    let communities = label_propagation(adjacency, &config);

    if communities.is_empty() {
        return 0;
    }

    let unique_communities: HashSet<_> = communities.values().copied().collect();
    unique_communities.len()
}

/// Get community sizes (number of nodes in each community)
pub fn community_sizes(adjacency: &GraphAdjacency) -> HashMap<usize, usize> {
    let config = LabelPropagationConfig::default();
    let communities = label_propagation(adjacency, &config);

    let mut sizes: HashMap<usize, usize> = HashMap::new();
    for community_id in communities.values() {
        *sizes.entry(*community_id).or_insert(0) += 1;
    }

    sizes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_two_community_graph() -> HashMap<(String, String), Vec<(String, String, String)>> {
        let mut graph = HashMap::new();

        // Community 1: Dense connections A <-> B <-> C
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
                ("ws".to_string(), "D".to_string(), "LINK".to_string()), // Bridge to community 2
            ],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![
                ("ws".to_string(), "A".to_string(), "LINK".to_string()),
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
            ],
        );

        // Community 2: Dense connections D <-> E <-> F
        graph.insert(
            ("ws".to_string(), "D".to_string()),
            vec![
                ("ws".to_string(), "B".to_string(), "LINK".to_string()), // Bridge from community 1
                ("ws".to_string(), "E".to_string(), "LINK".to_string()),
                ("ws".to_string(), "F".to_string(), "LINK".to_string()),
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
            vec![
                ("ws".to_string(), "D".to_string(), "LINK".to_string()),
                ("ws".to_string(), "E".to_string(), "LINK".to_string()),
            ],
        );

        graph
    }

    #[test]
    fn test_label_propagation_two_communities() {
        let graph = create_two_community_graph();
        let config = LabelPropagationConfig {
            max_iterations: 100,
            randomize_ties: false, // Deterministic for testing
        };

        let communities = label_propagation(&graph, &config);

        assert_eq!(communities.len(), 6);

        // Should detect 2 communities (though exact assignment may vary)
        let unique_labels: HashSet<_> = communities.values().copied().collect();
        assert!(unique_labels.len() <= 3); // At most 3 communities (ideally 2)
    }

    #[test]
    fn test_clique_graph() {
        let mut graph = HashMap::new();

        // Complete graph (clique): all nodes connected
        for i in 0..4 {
            let node = format!("node{}", i);
            let mut neighbors = vec![];
            for j in 0..4 {
                if i != j {
                    neighbors.push(("ws".to_string(), format!("node{}", j), "LINK".to_string()));
                }
            }
            graph.insert(("ws".to_string(), node), neighbors);
        }

        let count = community_count(&graph);
        // Clique should form single community
        assert_eq!(count, 1);
    }

    #[test]
    fn test_empty_graph() {
        let graph = HashMap::new();
        let communities = label_propagation(&graph, &LabelPropagationConfig::default());
        assert_eq!(communities.len(), 0);
        assert_eq!(community_count(&graph), 0);
    }

    #[test]
    fn test_node_community_id() {
        let graph = create_two_community_graph();
        let comm_a = node_community_id(&graph, &("ws".to_string(), "A".to_string()));
        assert!(comm_a.is_some());
    }

    #[test]
    fn test_community_sizes() {
        let graph = create_two_community_graph();
        let sizes = community_sizes(&graph);

        // Total nodes should equal 6
        let total_nodes: usize = sizes.values().sum();
        assert_eq!(total_nodes, 6);
    }

    #[test]
    fn test_convergence() {
        // Star graph should converge quickly
        let mut graph = HashMap::new();
        graph.insert(
            ("ws".to_string(), "center".to_string()),
            vec![
                ("ws".to_string(), "A".to_string(), "LINK".to_string()),
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
                ("ws".to_string(), "C".to_string(), "LINK".to_string()),
            ],
        );

        let config = LabelPropagationConfig {
            max_iterations: 100,
            randomize_ties: false,
        };

        let communities = label_propagation(&graph, &config);
        assert_eq!(communities.len(), 4);
    }

    #[test]
    fn test_normalize_community_ids() {
        let mut labels = HashMap::new();
        labels.insert(("ws".to_string(), "A".to_string()), 10);
        labels.insert(("ws".to_string(), "B".to_string()), 10);
        labels.insert(("ws".to_string(), "C".to_string()), 50);

        let normalized = normalize_community_ids(labels);

        // Should map to 0 and 1
        let unique: HashSet<_> = normalized.values().copied().collect();
        assert_eq!(unique.len(), 2);
        assert!(unique.contains(&0));
        assert!(unique.contains(&1));
    }
}
