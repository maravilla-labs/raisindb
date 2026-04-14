//! Community Detection Label Propagation (CDLP) - Synchronous Variant
//!
//! Implements synchronous (double-buffered) label propagation for community
//! detection on directed graphs. Unlike the asynchronous variant in
//! `label_propagation.rs`, this version updates all labels simultaneously
//! per iteration for deterministic results.
//!
//! Time Complexity: O(max_iterations * E)
//! Space Complexity: O(V)

use std::collections::HashMap;

use super::types::{GraphAdjacency, GraphNodeId};

/// Detect communities using synchronous label propagation.
///
/// All nodes read from the previous iteration's labels and write to a new
/// buffer (double-buffered / synchronous update). Ties are broken by choosing
/// the smallest label, making the output fully deterministic.
///
/// Runs exactly `max_iterations` iterations (no early convergence check)
/// to guarantee reproducible results.
///
/// # Arguments
/// * `adjacency` - Directed graph adjacency list (treated as undirected)
/// * `max_iterations` - Number of iterations to run
///
/// # Returns
/// * HashMap of (node -> community_id), normalized to 0..N-1
pub fn cdlp(adjacency: &GraphAdjacency, max_iterations: usize) -> HashMap<GraphNodeId, usize> {
    // Build undirected neighbor sets
    let undirected = build_undirected_neighbors(adjacency);

    if undirected.is_empty() {
        return HashMap::new();
    }

    // Initialize: each node gets a unique label based on sorted order
    let mut all_nodes: Vec<GraphNodeId> = undirected.keys().cloned().collect();
    all_nodes.sort();

    let mut labels: HashMap<GraphNodeId, usize> = all_nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.clone(), i))
        .collect();

    // Synchronous iteration: read from old labels, write to new buffer
    for _iter in 0..max_iterations {
        let mut new_labels: HashMap<GraphNodeId, usize> = HashMap::new();

        for node in &all_nodes {
            let neighbors = match undirected.get(node) {
                Some(n) => n,
                None => {
                    // Isolated node keeps its label
                    new_labels.insert(node.clone(), labels[node]);
                    continue;
                }
            };

            if neighbors.is_empty() {
                new_labels.insert(node.clone(), labels[node]);
                continue;
            }

            // Count neighbor labels from the *previous* iteration
            let mut label_counts: HashMap<usize, usize> = HashMap::new();
            for neighbor in neighbors {
                if let Some(&lbl) = labels.get(neighbor) {
                    *label_counts.entry(lbl).or_insert(0) += 1;
                }
            }

            if label_counts.is_empty() {
                new_labels.insert(node.clone(), labels[node]);
                continue;
            }

            // Find the mode; break ties by smallest label
            let max_count = *label_counts.values().max().unwrap();
            let best_label = label_counts
                .iter()
                .filter(|(_, &count)| count == max_count)
                .map(|(&lbl, _)| lbl)
                .min()
                .unwrap();

            new_labels.insert(node.clone(), best_label);
        }

        labels = new_labels;
    }

    // Normalize to consecutive 0..N-1
    normalize_community_ids(labels)
}

/// Get the community ID for a specific node using synchronous CDLP.
///
/// Returns `None` if the node is not present in the graph.
pub fn node_cdlp_community(adjacency: &GraphAdjacency, node: &GraphNodeId) -> Option<usize> {
    let communities = cdlp(adjacency, 10);
    communities.get(node).copied()
}

/// Build undirected neighbor lists from directed adjacency.
///
/// Per LDBC spec, reciprocal edges (A→B and B→A) must NOT be deduplicated:
/// both directions contribute to the neighbor list so that the label counts
/// correctly in mode computation.
fn build_undirected_neighbors(
    adjacency: &GraphAdjacency,
) -> HashMap<GraphNodeId, Vec<GraphNodeId>> {
    let mut undirected: HashMap<GraphNodeId, Vec<GraphNodeId>> = HashMap::new();

    for (source, neighbors) in adjacency.iter() {
        for (tgt_w, tgt_id, _rel_type) in neighbors {
            let target = (tgt_w.clone(), tgt_id.clone());
            if source != &target {
                undirected
                    .entry(source.clone())
                    .or_default()
                    .push(target.clone());
                undirected.entry(target).or_default().push(source.clone());
            }
        }
    }

    undirected
}

