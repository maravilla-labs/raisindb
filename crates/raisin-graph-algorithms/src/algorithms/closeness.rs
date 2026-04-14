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

//! Closeness Centrality on CSR (Wasserman-Faust formula).
//!
//! Closeness centrality measures how close a node is to all other reachable nodes.
//! Nodes with high closeness can reach other nodes via shorter paths on average.
//!
//! Uses the Wasserman-Faust formula which handles disconnected graphs:
//!
//! ```text
//! closeness(v) = ((n-1) / (N-1)) * ((n-1) / sum_of_distances)
//! ```
//!
//! Where:
//! - n = number of nodes reachable from v (including v itself)
//! - N = total number of nodes in the graph
//! - sum_of_distances = sum of shortest path distances to all reachable nodes
//!
//! For connected graphs, n == N so the formula reduces to the classic form.
//! For disconnected graphs, nodes in small components are penalized by (n-1)/(N-1).
//! Isolated nodes (no reachable neighbors) get closeness 0.0.
//!
//! Time Complexity: O(V * (V + E))
//! Space Complexity: O(V)

use crate::projection::GraphProjection;
use rayon::prelude::*;
use std::collections::{HashMap, VecDeque};

/// Compute closeness centrality for all nodes using the Wasserman-Faust formula.
///
/// For each node, runs a BFS to find distances to all reachable nodes.
/// Uses Wasserman-Faust: `((n-1)/(N-1)) * ((n-1)/sum_distances)` where
/// n = reachable count and N = total nodes. This penalizes nodes in small
/// components of disconnected graphs while preserving classic behavior
/// for connected graphs.
/// Returns 0.0 for isolated nodes (no reachable neighbors).
///
/// BFS passes are parallelized with rayon since each is independent.
///
/// Returns a map of NodeID (string) -> closeness score.
pub fn closeness_centrality(projection: &GraphProjection) -> HashMap<String, f64> {
    let graph = projection.graph();
    let node_count = projection.node_count();

    if node_count == 0 {
        return HashMap::new();
    }

    // Compute closeness for each node in parallel
    let scores: Vec<f64> = (0..node_count)
        .into_par_iter()
        .map(|s| {
            let mut dist: Vec<i64> = vec![-1; node_count];
            dist[s] = 0;

            let mut queue = VecDeque::new();
            queue.push_back(s as u32);

            let mut sum_distances: i64 = 0;
            let mut reachable_count: usize = 0;

            // BFS from node s
            while let Some(v) = queue.pop_front() {
                let v_dist = dist[v as usize];

                if (v as usize) < graph.node_count() {
                    for &w in graph.neighbors_slice(v) {
                        let w_usize = w as usize;
                        if dist[w_usize] < 0 {
                            dist[w_usize] = v_dist + 1;
                            sum_distances += dist[w_usize];
                            reachable_count += 1;
                            queue.push_back(w);
                        }
                    }
                }
            }

            // Wasserman-Faust formula for disconnected graphs:
            // ((n-1)/(N-1)) * ((n-1)/sum_distances)
            // where n = reachable_count + 1 (including self), N = total nodes
            let total_nodes = node_count;
            let n = reachable_count + 1; // reachable_count excludes self
            if total_nodes <= 1 || n <= 1 || sum_distances == 0 {
                0.0
            } else {
                let n_minus_1 = (n - 1) as f64;
                let big_n_minus_1 = (total_nodes - 1) as f64;
                (n_minus_1 / big_n_minus_1) * (n_minus_1 / sum_distances as f64)
            }
        })
        .collect();

    // Map back to string IDs
    let mut result = HashMap::with_capacity(node_count);
    for (i, score) in scores.into_iter().enumerate() {
        if let Some(node_id) = projection.get_node_id(i as u32) {
            result.insert(node_id.clone(), score);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::GraphProjection;

    #[test]
    fn test_closeness_empty_graph() {
        let projection = GraphProjection::from_parts(vec![], vec![]);
        let scores = closeness_centrality(&projection);
        assert!(scores.is_empty(), "Empty graph should return empty result");
    }

    #[test]
    fn test_closeness_single_node() {
        let nodes = vec!["A".to_string()];
        let edges: Vec<(String, String)> = vec![];
        let projection = GraphProjection::from_parts(nodes, edges);

        let scores = closeness_centrality(&projection);

        assert_eq!(scores.len(), 1);
        assert_eq!(scores["A"], 0.0, "Isolated node should have closeness 0.0");
    }

    #[test]
    fn test_closeness_triangle() {
        // Directed cycle: A -> B -> C -> A
        // From A: B=1, C=2 -> closeness = 2/3
        // From B: C=1, A=2 -> closeness = 2/3
        // From C: A=1, B=2 -> closeness = 2/3
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
            ("C".to_string(), "A".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let scores = closeness_centrality(&projection);

        assert_eq!(scores.len(), 3);
        // All nodes should have the same closeness in a directed cycle
        let expected = 2.0 / 3.0;
        assert!(
            (scores["A"] - expected).abs() < 1e-10,
            "A closeness should be {}, got {}",
            expected,
            scores["A"]
        );
        assert!(
            (scores["B"] - expected).abs() < 1e-10,
            "B closeness should be {}, got {}",
            expected,
            scores["B"]
        );
        assert!(
            (scores["C"] - expected).abs() < 1e-10,
            "C closeness should be {}, got {}",
            expected,
            scores["C"]
        );
    }

    #[test]
    fn test_closeness_star_graph() {
        // Bidirectional star: Center <-> S1, Center <-> S2, Center <-> S3
        let nodes = vec![
            "Center".to_string(),
            "S1".to_string(),
            "S2".to_string(),
            "S3".to_string(),
        ];
        let edges = vec![
            ("Center".to_string(), "S1".to_string()),
            ("Center".to_string(), "S2".to_string()),
            ("Center".to_string(), "S3".to_string()),
            ("S1".to_string(), "Center".to_string()),
            ("S2".to_string(), "Center".to_string()),
            ("S3".to_string(), "Center".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let scores = closeness_centrality(&projection);

        assert_eq!(scores.len(), 4);
        // Center reaches all 3 spokes at distance 1 -> closeness = 3/3 = 1.0
        assert!(
            (scores["Center"] - 1.0).abs() < 1e-10,
            "Center closeness should be 1.0, got {}",
            scores["Center"]
        );
        // Each spoke reaches Center at 1, other 2 spokes at 2 -> closeness = 3/5 = 0.6
        let expected_spoke = 3.0 / 5.0;
        assert!(
            (scores["S1"] - expected_spoke).abs() < 1e-10,
            "S1 closeness should be {}, got {}",
            expected_spoke,
            scores["S1"]
        );
        // Center should have highest closeness
        assert!(
            scores["Center"] > scores["S1"],
            "Center ({}) should have higher closeness than S1 ({})",
            scores["Center"],
            scores["S1"]
        );
    }

    #[test]
    fn test_closeness_disconnected() {
        // A -> B, C isolated
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![("A".to_string(), "B".to_string())];
        let projection = GraphProjection::from_parts(nodes, edges);

        let scores = closeness_centrality(&projection);

        assert_eq!(scores.len(), 3);
        // A reaches B at distance 1 -> n=2, N=3
        // Wasserman-Faust: (1/2) * (1/1) = 0.5
        let expected_a = 0.5;
        assert!(
            (scores["A"] - expected_a).abs() < 1e-10,
            "A closeness should be {}, got {}",
            expected_a,
            scores["A"]
        );
        // B has no outgoing edges -> closeness = 0.0
        assert_eq!(
            scores["B"], 0.0,
            "B (no outgoing edges) should have closeness 0.0"
        );
        // C is completely isolated -> closeness = 0.0
        assert_eq!(scores["C"], 0.0, "Isolated C should have closeness 0.0");
    }
}
