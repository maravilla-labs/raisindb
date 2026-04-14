// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use crate::projection::GraphProjection;
use rayon::prelude::*;
use std::collections::HashMap;

/// Community Detection using Label Propagation (CDLP) per the LDBC Graphalytics spec.
///
/// Synchronous (double-buffered) label propagation on an undirected view of
/// the graph. Each node is initialized with its own integer label. On each
/// iteration every node adopts the most frequent label among its neighbors
/// (ties broken by smallest label value). Runs exactly `max_iterations`
/// rounds with no early convergence check, as required by the LDBC spec.
///
/// The algorithm is fully deterministic: no randomness, deterministic
/// tie-breaking, and synchronous updates guarantee reproducible results.
///
/// Returns a map of NodeID -> community label (u32).
pub fn cdlp(projection: &GraphProjection, max_iterations: usize) -> HashMap<String, u32> {
    let node_count = projection.node_count();
    if node_count == 0 {
        return HashMap::new();
    }

    let graph = projection.graph();

    // Build undirected adjacency lists from the directed forward graph.
    // For each edge u->v in the forward CSR, both u and v are neighbors of each other.
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); node_count];
    for u in 0..node_count {
        if u < graph.node_count() {
            for &v in graph.neighbors_slice(u as u32) {
                let v_idx = v as usize;
                adj[u].push(v);
                if v_idx < node_count {
                    adj[v_idx].push(u as u32);
                }
            }
        }
    }

    // Sort adjacency lists for deterministic mode-finding.
    // Do NOT dedup: per LDBC spec, reciprocal edges (A→B and B→A) must both
    // contribute to the neighbor list so the label counts correctly in mode computation.
    adj.par_iter_mut().for_each(|neighbors| {
        neighbors.sort_unstable();
    });

    // Initialize: each node is its own community
    let mut labels: Vec<u32> = (0..node_count as u32).collect();
    let mut new_labels = vec![0u32; node_count];

    for _iter in 0..max_iterations {
        // Synchronous update: compute all new labels from current labels
        new_labels
            .par_iter_mut()
            .enumerate()
            .for_each(|(u, new_label)| {
                let neighbors = &adj[u];
                if neighbors.is_empty() {
                    // Isolated node keeps its own label
                    *new_label = labels[u];
                    return;
                }

                // Collect neighbor labels
                let mut neighbor_labels: Vec<u32> =
                    neighbors.iter().map(|&v| labels[v as usize]).collect();

                // Sort-based mode finding with smallest-label tie-breaking
                neighbor_labels.sort_unstable();

                let mut best_label = neighbor_labels[0];
                let mut best_count = 1u32;
                let mut current_label = neighbor_labels[0];
                let mut current_count = 1u32;

                for &nl in &neighbor_labels[1..] {
                    if nl == current_label {
                        current_count += 1;
                    } else {
                        if current_count > best_count
                            || (current_count == best_count && current_label < best_label)
                        {
                            best_label = current_label;
                            best_count = current_count;
                        }
                        current_label = nl;
                        current_count = 1;
                    }
                }
                // Check last run
                if current_count > best_count
                    || (current_count == best_count && current_label < best_label)
                {
                    best_label = current_label;
                }

                *new_label = best_label;
            });

        std::mem::swap(&mut labels, &mut new_labels);
    }

    // Map back to String IDs
    let mut result = HashMap::with_capacity(node_count);
    for (i, label) in labels.into_iter().enumerate() {
        if let Some(node_id) = projection.get_node_id(i as u32) {
            result.insert(node_id.clone(), label);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::GraphProjection;

    #[test]
    fn test_cdlp_empty_graph() {
        let projection = GraphProjection::from_parts(vec![], vec![]);
        let result = cdlp(&projection, 10);
        assert!(result.is_empty());
    }

    #[test]
    fn test_cdlp_single_node() {
        let nodes = vec!["A".to_string()];
        let projection = GraphProjection::from_parts(nodes, vec![]);
        let result = cdlp(&projection, 10);

        assert_eq!(result.len(), 1);
        assert_eq!(result["A"], 0);
    }

    #[test]
    fn test_cdlp_clique_k4() {
        // Fully connected K4: all nodes should converge to the same community
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
        ];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("A".to_string(), "C".to_string()),
            ("A".to_string(), "D".to_string()),
            ("B".to_string(), "C".to_string()),
            ("B".to_string(), "D".to_string()),
            ("C".to_string(), "D".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);
        let result = cdlp(&projection, 10);

        assert_eq!(result.len(), 4);
        // All should have the same label
        let label = result["A"];
        assert_eq!(result["B"], label);
        assert_eq!(result["C"], label);
        assert_eq!(result["D"], label);
    }

    #[test]
    fn test_cdlp_two_disconnected_cliques() {
        // Clique 1: A-B-C, Clique 2: D-E-F
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
            "E".to_string(),
            "F".to_string(),
        ];
        let edges = vec![
            // Clique 1
            ("A".to_string(), "B".to_string()),
            ("A".to_string(), "C".to_string()),
            ("B".to_string(), "C".to_string()),
            // Clique 2
            ("D".to_string(), "E".to_string()),
            ("D".to_string(), "F".to_string()),
            ("E".to_string(), "F".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);
        let result = cdlp(&projection, 10);

        assert_eq!(result.len(), 6);
        // Clique 1 should share a label
        assert_eq!(result["A"], result["B"]);
        assert_eq!(result["A"], result["C"]);
        // Clique 2 should share a label
        assert_eq!(result["D"], result["E"]);
        assert_eq!(result["D"], result["F"]);
        // The two cliques should have different labels
        assert_ne!(result["A"], result["D"]);
    }

    #[test]
    fn test_cdlp_two_cliques_with_bridge() {
        // Clique 1: A-B-C, Clique 2: D-E-F, bridge: C-D
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
            "E".to_string(),
            "F".to_string(),
        ];
        let edges = vec![
            // Clique 1
            ("A".to_string(), "B".to_string()),
            ("A".to_string(), "C".to_string()),
            ("B".to_string(), "C".to_string()),
            // Bridge
            ("C".to_string(), "D".to_string()),
            // Clique 2
            ("D".to_string(), "E".to_string()),
            ("D".to_string(), "F".to_string()),
            ("E".to_string(), "F".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);
        let result = cdlp(&projection, 10);

        assert_eq!(result.len(), 6);
        // With a bridge, typically the two cliques maintain separate communities
        // The interior nodes (A,B vs E,F) should share labels within their clique
        assert_eq!(result["A"], result["B"]);
        assert_eq!(result["E"], result["F"]);
    }

    #[test]
    fn test_cdlp_path_graph() {
        // A - B - C (path)
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);
        let result = cdlp(&projection, 10);

        assert_eq!(result.len(), 3);
        // All should have valid labels
        for (_, label) in &result {
            assert!(*label < 3);
        }
    }

    #[test]
    fn test_cdlp_deterministic() {
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
            "E".to_string(),
        ];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("A".to_string(), "C".to_string()),
            ("B".to_string(), "D".to_string()),
            ("C".to_string(), "D".to_string()),
            ("D".to_string(), "E".to_string()),
        ];

        let projection1 = GraphProjection::from_parts(nodes.clone(), edges.clone());
        let result1 = cdlp(&projection1, 10);

        let projection2 = GraphProjection::from_parts(nodes, edges);
        let result2 = cdlp(&projection2, 10);

        assert_eq!(result1, result2, "CDLP must be deterministic");
    }

    #[test]
    fn test_cdlp_isolated_nodes() {
        // Three isolated nodes with no edges
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let projection = GraphProjection::from_parts(nodes, vec![]);
        let result = cdlp(&projection, 10);

        assert_eq!(result.len(), 3);
        // Each isolated node keeps its own label
        assert_ne!(result["A"], result["B"]);
        assert_ne!(result["A"], result["C"]);
        assert_ne!(result["B"], result["C"]);
    }
}
