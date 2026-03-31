//! Louvain Community Detection Algorithm
//!
//! Implements the Louvain method for community detection.
//! This algorithm detects communities by optimizing modularity.
//!
//! Reference: Blondel, V. D., Guillaume, J. L., Lambiotte, R., & Lefebvre, E. (2008).
//! Fast unfolding of communities in large networks.

use std::collections::HashMap;

use super::types::{GraphAdjacency, GraphNodeId};

/// Configuration for Louvain algorithm
pub struct LouvainConfig {
    /// Maximum number of iterations
    pub max_iterations: usize,
    /// Resolution parameter (default 1.0)
    pub resolution: f64,
}

impl Default for LouvainConfig {
    fn default() -> Self {
        Self {
            max_iterations: 100,
            resolution: 1.0,
        }
    }
}

/// Detect communities using Louvain Algorithm
///
/// Returns a mapping of node -> community_id.
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `config` - Algorithm configuration
///
/// # Returns
/// * HashMap of (node -> community_id) for all nodes
pub fn louvain(adjacency: &GraphAdjacency, config: &LouvainConfig) -> HashMap<GraphNodeId, usize> {
    // 1. Build undirected graph with weights (assuming unweighted input = 1.0)
    // We need to sum weights if multiple edges exist between nodes.
    let mut graph: HashMap<(String, String), HashMap<(String, String), f64>> = HashMap::new();
    let mut node_degrees: HashMap<(String, String), f64> = HashMap::new();
    let mut m = 0.0; // Total weight of all edges

    for (source, neighbors) in adjacency.iter() {
        for (tgt_workspace, tgt_id, _rel_type) in neighbors {
            let target = (tgt_workspace.clone(), tgt_id.clone());

            // Add edge source -> target
            *graph
                .entry(source.clone())
                .or_default()
                .entry(target.clone())
                .or_default() += 1.0;
            // Add edge target -> source (undirected)
            *graph
                .entry(target.clone())
                .or_default()
                .entry(source.clone())
                .or_default() += 1.0;

            // Update degrees
            *node_degrees.entry(source.clone()).or_default() += 1.0;
            *node_degrees.entry(target.clone()).or_default() += 1.0;

            m += 1.0; // Count each edge once (we added it twice to graph map but m is usually sum of weights / 2 for undirected formula if we sum all degrees)
        }
    }

    // If we count each undirected edge as weight 1, then sum of degrees = 2 * m.
    // Here m is number of directed edges in adjacency.
    // If adjacency is directed, we treated it as undirected by adding reverse edges.
    // So total weight in `graph` is 2 * m (if no duplicates in adjacency).
    // Let's recalculate m from degrees to be safe.

    let total_weight: f64 = node_degrees.values().sum();
    m = total_weight / 2.0;

    if m == 0.0 {
        return HashMap::new();
    }

    let all_nodes: Vec<(String, String)> = node_degrees.keys().cloned().collect();
    let node_count = all_nodes.len();

    // Map nodes to indices for faster access
    let node_to_idx: HashMap<(String, String), usize> = all_nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.clone(), i))
        .collect();

    // 2. Initialize each node in its own community
    let mut community: Vec<usize> = (0..node_count).collect();

    // Community weights (sum of degrees of nodes in community)
    let mut tot: Vec<f64> = (0..node_count)
        .map(|i| node_degrees[&all_nodes[i]])
        .collect();

    // Node degrees by index
    let k: Vec<f64> = (0..node_count)
        .map(|i| node_degrees[&all_nodes[i]])
        .collect();

    let mut improved = true;
    let mut iter = 0;

    while improved && iter < config.max_iterations {
        improved = false;
        iter += 1;

        for i in 0..node_count {
            let node_u = &all_nodes[i];
            let current_comm = community[i];
            let k_u = k[i];

            // Remove i from its community
            tot[current_comm] -= k_u;

            // Find best neighbor community
            // k_u_c[c] = sum of weights from i to community c
            let mut k_u_c: HashMap<usize, f64> = HashMap::new();

            if let Some(neighbors) = graph.get(node_u) {
                for (neighbor, weight) in neighbors {
                    if let Some(&neighbor_idx) = node_to_idx.get(neighbor) {
                        let neighbor_comm = community[neighbor_idx];
                        *k_u_c.entry(neighbor_comm).or_default() += weight;
                    }
                }
            }

            let mut best_comm = current_comm;
            let mut max_gain = 0.0;

            // Check neighbors' communities and current
            let mut candidates: Vec<usize> = k_u_c.keys().cloned().collect();
            if !k_u_c.contains_key(&current_comm) {
                candidates.push(current_comm);
            }

            for target_comm in candidates {
                let k_c_in = *k_u_c.get(&target_comm).unwrap_or(&0.0);
                let tot_c = tot[target_comm];

                // Modularity gain
                let gain = k_c_in - (k_u * tot_c * config.resolution) / (2.0 * m);

                if gain > max_gain {
                    max_gain = gain;
                    best_comm = target_comm;
                }
            }

            // Apply move
            community[i] = best_comm;
            tot[best_comm] += k_u;

            if best_comm != current_comm {
                improved = true;
            }
        }
    }

    // Map results back to (String, String) -> CommunityID
    let mut result = HashMap::new();
    for i in 0..node_count {
        result.insert(all_nodes[i].clone(), community[i]);
    }

    // Normalize community IDs
    normalize_community_ids(result)
}