/// Normalize community IDs to consecutive integers starting from 0.
fn normalize_community_ids(labels: HashMap<GraphNodeId, usize>) -> HashMap<GraphNodeId, usize> {
    let mut unique_labels: Vec<usize> = labels.values().copied().collect();
    unique_labels.sort();
    unique_labels.dedup();

    let label_map: HashMap<usize, usize> = unique_labels
        .into_iter()
        .enumerate()
        .map(|(i, old)| (old, i))
        .collect();

    labels
        .into_iter()
        .map(|(node, old)| (node, label_map[&old]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_clique_single_community() {
        let mut graph: GraphAdjacency = HashMap::new();

        // K4 complete graph
        for i in 0..4 {
            let mut edges = vec![];
            for j in 0..4 {
                if i != j {
                    edges.push(("ws".to_string(), format!("n{}", j), "LINK".to_string()));
                }
            }
            graph.insert(("ws".to_string(), format!("n{}", i)), edges);
        }

        let communities = cdlp(&graph, 10);
        let unique: HashSet<usize> = communities.values().copied().collect();
        assert_eq!(unique.len(), 1, "K4 should form a single community");
    }

    #[test]
    fn test_two_disconnected_communities() {
        let mut graph: GraphAdjacency = HashMap::new();

        // Community 1: A <-> B <-> C <-> A
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

        // Community 2: X <-> Y <-> Z <-> X
        graph.insert(
            ("ws".to_string(), "X".to_string()),
            vec![
                ("ws".to_string(), "Y".to_string(), "LINK".to_string()),
                ("ws".to_string(), "Z".to_string(), "LINK".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "Y".to_string()),
            vec![
                ("ws".to_string(), "X".to_string(), "LINK".to_string()),
                ("ws".to_string(), "Z".to_string(), "LINK".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "Z".to_string()),
            vec![
                ("ws".to_string(), "X".to_string(), "LINK".to_string()),
                ("ws".to_string(), "Y".to_string(), "LINK".to_string()),
            ],
        );

        let communities = cdlp(&graph, 10);

        // Nodes in same community should share label
        assert_eq!(
            communities[&("ws".to_string(), "A".to_string())],
            communities[&("ws".to_string(), "B".to_string())]
        );
        assert_eq!(
            communities[&("ws".to_string(), "X".to_string())],
            communities[&("ws".to_string(), "Y".to_string())]
        );

        // Communities should differ
        assert_ne!(
            communities[&("ws".to_string(), "A".to_string())],
            communities[&("ws".to_string(), "X".to_string())]
        );
    }

    #[test]
    fn test_deterministic() {
        let mut graph: GraphAdjacency = HashMap::new();
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![("ws".to_string(), "A".to_string(), "LINK".to_string())],
        );

        let r1 = cdlp(&graph, 5);
        let r2 = cdlp(&graph, 5);
        assert_eq!(r1, r2, "synchronous CDLP must be deterministic");
    }

    #[test]
    fn test_node_cdlp_community() {
        let mut graph: GraphAdjacency = HashMap::new();
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );

        let comm = node_cdlp_community(&graph, &("ws".to_string(), "A".to_string()));
        assert!(comm.is_some());

        let missing = node_cdlp_community(&graph, &("ws".to_string(), "Z".to_string()));
        assert!(missing.is_none());
    }

    #[test]
    fn test_empty_graph() {
        let graph: GraphAdjacency = HashMap::new();
        let communities = cdlp(&graph, 10);
        assert!(communities.is_empty());
    }
}