/// Normalize community IDs to be consecutive integers starting from 0
fn normalize_community_ids(labels: HashMap<GraphNodeId, usize>) -> HashMap<GraphNodeId, usize> {
    let mut unique_labels: Vec<_> = labels.values().copied().collect();
    unique_labels.sort();
    unique_labels.dedup();

    let label_map: HashMap<usize, usize> = unique_labels
        .into_iter()
        .enumerate()
        .map(|(i, old_label)| (old_label, i))
        .collect();

    labels
        .into_iter()
        .map(|(node, old_label)| (node, label_map[&old_label]))
        .collect()
}

/// Get the community ID for a specific node using Louvain
pub fn node_louvain_community_id(adjacency: &GraphAdjacency, node: &GraphNodeId) -> Option<usize> {
    let config = LouvainConfig::default();
    let communities = louvain(adjacency, &config);
    communities.get(node).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn create_two_community_graph() -> HashMap<(String, String), Vec<(String, String, String)>> {
        let mut graph = HashMap::new();
        // Same graph as in label_propagation tests
        // Community 1: A-B-C
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
                ("ws".to_string(), "D".to_string(), "LINK".to_string()), // Bridge
            ],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![
                ("ws".to_string(), "A".to_string(), "LINK".to_string()),
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
            ],
        );

        // Community 2: D-E-F
        graph.insert(
            ("ws".to_string(), "D".to_string()),
            vec![
                ("ws".to_string(), "B".to_string(), "LINK".to_string()), // Bridge
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
    fn test_louvain_two_communities() {
        let graph = create_two_community_graph();
        let config = LouvainConfig::default();
        let communities = louvain(&graph, &config);

        // Should detect 2 communities
        let unique_labels: HashSet<_> = communities.values().copied().collect();
        assert_eq!(unique_labels.len(), 2);

        // A, B, C should be in same community
        let comm_a = communities[&("ws".to_string(), "A".to_string())];
        let comm_b = communities[&("ws".to_string(), "B".to_string())];
        let comm_c = communities[&("ws".to_string(), "C".to_string())];
        assert_eq!(comm_a, comm_b);
        assert_eq!(comm_a, comm_c);

        // D, E, F should be in same community
        let comm_d = communities[&("ws".to_string(), "D".to_string())];
        let comm_e = communities[&("ws".to_string(), "E".to_string())];
        let comm_f = communities[&("ws".to_string(), "F".to_string())];
        assert_eq!(comm_d, comm_e);
        assert_eq!(comm_d, comm_f);

        // Communities should be different
        assert_ne!(comm_a, comm_d);
    }
}
